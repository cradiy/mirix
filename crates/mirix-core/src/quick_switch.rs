//! Modal navigation for panel pickers, independent of the GUI and input engine.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{EngineMenu, KeyEvent, MenuEntry, Shortcut, keysym};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuickSwitchKind {
    Engine { id: String },
    Skin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickSwitchSnapshot {
    pub session: u64,
    pub kind: QuickSwitchKind,
    pub label: String,
    pub entries: Vec<MenuEntry>,
    pub selected: usize,
    pub active: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QuickSwitchShortcuts {
    pub skin: Shortcut,
}

impl Default for QuickSwitchShortcuts {
    fn default() -> Self {
        Self {
            skin: "Ctrl+Shift+S".parse().unwrap(),
        }
    }
}

pub(crate) enum QuickSwitchAction {
    Pass,
    Consumed,
    Select(QuickSwitchKind, String),
}

#[derive(Default)]
pub(crate) struct QuickSwitch {
    pub shortcuts: QuickSwitchShortcuts,
    pub available: bool,
    pub snapshot: Option<QuickSwitchSnapshot>,
    pub skins: Vec<MenuEntry>,
    pub active_skin: Option<String>,
    next_session: u64,
    // Navigation may repeat. Confirm/open/cancel must not repeat into the engine.
    held: HashMap<u32, Option<isize>>,
}

impl QuickSwitch {
    pub fn close(&mut self) {
        self.snapshot = None;
        self.held.values_mut().for_each(|step| *step = None);
    }

    pub fn reset_focus(&mut self) {
        self.close();
        self.held.clear();
    }

    pub fn select(&self, session: u64, index: usize) -> Option<(QuickSwitchKind, String)> {
        let snapshot = self.snapshot.as_ref()?;
        (snapshot.session == session)
            .then(|| snapshot.entries.get(index))
            .flatten()
            .map(|entry| (snapshot.kind.clone(), entry.id.clone()))
    }

    pub fn handle(
        &mut self,
        event: &KeyEvent,
        visible: bool,
        menus: impl FnOnce() -> Vec<EngineMenu>,
    ) -> QuickSwitchAction {
        // Synthetic keys use a keysym identity; physical keys keep their identity
        // when a modifier is released before the key itself.
        let key = if event.keycode == 0 {
            event.keysym | 0x8000_0000
        } else {
            event.keycode
        };
        if !event.is_press() {
            return if self.held.remove(&key).is_some() {
                QuickSwitchAction::Consumed
            } else {
                QuickSwitchAction::Pass
            };
        }
        if let Some(step) = self.held.get(&key).copied() {
            if let Some(step) = step {
                self.navigate(step);
            }
            return QuickSwitchAction::Consumed;
        }
        let opening = if self.shortcuts.skin.matches(event) {
            Some((
                QuickSwitchKind::Skin,
                String::new(),
                self.skins.clone(),
                self.active_skin.clone(),
            ))
        } else if self.available && visible && event.mods.has_ctrl_alt_or_logo() {
            menus()
                .into_iter()
                .find(|menu| menu.shortcut.matches(event) && !menu.entries.is_empty())
                .map(|menu| {
                    (
                        QuickSwitchKind::Engine { id: menu.id },
                        menu.label,
                        menu.entries,
                        menu.active,
                    )
                })
        } else {
            None
        };
        if self.available
            && visible
            && let Some((kind, label, entries, active)) = opening
        {
            self.held.insert(key, None);
            if self
                .snapshot
                .as_ref()
                .is_some_and(|state| state.kind == kind)
            {
                self.close();
            } else {
                self.next_session += 1;
                let selected = entries
                    .iter()
                    .position(|entry| Some(&entry.id) == active.as_ref())
                    .unwrap_or(0);
                self.snapshot = Some(QuickSwitchSnapshot {
                    session: self.next_session,
                    kind,
                    label,
                    entries,
                    selected,
                    active,
                    error: None,
                });
            }
            return QuickSwitchAction::Consumed;
        }
        if self.snapshot.is_none() {
            return QuickSwitchAction::Pass;
        }
        // Let modifier presses/releases reach the normal pipeline. The following
        // non-modifier shortcut decides whether to dismiss the picker.
        if matches!(event.keysym, 0xffe1..=0xffee) {
            return QuickSwitchAction::Pass;
        }
        if event.mods.has_ctrl_alt_or_logo() {
            self.close();
            return QuickSwitchAction::Pass;
        }
        let step = match event.keysym {
            keysym::DOWN | 0x6a | 0x6e => Some(1),
            keysym::UP | 0x6b | 0x70 => Some(-1),
            _ => None,
        };
        self.held.insert(key, step);
        if let Some(step) = step {
            self.navigate(step);
        } else if event.keysym == keysym::ESCAPE {
            self.close();
        } else if event.keysym == keysym::RETURN {
            let state = self.snapshot.as_ref().unwrap();
            if let Some((kind, id)) = self.select(state.session, state.selected) {
                return QuickSwitchAction::Select(kind, id);
            }
        }
        QuickSwitchAction::Consumed
    }

    fn navigate(&mut self, step: isize) {
        if let Some(state) = &mut self.snapshot
            && !state.entries.is_empty()
        {
            state.selected =
                (state.selected as isize + step).rem_euclid(state.entries.len() as isize) as usize;
            state.error = None;
        }
    }
}

#[cfg(test)]
mod tests;
