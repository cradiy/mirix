//! Declarative engine menus. The host treats menu and entry IDs as opaque values.
use crate::{KeyEvent, Modifiers};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Shortcut {
    pub mods: Modifiers,
    pub letter: char,
}

impl std::str::FromStr for Shortcut {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut mods = Modifiers::default();
        let mut letter = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" => mods.alt = true,
                "super" => mods.logo = true,
                "space" => {
                    if letter.replace(' ').is_some() {
                        return Err("shortcut must contain exactly one key".into());
                    }
                }
                key if key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
                    if letter.replace(key.as_bytes()[0] as char).is_some() {
                        return Err("shortcut must contain exactly one letter".into());
                    }
                }
                _ => return Err(format!("invalid shortcut component {part:?}")),
            }
        }
        if !mods.has_ctrl_alt_or_logo() {
            return Err("shortcut needs Ctrl, Alt or Super".into());
        }
        Ok(Self {
            mods,
            letter: letter.ok_or("shortcut needs a letter")?,
        })
    }
}

impl Shortcut {
    pub fn matches(self, event: &KeyEvent) -> bool {
        event.mods == self.mods
            && (char::from_u32(event.keysym)
                .is_some_and(|letter| letter.to_ascii_lowercase() == self.letter)
                || (self.letter == ' ' && event.keycode == crate::evdev::KEY_SPACE))
    }
}

impl std::fmt::Display for Shortcut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (enabled, name) in [
            (self.mods.ctrl, "Ctrl"),
            (self.mods.alt, "Alt"),
            (self.mods.shift, "Shift"),
            (self.mods.logo, "Super"),
        ] {
            if enabled {
                write!(f, "{name}+")?;
            }
        }
        if self.letter == ' ' {
            f.write_str("Space")
        } else {
            write!(f, "{}", self.letter.to_ascii_uppercase())
        }
    }
}

impl TryFrom<String> for Shortcut {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<Shortcut> for String {
    fn from(value: Shortcut) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuEntry {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineMenu {
    pub id: String,
    pub label: String,
    pub shortcut: Shortcut,
    pub entries: Vec<MenuEntry>,
    pub active: Option<String>,
}

pub fn validate_menus(menus: &[EngineMenu]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    let mut shortcuts = std::collections::HashSet::new();
    for menu in menus {
        if menu.id.is_empty()
            || menu.label.is_empty()
            || !ids.insert(&menu.id)
            || !shortcuts.insert(menu.shortcut.to_string())
        {
            return Err("menus need unique nonempty IDs, labels, and shortcuts".into());
        }
        let mut entries = std::collections::HashSet::new();
        for entry in &menu.entries {
            if entry.id.is_empty() || entry.label.is_empty() || !entries.insert(&entry.id) {
                return Err(format!("invalid entries in menu {}", menu.id));
            }
        }
        if menu.active.as_ref().is_some_and(|id| !entries.contains(id)) {
            return Err(format!("unknown active entry in menu {}", menu.id));
        }
    }
    Ok(())
}

pub fn select_menu(
    engine: &mut dyn crate::Engine,
    menu: &str,
    entry: &str,
) -> Result<crate::EngineOutput, String> {
    let menus = engine.menus();
    validate_menus(&menus)?;
    if !menus
        .iter()
        .any(|item| item.id == menu && item.entries.iter().any(|item| item.id == entry))
    {
        return Err("menu or entry is no longer available".into());
    }
    engine.select_menu(menu, entry)
}
