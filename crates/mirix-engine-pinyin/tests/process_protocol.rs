use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use mirix_engine_process::{InputMethod, ProcessEngine};
use mirix_engine_protocol::{Engine, KeyEvent, KeyState, Modifiers};

fn key(keysym: u32, text: Option<&str>) -> KeyEvent {
    KeyEvent {
        keycode: 0,
        keysym,
        state: KeyState::Press,
        mods: Modifiers::default(),
        text: text.map(str::to_owned),
    }
}

#[test]
fn resetting_a_leased_process_clears_child_composition_before_the_next_key() {
    let directory = tempfile::tempdir().unwrap();
    let launcher = directory.path().join("engine");
    std::fs::write(
        &launcher,
        format!(
            "#!/bin/sh\nexec '{}' --data-dir '{}' --config-dir '{}'\n",
            env!("CARGO_BIN_EXE_mirix-engine-pinyin"),
            directory.path().join("data").display(),
            directory.path().join("config").display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let engine = ProcessEngine::spawn(InputMethod {
        name: "leased-pinyin".into(),
        directory: directory.path().to_path_buf(),
        executable: launcher,
    })
    .unwrap();
    let mut instance = mirix_core::Instance::new(Box::new(engine));
    let context = instance.create_context("fixture");
    instance.focus_in(context);
    let epoch = instance.input_epoch();
    let mut worker = instance.take_engine_worker("leased-pinyin").unwrap();
    let output = worker.process_key(&key(b'n' as u32, Some("n")));
    assert_eq!(output.preedit_text(), Some("n"));
    instance.focus_out(context);
    instance.focus_in(context);
    instance.restore_engine_worker(worker);
    assert!(!instance.complete_key(context, epoch, &output));
    assert!(instance.candidates().is_empty());
    let output = instance.process_key(context, &key(b'a' as u32, Some("a")));
    assert_eq!(output.preedit_text(), Some("a"));
    assert!(output.commit.is_none());
}

#[test]
fn reference_engine_roundtrips_through_process_protocol() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_mirix-engine-pinyin"));
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("data");
    let config = directory.path().join("config");
    std::fs::create_dir_all(config.join("profiles/编程/dictionaries")).unwrap();
    std::fs::create_dir_all(config.join("profiles/日常/dictionaries")).unwrap();
    std::fs::create_dir_all(config.join("extensions")).unwrap();
    std::fs::create_dir_all(config.join("dictionaries")).unwrap();
    std::fs::write(
        config.join("dictionaries/sample.tsv"),
        "shurufa\tshū rù fǎ\t输入法\t10\n",
    )
    .unwrap();
    std::fs::write(
        config.join("extensions/exact.tsv"),
        "goo\tGoole\t1000\nsmile\t👩‍💻\t100\n",
    )
    .unwrap();
    let launcher = directory.path().join("engine");
    std::fs::write(
        &launcher,
        format!(
            "#!/bin/sh\nexec '{}' --data-dir '{}' --config-dir '{}'\n",
            executable.display(),
            root.display(),
            config.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let method = InputMethod {
        name: "process-pinyin".into(),
        directory: directory.path().to_path_buf(),
        executable: launcher,
    };
    let mut engine = ProcessEngine::spawn(method.clone()).unwrap();

    assert_eq!(engine.name(), "process-pinyin");
    let icon = engine.tray_icon().expect("engine supplies its tray icon");
    icon.validate().unwrap();
    assert_eq!(
        engine.punctuation_mode(),
        Some(mirix_engine_protocol::PunctuationMode::Full)
    );
    assert_eq!(engine.menus()[0].active.as_deref(), Some("日常"));
    assert_eq!(
        engine
            .process_key(&key(b'n' as u32, Some("n")))
            .preedit_text(),
        Some("n")
    );
    assert_eq!(
        engine
            .process_key(&key(b'i' as u32, Some("i")))
            .preedit_text(),
        Some("ni")
    );
    assert_eq!(engine.candidates()[0].text, "你");
    assert!(std::sync::Arc::ptr_eq(&icon, &engine.tray_icon().unwrap()));
    let stale = engine.candidates()[0].id;
    let effect = engine.select_menu("profiles", "编程").unwrap();
    assert!(matches!(effect.preedit_text(), Some("ni")));
    let candidates = engine.candidates().to_vec();
    let effect = engine.select_menu("profiles", "编程").unwrap();
    assert_eq!(effect, mirix_engine_protocol::EngineOutput::consume());
    assert_eq!(engine.candidates(), candidates);
    assert_eq!(engine.menus()[0].active.as_deref(), Some("编程"));
    assert!(engine.select(stale).commit.is_none());
    assert_eq!(engine.candidates()[0].text, "你");
    let count = engine.candidates().len();
    let mut ends = vec![1];
    let mut end = 1;
    while end < count {
        end = (end + 5).min(count);
        ends.push(end);
    }
    engine.set_candidate_pages(ends.clone());
    assert_eq!(engine.candidate_page_ends(), ends);
    engine.process_key(&key(b'=' as u32, Some("=")));
    assert_eq!(engine.selected_candidate(), 1);
    let snapshot = engine.candidates().to_vec();
    let page_snapshot = engine.candidate_page_ends().to_vec();
    assert!(matches!(
        engine
            .vocabulary(
                Some("编程"),
                mirix_engine_protocol::VocabularyOperation::List {
                    query: String::new(),
                    offset: 0,
                    limit: 10,
                }
            )
            .unwrap()
            .value,
        mirix_engine_protocol::VocabularyResult::Words { .. }
    ));
    assert_eq!(engine.candidates(), snapshot);
    assert_eq!(engine.selected_candidate(), 1);
    assert_eq!(engine.candidate_page_ends(), page_snapshot);
    // A validation error is recoverable and must not stop the process.
    assert!(
        engine
            .vocabulary(
                Some("日常"),
                mirix_engine_protocol::VocabularyOperation::Pin {
                    word: mirix_engine_protocol::VocabularyWord {
                        text: "泥".into(),
                        reading: "ni".into()
                    },
                    enabled: true,
                }
            )
            .is_err()
    );
    engine.process_key(&key(b'-' as u32, Some("-")));
    assert_eq!(engine.selected_candidate(), 0);
    let candidate_id = engine.candidates()[0].id;
    assert_eq!(
        engine.select(candidate_id).commit.map(|commit| commit.text),
        Some("你".into())
    );
    let punctuation = engine.process_key(&key(b',' as u32, Some(",")));
    assert!(punctuation.consumed);
    assert_eq!(punctuation.commit.unwrap().text, "，");
    engine.process_key(&key(b'n' as u32, Some("n")));
    engine.process_key(&key(b'i' as u32, Some("i")));
    let punctuation = engine.process_key(&key(b'!' as u32, Some("!")));
    assert!(punctuation.consumed);
    assert_eq!(punctuation.commit.unwrap().text, "你！");
    assert!(engine.candidates().is_empty());
    for (input, expected) in [("goo", "Goole"), ("smile", "👩‍💻")] {
        for character in input.chars() {
            engine.process_key(&key(character as u32, Some(&character.to_string())));
        }
        let candidate = engine
            .candidates()
            .iter()
            .find(|item| item.text == expected)
            .unwrap();
        let id = candidate.id;
        assert_eq!(engine.select(id).commit.unwrap().text, expected);
        assert!(engine.candidates().is_empty());
    }
    for input in ["srf", "shrf", "shuruf"] {
        for character in input.chars() {
            engine.process_key(&key(character as u32, Some(&character.to_string())));
        }
        assert_eq!(engine.candidates()[0].text, "输入法", "{input}");
        assert_eq!(engine.candidates()[0].reading.as_deref(), Some("shū rù fǎ"));
        assert_eq!(
            engine
                .process_key(&key(mirix_engine_protocol::keysym::SPACE, Some(" ")))
                .commit
                .unwrap()
                .text,
            "输入法"
        );
    }
    let mut toggle = key('.' as u32, Some("."));
    toggle.mods.ctrl = true;
    assert!(engine.process_key(&toggle).consumed);
    assert_eq!(
        engine.punctuation_mode(),
        Some(mirix_engine_protocol::PunctuationMode::Half)
    );
    toggle.state = KeyState::Release;
    engine.process_key(&toggle);
    engine.reset();
    assert_eq!(
        engine.punctuation_mode(),
        Some(mirix_engine_protocol::PunctuationMode::Half)
    );
    assert_eq!(
        engine
            .process_key(&key(',' as u32, Some(",")))
            .commit
            .unwrap()
            .text,
        ","
    );
    for character in ['n', 'i'] {
        engine.process_key(&key(character as u32, Some(&character.to_string())));
    }
    engine
        .vocabulary(
            Some("编程"),
            mirix_engine_protocol::VocabularyOperation::Pin {
                word: mirix_engine_protocol::VocabularyWord {
                    text: "泥".into(),
                    reading: "ni".into(),
                },
                enabled: true,
            },
        )
        .unwrap();
    assert_eq!(engine.candidates()[0].text, "泥");
    assert_eq!(
        engine
            .process_key(&key(mirix_engine_protocol::keysym::SPACE, Some(" ")))
            .commit
            .unwrap()
            .text,
        "泥"
    );
    drop(engine);
    let learning = std::fs::read_to_string(root.join("profiles/编程/learning.json")).unwrap();
    assert!(learning.contains("你"));
    assert!(!learning.contains("Goole"));
    assert!(!learning.contains("👩‍💻"));
    assert!(!root.join("profiles/日常/learning.json").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("active-profile.json")).unwrap(),
        "\"编程\""
    );
    let mut engine = ProcessEngine::spawn(method.clone()).unwrap();
    let page = engine.configuration("en-US").unwrap();
    let event = mirix_engine_protocol::ConfigurationEvent::Change {
        id: "create-profile".into(),
        value: mirix_engine_protocol::ConfigurationValue::String("Writing 写作".into()),
    };
    page.validate_event(&event).unwrap();
    engine
        .configure("en-US", engine.configuration_revision(), event.clone())
        .unwrap();
    assert_eq!(engine.menus()[0].active.as_deref(), Some("Writing 写作"));
    let page = engine.configuration("en-US").unwrap();
    assert!(matches!(&page.groups[0].controls[0].kind,
        mirix_engine_protocol::ConfigurationKind::Select { value, choices }
        if value == "Writing 写作" && choices.iter().any(|choice| choice.value == *value)));
    assert!(
        engine
            .configure("en-US", engine.configuration_revision(), event)
            .is_err()
    );
    assert!(engine.configuration("en-US").is_ok());
    drop(engine);
    let engine = ProcessEngine::spawn(method).unwrap();
    assert_eq!(engine.menus()[0].active.as_deref(), Some("Writing 写作"));
}
