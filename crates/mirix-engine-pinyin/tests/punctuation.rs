use mirix_core::{
    Engine, EngineManager, Instance, KeyEvent, KeyState, Modifiers, PunctuationMode, keysym,
};
use mirix_engine_keyboard::KeyboardEngine;
use mirix_engine_pinyin::PinyinEngine;

fn key(character: char) -> KeyEvent {
    KeyEvent {
        keycode: 0,
        keysym: character as u32,
        state: KeyState::Press,
        mods: Modifiers::default(),
        text: Some(character.to_string()),
    }
}

fn shortcut() -> KeyEvent {
    KeyEvent {
        keycode: 52,
        mods: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
        ..key('.')
    }
}

fn toggle(engine: &mut dyn Engine) {
    let event = shortcut();
    assert!(engine.process_key(&event).consumed);
    assert!(
        engine
            .process_key(&KeyEvent {
                state: KeyState::Release,
                ..event
            })
            .consumed
    );
}

fn open(root: &std::path::Path) -> PinyinEngine {
    PinyinEngine::open(root.join("data"), root.join("config")).unwrap()
}

#[test]
fn shortcut_preserves_composition_selection_and_does_not_repeat() {
    let root = tempfile::tempdir().unwrap();
    let mut instance = Instance::new(Box::new(open(root.path())));
    let id = instance.create_context("test");
    instance.focus_in(id);
    for character in "ni".chars() {
        instance.process_key(id, &key(character));
    }
    let before = instance.panel_snapshot();
    let control = KeyEvent {
        keysym: 0xffe3,
        text: None,
        ..shortcut()
    };
    assert!(!instance.process_key(id, &control).consumed);
    let output = instance.process_key(id, &shortcut());
    assert!(output.consumed);
    assert!(output.commit.is_none());
    assert!(output.preedit_text().is_none());
    let mut expected = before;
    expected.punctuation_mode = Some(PunctuationMode::Half);
    assert_eq!(instance.panel_snapshot(), expected);
    instance.process_key(id, &shortcut());
    assert_eq!(instance.panel_snapshot(), expected);
    // Releasing Ctrl/pressing Shift first can change the release keysym.
    instance.process_key(
        id,
        &KeyEvent {
            state: KeyState::Release,
            keysym: '>' as u32,
            text: Some(">".into()),
            ..shortcut()
        },
    );
    assert_eq!(
        instance.process_key(id, &key(',')).commit.unwrap().text,
        "你,"
    );
    assert!(!instance.panel_snapshot().visible);
    instance.process_key(id, &shortcut());
    assert_eq!(
        instance.panel_snapshot().punctuation_mode,
        Some(PunctuationMode::Full)
    );
}

#[test]
fn punctuation_setting_survives_restart_reset_and_profile_changes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("config/profiles/日常")).unwrap();
    std::fs::create_dir_all(root.path().join("config/profiles/编程")).unwrap();
    let mut engine = open(root.path());
    assert_eq!(engine.punctuation_mode(), Some(PunctuationMode::Full));
    toggle(&mut engine);
    engine.reset();
    engine.activate_profile("编程").unwrap();
    assert_eq!(engine.punctuation_mode(), Some(PunctuationMode::Half));
    assert_eq!(engine.process_key(&key('.')).commit.unwrap().text, ".");
    drop(engine);
    assert_eq!(
        std::fs::read_to_string(root.path().join("data/punctuation-mode.json")).unwrap(),
        "\"half\""
    );
    let mut engine = open(root.path());
    assert_eq!(engine.punctuation_mode(), Some(PunctuationMode::Half));
    toggle(&mut engine);
    assert_eq!(engine.process_key(&key('.')).commit.unwrap().text, "。");
    drop(engine);
    assert_eq!(
        open(root.path()).punctuation_mode(),
        Some(PunctuationMode::Full)
    );
}

#[test]
fn keyboard_mode_does_not_intercept_the_shortcut_or_show_an_indicator() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = EngineManager::new(vec![
        Box::new(KeyboardEngine::new()),
        Box::new(open(root.path())),
    ])
    .unwrap();
    assert_eq!(engine.punctuation_mode(), None);
    assert!(!engine.process_key(&shortcut()).consumed);
    engine.activate_input_method("pinyin");
    assert!(engine.process_key(&shortcut()).consumed);
    engine.process_key(&KeyEvent {
        state: KeyState::Release,
        ..shortcut()
    });
    assert_eq!(engine.punctuation_mode(), Some(PunctuationMode::Half));
    engine.activate_input_method("keyboard");
    assert_eq!(engine.punctuation_mode(), None);
    assert!(!engine.process_key(&shortcut()).consumed);
    engine.activate_input_method("pinyin");
    assert_eq!(engine.punctuation_mode(), Some(PunctuationMode::Half));
}

#[test]
fn punctuation_translation_does_not_change_letters_digits_or_composition_controls() {
    let root = tempfile::tempdir().unwrap();
    let mut engine = open(root.path());
    assert_eq!(engine.process_key(&key('"')).commit.unwrap().text, "“");
    toggle(&mut engine);
    for character in ".,!?;:()[]<>/\\^_~$'\"{}#&%@*+-=|`".chars() {
        assert_eq!(
            engine.process_key(&key(character)).commit.unwrap().text,
            character.to_string()
        );
    }
    assert!(!engine.process_key(&key('1')).consumed);
    assert_eq!(engine.process_key(&key('.')).commit.unwrap().text, ".");
    engine.process_key(&key('n'));
    engine.process_key(&key('i'));
    let apostrophe = engine.process_key(&key('\''));
    assert_eq!(apostrophe.preedit_text(), Some("ni'"));
    assert!(apostrophe.commit.is_none());
    assert!(engine.process_key(&key('=')).commit.is_none());
    engine.process_key(&KeyEvent {
        keysym: keysym::ESCAPE,
        text: None,
        ..key(' ')
    });
    toggle(&mut engine);
    assert_eq!(engine.process_key(&key('"')).commit.unwrap().text, "“");
    assert!(!engine.process_key(&key('3')).consumed);
    assert!(!engine.process_key(&key('.')).consumed);
    for mods in [
        Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        },
        Modifiers {
            ctrl: true,
            alt: true,
            ..Modifiers::default()
        },
        Modifiers {
            ctrl: true,
            logo: true,
            ..Modifiers::default()
        },
    ] {
        assert!(
            !engine
                .process_key(&KeyEvent { mods, ..shortcut() })
                .consumed
        );
        assert_eq!(engine.punctuation_mode(), Some(PunctuationMode::Full));
    }
}
