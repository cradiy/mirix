use mirix_core::{EngineManager, Instance};
use mirix_engine_keyboard::KeyboardEngine;
use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::*;

fn change(id: &str, value: &str) -> ConfigurationEvent {
    ConfigurationEvent::Change {
        id: id.into(),
        value: ConfigurationValue::String(value.into()),
    }
}
fn key(ch: char) -> KeyEvent {
    KeyEvent {
        keycode: 0,
        keysym: ch as u32,
        state: KeyState::Press,
        mods: Modifiers::default(),
        text: Some(ch.to_string()),
    }
}

#[test]
fn engine_owned_menu_shortcut_is_configurable_persistent_and_does_not_edit_input() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    assert_eq!(engine.menus()[0].shortcut.to_string(), "Ctrl+Shift+P");
    for ch in "ni".chars() {
        engine.process_key(&key(ch));
    }
    let candidates = engine.candidates().to_vec();
    let selected = engine.selected_candidate();
    let page = engine.configuration("zh-CN").unwrap();
    let event = change("profile-shortcut", "Alt+P");
    page.validate_event(&event).unwrap();
    engine
        .configure("zh-CN", engine.configuration_revision(), event)
        .unwrap();
    assert_eq!(engine.menus()[0].shortcut.to_string(), "Alt+P");
    assert_eq!(engine.menus()[0].label, "场景");
    assert_eq!(engine.candidates(), candidates);
    assert_eq!(engine.selected_candidate(), selected);
    assert_eq!(
        engine.select_menu("profiles", "日常").unwrap(),
        EngineOutput::consume()
    );
    drop(engine);
    let engine = PinyinEngine::open(&data, &config).unwrap();
    assert_eq!(engine.menus()[0].shortcut.to_string(), "Alt+P");
}

#[test]
fn settings_preserve_candidates_and_are_persisted_by_the_engine() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    for name in ["日常", "编程"] {
        std::fs::create_dir_all(config.join("profiles").join(name)).unwrap();
    }
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    for ch in "ni".chars() {
        engine.process_key(&key(ch));
    }
    engine.process_key(&KeyEvent {
        keysym: keysym::RIGHT,
        text: None,
        ..key(' ')
    });
    let candidates = engine.candidates().to_vec();
    let selected = engine.selected_candidate();
    let page = engine.configuration("zh-CN").unwrap();
    assert_eq!(page.title, "中文拼音");
    let revision = engine.configuration_revision();
    engine
        .configure("zh-CN", revision, change("punctuation", "half"))
        .unwrap();
    assert_eq!(engine.candidates(), candidates);
    assert_eq!(engine.selected_candidate(), selected);
    assert!(
        engine
            .configure("zh-CN", revision, change("profile", "编程"))
            .is_err()
    );
    assert_eq!(engine.profiles().active.as_deref(), Some("日常"));
    let revision = engine.configuration_revision();
    engine
        .configure("en-US", revision, change("punctuation", "half"))
        .unwrap();
    assert_eq!(engine.configuration_revision(), revision); // No-op does not invalidate menus.
    engine
        .configure("en-US", revision, change("profile", "编程"))
        .unwrap();
    drop(engine);
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    assert_eq!(engine.profiles().active.as_deref(), Some("编程"));
    assert_eq!(engine.punctuation_mode(), Some(PunctuationMode::Half));
    let revision = engine.configuration_revision();
    let mut shortcut = key('.');
    shortcut.mods.ctrl = true;
    engine.process_key(&shortcut);
    assert!(engine.configuration_revision() > revision);
    let page = engine.configuration("en-US").unwrap();
    assert_eq!(page.title, "Chinese Pinyin");
    assert!(
        matches!(&page.groups[0].controls[1].kind, ConfigurationKind::Select { value, .. } if value == "full")
    );
}

#[test]
fn inactive_configuration_and_session_rejection_preserve_active_composition() {
    let root = tempfile::tempdir().unwrap();
    let mut instance = Instance::with_manager(
        EngineManager::new(vec![
            Box::new(KeyboardEngine::new()),
            Box::new(
                PinyinEngine::open(root.path().join("data"), root.path().join("config")).unwrap(),
            ),
        ])
        .unwrap(),
    );
    let id = instance.create_context("test");
    instance.focus_in(id);
    let mut unicode = key('U');
    unicode.mods.ctrl = true;
    unicode.mods.shift = true;
    instance.process_key(id, &unicode);
    let before = instance.panel_snapshot();
    let configuration = instance.configuration("pinyin", "en-US").unwrap();
    instance
        .configure(
            &configuration.status,
            "en-US",
            change("punctuation", "half"),
        )
        .unwrap();
    assert_eq!(instance.engine_name(), "keyboard");
    assert_eq!(instance.panel_snapshot(), before);
    let mut status = instance.configuration("pinyin", "en-US").unwrap().status;
    status.session.push_str("-different-session");
    assert!(
        instance
            .configure(&status, "en-US", change("punctuation", "full"))
            .is_err()
    );
    assert_eq!(instance.panel_snapshot(), before);
}

#[test]
fn create_profile_is_immediately_available_isolated_and_persistent() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    std::fs::create_dir_all(config.join("dictionaries")).unwrap();
    std::fs::write(
        config.join("dictionaries/shared.tsv"),
        "nihao\tnǐ hǎo\t你好\t100\n",
    )
    .unwrap();
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    for ch in "ni".chars() {
        engine.process_key(&key(ch));
    }
    let candidate = engine.candidates()[0].id;
    engine.select(candidate);
    let revision = engine.configuration_revision();
    engine
        .configure("en-US", revision, change("create-profile", "  编程 Work  "))
        .unwrap();
    assert_eq!(engine.profiles().active.as_deref(), Some("编程 Work"));
    assert!(engine.configuration_revision() > revision);
    let page = engine.configuration("en-US").unwrap();
    assert!(matches!(&page.groups[0].controls[0].kind,
        ConfigurationKind::Select { value, choices }
        if value == "编程 Work" && choices.iter().any(|choice| choice.value == *value)));
    assert!(config.join("profiles/编程 Work/dictionaries").is_dir());
    assert!(config.join("profiles/编程 Work/extensions").is_dir());
    assert!(data.join("profiles/编程 Work").is_dir());
    assert!(!data.join("profiles/编程 Work/learning.json").exists());
    for ch in "nihao".chars() {
        engine.process_key(&key(ch));
    }
    assert!(
        engine
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "你好")
    );
    drop(engine);
    assert!(data.join("profiles/日常/learning.json").is_file());
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    assert_eq!(engine.profiles().active.as_deref(), Some("编程 Work"));
    // The next profile also starts with independent storage.
    engine
        .configure(
            "en-US",
            engine.configuration_revision(),
            change("create-profile", "Writing"),
        )
        .unwrap();
    assert_eq!(engine.profiles().active.as_deref(), Some("Writing"));
}

#[test]
fn invalid_or_existing_profile_names_preserve_state_and_disk_contents() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    std::fs::create_dir_all(config.join("profiles/External")).unwrap();
    std::fs::write(config.join("profiles/External/keep"), "untouched").unwrap();
    std::fs::create_dir_all(data.join("profiles/Stored")).unwrap();
    std::fs::write(data.join("profiles/Stored/keep"), "untouched").unwrap();
    for ch in "ni".chars() {
        engine.process_key(&key(ch));
    }
    let before = engine.candidates().to_vec();
    let profiles = engine.profiles();
    let revision = engine.configuration_revision();
    for name in [
        "",
        "  ",
        ".",
        "..",
        "../escape",
        "/absolute",
        "a/b",
        "a\\b",
        "a\0b",
        "a\nb",
        "日常",
        "External",
        "Stored",
    ] {
        assert!(
            engine
                .configure("en-US", revision, change("create-profile", name))
                .is_err(),
            "{name:?}"
        );
        assert_eq!(engine.configuration_revision(), revision);
        assert_eq!(engine.profiles(), profiles);
        assert_eq!(engine.candidates(), before);
    }
    assert_eq!(
        std::fs::read_to_string(config.join("profiles/External/keep")).unwrap(),
        "untouched"
    );
    assert_eq!(
        std::fs::read_to_string(data.join("profiles/Stored/keep")).unwrap(),
        "untouched"
    );
    assert!(!config.join("profiles/Stored").exists());
    assert!(!data.join("profiles/External").exists());
}
