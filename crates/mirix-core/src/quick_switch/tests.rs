use super::*;
use crate::{
    Candidate, CandidateId, ContextId, Engine, EngineOutput, Instance, KeyState, Modifiers,
};

fn key(symbol: u32) -> KeyEvent {
    KeyEvent {
        keycode: symbol,
        keysym: symbol,
        state: KeyState::Press,
        mods: Modifiers::default(),
        text: char::from_u32(symbol).map(|c| c.to_string()),
    }
}

fn shortcut(letter: char) -> KeyEvent {
    KeyEvent {
        mods: Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        },
        ..key(letter as u32)
    }
}

fn tap(instance: &mut Instance, id: ContextId, event: KeyEvent) -> EngineOutput {
    let result = instance.process_key(id, &event);
    instance.process_key(
        id,
        &KeyEvent {
            state: KeyState::Release,
            ..event
        },
    );
    result
}

struct Method {
    mode: String,
    candidates: Vec<Candidate>,
}

impl Engine for Method {
    fn name(&self) -> &str {
        "fixture-ime"
    }
    fn menus(&self) -> Vec<EngineMenu> {
        vec![
            EngineMenu {
                id: "modes".into(),
                label: "Modes".into(),
                shortcut: "Ctrl+Shift+P".parse().unwrap(),
                active: Some(self.mode.clone()),
                entries: ["daily", "code", "broken"]
                    .into_iter()
                    .map(|name| MenuEntry {
                        id: name.into(),
                        label: name.into(),
                    })
                    .collect(),
            },
            EngineMenu {
                id: "rewrite".into(),
                label: "Rewrite input".into(),
                shortcut: "Ctrl+Shift+R".parse().unwrap(),
                entries: vec![MenuEntry {
                    id: "replacement".into(),
                    label: "Replace composition".into(),
                }],
                active: None,
            },
        ]
    }
    fn select_menu(&mut self, menu: &str, name: &str) -> Result<EngineOutput, String> {
        if menu == "rewrite" {
            self.candidates = vec![
                Candidate {
                    keep_open: false,
                    id: CandidateId(3),
                    text: "新".into(),
                    comment: None,
                    reading: None,
                },
                Candidate {
                    keep_open: false,
                    id: CandidateId(4),
                    text: "心".into(),
                    comment: None,
                    reading: None,
                },
            ];
            return Ok(EngineOutput {
                preedit: crate::PreeditUpdate::Set(crate::Composition {
                    text: "xin".into(),
                    cursor: 2,
                    selection: None,
                }),
                candidates_changed: true,
                ..EngineOutput::consume()
            });
        }
        if name == "broken" {
            return Err("cannot apply mode".into());
        }
        self.mode = name.into();
        Ok(EngineOutput::consume())
    }
    fn process_key(&mut self, event: &KeyEvent) -> EngineOutput {
        if !event.is_press() || event.mods.has_ctrl_alt_or_logo() {
            return EngineOutput::pass();
        }
        if matches!(event.keysym, keysym::RETURN | keysym::SPACE) {
            return self.select(CandidateId(1));
        }
        self.candidates = vec![
            Candidate {
                keep_open: false,
                id: CandidateId(1),
                text: "世界".into(),
                comment: None,
                reading: None,
            },
            Candidate {
                keep_open: false,
                id: CandidateId(2),
                text: "时节".into(),
                comment: None,
                reading: None,
            },
        ];
        EngineOutput::preedit("shi'jie")
    }
    fn reset(&mut self) {
        self.candidates.clear();
    }
    fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }
    fn selected_candidate(&self) -> usize {
        usize::from(!self.candidates.is_empty())
    }
    fn select(&mut self, _: CandidateId) -> EngineOutput {
        self.candidates.clear();
        EngineOutput::commit("世界")
    }
}

fn instance() -> (Instance, ContextId) {
    let mut instance = Instance::new(Box::new(Method {
        mode: "daily".into(),
        candidates: vec![],
    }));
    let id = instance.create_context("test");
    instance.focus_in(id);
    instance.set_quick_switch_available(true);
    instance.set_panel_skins(
        ["default", "night"]
            .into_iter()
            .map(|name| MenuEntry {
                id: name.into(),
                label: name.into(),
            })
            .collect(),
        Some("default".into()),
    );
    tap(&mut instance, id, key('s' as u32));
    (instance, id)
}

#[test]
fn modal_navigation_preserves_composition_and_enter_does_not_commit_even_when_repeated() {
    let (mut instance, id) = instance();
    let before = instance.panel_snapshot();
    assert!(tap(&mut instance, id, shortcut('P')).consumed);
    for symbol in ['a' as u32, '1' as u32, keysym::SPACE, keysym::BACKSPACE] {
        assert!(tap(&mut instance, id, key(symbol)).consumed);
    }
    assert!(instance.select(id, CandidateId(1)).is_none());
    tap(&mut instance, id, key('j' as u32));
    assert_eq!(instance.menus()[0].active.as_deref(), Some("daily"));
    assert_eq!(instance.panel_snapshot(), before);
    for _ in 0..3 {
        let out = instance.process_key(id, &key(keysym::RETURN));
        assert!(out.consumed);
        assert_eq!(out.commit, None);
    }
    assert_eq!(instance.menus()[0].active.as_deref(), Some("code"));
    assert_eq!(instance.panel_snapshot(), before);
    assert!(instance.quick_switch_snapshot().is_none());
    assert!(
        instance
            .process_key(
                id,
                &KeyEvent {
                    state: KeyState::Release,
                    ..key(keysym::RETURN)
                }
            )
            .consumed
    );
    assert!(tap(&mut instance, id, key(keysym::RETURN)).commit.is_some());
}

#[test]
fn navigation_aliases_wrap_and_held_keys_repeat_only_navigation() {
    let (mut instance, id) = instance();
    instance.process_key(id, &shortcut('P'));
    instance.process_key(id, &shortcut('P'));
    assert!(instance.quick_switch_snapshot().is_some());
    for (symbol, expected) in [
        ('k' as u32, 2),
        ('n' as u32, 0),
        (keysym::UP, 2),
        ('p' as u32, 1),
        (keysym::DOWN, 2),
    ] {
        tap(&mut instance, id, key(symbol));
        assert_eq!(instance.quick_switch_snapshot().unwrap().selected, expected);
    }
    instance.process_key(id, &key('j' as u32));
    instance.process_key(id, &key('j' as u32));
    assert_eq!(instance.quick_switch_snapshot().unwrap().selected, 1);
}

#[test]
fn switching_kind_and_stale_clicks_do_not_apply_to_reopened_menu() {
    let (mut instance, id) = instance();
    tap(&mut instance, id, shortcut('P'));
    let old = instance.quick_switch_snapshot().unwrap().session;
    tap(&mut instance, id, shortcut('S'));
    assert!(!instance.choose_quick_switch(old, 1));
    let session = instance.quick_switch_snapshot().unwrap().session;
    assert!(instance.choose_quick_switch(session, 1));
    assert_eq!(instance.take_skin_selection().as_deref(), Some("night"));
    assert_eq!(instance.take_skin_selection(), None);
    assert!(!instance.choose_quick_switch(session, 1));
    assert_eq!(instance.menus()[0].active.as_deref(), Some("daily"));
}

#[test]
fn failure_keeps_menu_with_error_and_escape_does_not_cancel_composition() {
    let (mut instance, id) = instance();
    let before = instance.panel_snapshot();
    tap(&mut instance, id, shortcut('P'));
    tap(&mut instance, id, key('k' as u32));
    tap(&mut instance, id, key(keysym::RETURN));
    assert!(instance.quick_switch_snapshot().unwrap().error.is_some());
    tap(&mut instance, id, key(keysym::ESCAPE));
    assert!(instance.quick_switch_snapshot().is_none());
    assert_eq!(instance.panel_snapshot(), before);
}

#[test]
fn unavailable_panel_and_invisible_candidates_do_not_capture_shortcuts() {
    let (mut instance, id) = instance();
    instance.set_quick_switch_available(false);
    assert!(!tap(&mut instance, id, shortcut('P')).consumed);
    instance.set_quick_switch_available(true);
    instance.reset(id);
    assert!(!tap(&mut instance, id, shortcut('P')).consumed);
}

#[test]
fn focus_loss_reset_disconnect_and_other_shortcuts_dismiss_menu() {
    for action in 0..4 {
        let (mut instance, id) = instance();
        tap(&mut instance, id, shortcut('P'));
        match action {
            0 => instance.focus_out(id),
            1 => instance.reset(id),
            2 => instance.set_quick_switch_available(false),
            _ => {
                assert!(!tap(&mut instance, id, shortcut('C')).consumed);
            }
        }
        assert!(instance.quick_switch_snapshot().is_none());
    }
}

#[test]
fn configured_shortcuts_override_defaults() {
    let (mut instance, id) = instance();
    instance.set_quick_switch_shortcuts(QuickSwitchShortcuts {
        skin: "Alt+S".parse().unwrap(),
    });
    assert!(!tap(&mut instance, id, shortcut('S')).consumed);
    let mut event = key('s' as u32);
    event.mods.alt = true;
    assert!(tap(&mut instance, id, event).consumed);
    assert!(instance.quick_switch_snapshot().is_some());
    assert!("Shift+P".parse::<Shortcut>().is_err());
    assert!("Ctrl+P+S".parse::<Shortcut>().is_err());
}

#[test]
fn unchanged_or_unavailable_method_does_not_silently_close_picker() {
    let (mut instance, id) = instance();
    tap(&mut instance, id, shortcut('P'));
    let menu = instance.quick_switch_snapshot().cloned();
    assert!(instance.activate_input_method("fixture-ime"));
    assert_eq!(instance.quick_switch_snapshot(), menu.as_ref());
    assert!(!instance.activate_input_method("missing"));
    assert_eq!(instance.quick_switch_snapshot(), menu.as_ref());
    assert_eq!(instance.toggle_input_method(), None);
    assert_eq!(instance.quick_switch_snapshot(), menu.as_ref());
}

#[test]
fn engine_menus_route_independently_and_refresh_only_on_explicit_effect() {
    let (mut instance, id) = instance();
    let before = instance.panel_snapshot();
    tap(&mut instance, id, shortcut('P'));
    let first = instance.quick_switch_snapshot().unwrap().session;
    tap(&mut instance, id, shortcut('R'));
    let popup = instance.quick_switch_snapshot().unwrap();
    assert_eq!(
        popup.kind,
        QuickSwitchKind::Engine {
            id: "rewrite".into()
        }
    );
    assert_eq!(popup.label, "Rewrite input");
    assert!(!instance.choose_quick_switch(first, 0));
    assert_eq!(instance.panel_snapshot(), before);
    let output = tap(&mut instance, id, key(keysym::RETURN));
    assert!(output.commit.is_none());
    assert_eq!(output.preedit_text(), Some("xin"));
    assert_eq!(output.preedit_cursor(), 2);
    let after = instance.panel_snapshot();
    assert_eq!(after.preedit, "xin");
    assert_eq!(after.candidates[0].text, "新");
    assert_eq!(after.selected, 1);
    assert!(instance.quick_switch_snapshot().is_none());
}

#[test]
fn framework_skin_binding_has_priority_over_engine_binding() {
    let (mut instance, id) = instance();
    instance.set_quick_switch_shortcuts(QuickSwitchShortcuts {
        skin: "Ctrl+Shift+P".parse().unwrap(),
    });
    tap(&mut instance, id, shortcut('P'));
    assert_eq!(
        instance.quick_switch_snapshot().unwrap().kind,
        QuickSwitchKind::Skin
    );
}
