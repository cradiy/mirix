//! Daemon preferences. Explicit CLI shortcuts override saved values.

use std::{io::ErrorKind, path::Path};

use anyhow::{Context, Result, bail};
use mirix_core::{QuickSwitchShortcuts, Shortcut};

pub(super) fn save_enabled(path: &Path, names: &[String]) -> Result<()> {
    use mirix_config::{Item, Table, value};
    mirix_config::update(path, |document| {
        if !document.contains_key("input_methods") {
            document["input_methods"] = Item::Table(Table::new());
        }
        let table = document["input_methods"]
            .as_table_like_mut()
            .context("input_methods must be a table")?;
        table.insert(
            "enabled",
            value(names.iter().cloned().collect::<mirix_config::Array>()),
        );
        Ok(())
    })
}

pub(super) fn save_shortcut(path: &Path, name: &str, shortcut: Shortcut) -> Result<()> {
    use mirix_config::{Item, Table, value};
    mirix_config::update(path, |document| {
        if !document.contains_key("shortcuts") {
            document["shortcuts"] = Item::Table(Table::new());
        }
        document["shortcuts"]
            .as_table_like_mut()
            .context("shortcuts must be a table")?
            .insert(name, value(shortcut.to_string()));
        Ok(())
    })
}

pub(super) fn enabled_methods(path: &Path, explicit: Vec<String>) -> Result<Vec<String>> {
    if !explicit.is_empty() {
        return validate_enabled(explicit);
    }
    let document = read_document(path)?;
    let Some(value) = document
        .get("input_methods")
        .map(|value| value.as_table().context("input_methods must be a table"))
        .transpose()?
        .and_then(|table| table.get("enabled"))
    else {
        return Ok(vec!["keyboard".into(), "pinyin".into()]);
    };
    let names = value
        .as_array()
        .context("input_methods.enabled must be an array")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .context("enabled input method must be a string")
        })
        .collect::<Result<Vec<_>>>()?;
    validate_enabled(names)
}

fn validate_enabled(names: Vec<String>) -> Result<Vec<String>> {
    let mut seen = std::collections::HashSet::new();
    if names
        .iter()
        .any(|name| name.is_empty() || !seen.insert(name))
    {
        bail!("enabled input methods must be nonempty, unique names");
    }
    Ok(names)
}

pub(super) fn switch_shortcut(path: &Path, explicit: Option<Shortcut>) -> Result<Shortcut> {
    if let Some(shortcut) = explicit {
        return Ok(shortcut);
    }
    let document = read_document(path)?;
    let value = document
        .get("shortcuts")
        .and_then(|table| table.get("switch"));
    value
        .map(|value| value.as_str().context("shortcuts.switch must be a string"))
        .transpose()?
        .unwrap_or("Ctrl+Space")
        .parse()
        .map_err(anyhow::Error::msg)
}

fn read_document(path: &Path) -> Result<toml::Table> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse()
            .with_context(|| format!("parse {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

pub(super) fn quick_switch_shortcuts(
    path: &Path,
    skin: Option<Shortcut>,
) -> Result<QuickSwitchShortcuts> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let document: toml::Table = text
        .parse()
        .with_context(|| format!("parse {}", path.display()))?;
    let table = document
        .get("shortcuts")
        .map(|value| value.as_table().context("shortcuts must be a table"))
        .transpose()?;
    let read = |key: &str, explicit: Option<Shortcut>, default: Shortcut| -> Result<Shortcut> {
        if let Some(explicit) = explicit {
            return Ok(explicit);
        }
        let Some(value) = table.and_then(|table| table.get(key)) else {
            return Ok(default);
        };
        let value = value
            .as_str()
            .with_context(|| format!("shortcuts.{key} must be a string"))?;
        value
            .parse()
            .map_err(|error: String| anyhow::anyhow!("shortcuts.{key}: {error}"))
    };
    let defaults = QuickSwitchShortcuts::default();
    let shortcuts = QuickSwitchShortcuts {
        skin: read("skin", skin, defaults.skin)?,
    };

    Ok(shortcuts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_order_empty_selection_and_cli_override() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        std::fs::write(&path, "[input_methods]\nenabled = ['third', 'first', 'second']\n[shortcuts]\nswitch = 'Alt+Space'\n").unwrap();
        assert_eq!(
            enabled_methods(&path, vec![]).unwrap(),
            ["third", "first", "second"]
        );
        assert_eq!(
            enabled_methods(&path, vec!["explicit".into()]).unwrap(),
            ["explicit"]
        );
        assert_eq!(
            switch_shortcut(&path, None).unwrap().to_string(),
            "Alt+Space"
        );
        assert_eq!(
            switch_shortcut(&path, Some("Ctrl+J".parse().unwrap()))
                .unwrap()
                .to_string(),
            "Ctrl+J"
        );
        std::fs::write(&path, "[input_methods]\nenabled = []\n").unwrap();
        assert!(enabled_methods(&path, vec![]).unwrap().is_empty());
        std::fs::write(&path, "[input_methods]\nenabled = ['one', 'one']\n").unwrap();
        assert!(enabled_methods(&path, vec![]).is_err());
    }

    #[test]
    fn saved_shortcuts_and_cli_overrides_share_existing_config_without_writing_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        assert_eq!(
            quick_switch_shortcuts(&path, None).unwrap().skin,
            QuickSwitchShortcuts::default().skin
        );
        let source = "[panel]\nskin = '/skins/night'\n[shortcuts]\nskin = 'Alt+S'\n";
        std::fs::write(&path, source).unwrap();
        assert_eq!(
            quick_switch_shortcuts(&path, None).unwrap().skin,
            "Alt+S".parse().unwrap()
        );
        assert_eq!(
            quick_switch_shortcuts(&path, Some("Ctrl+Alt+S".parse().unwrap()))
                .unwrap()
                .skin,
            "Ctrl+Alt+S".parse().unwrap()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    }

    #[test]
    fn invalid_shortcuts_report_the_setting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        std::fs::write(&path, "[shortcuts]\nskin = 'P'\n").unwrap();
        assert!(
            quick_switch_shortcuts(&path, None)
                .unwrap_err()
                .to_string()
                .contains("shortcuts.skin")
        );
    }
}
