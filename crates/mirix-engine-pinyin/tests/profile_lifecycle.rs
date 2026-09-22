use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::{
    ConfigurationEvent, ConfigurationValue, Engine, VocabularyOperation, VocabularyResult,
    VocabularyWord,
};

fn configure(engine: &mut PinyinEngine, id: &str, name: &str) {
    engine
        .configure(
            "en-US",
            engine.configuration_revision(),
            ConfigurationEvent::Change {
                id: id.into(),
                value: ConfigurationValue::String(name.into()),
            },
        )
        .unwrap();
}

fn words(engine: &mut PinyinEngine) -> usize {
    let VocabularyResult::Words { total, .. } = engine
        .vocabulary(
            None,
            VocabularyOperation::List {
                query: String::new(),
                offset: 0,
                limit: 10,
            },
        )
        .unwrap()
        .value
    else {
        panic!("list")
    };
    total
}

#[test]
fn profile_operations_keep_writer_state_in_sync_and_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    configure(&mut engine, "create-profile", "work");
    engine
        .vocabulary(
            None,
            VocabularyOperation::Add {
                word: VocabularyWord {
                    text: "输入".into(),
                    reading: "shū rù".into(),
                },
            },
        )
        .unwrap();
    configure(&mut engine, "rename-profile", "coding");
    assert_eq!(engine.profiles().active.as_deref(), Some("coding"));
    configure(&mut engine, "duplicate-profile", "copy");
    assert_eq!(words(&mut engine), 1);
    engine
        .vocabulary(
            None,
            VocabularyOperation::Remove {
                word: VocabularyWord {
                    text: "输入".into(),
                    reading: "shū rù".into(),
                },
            },
        )
        .unwrap();
    assert_eq!(words(&mut engine), 0);
    configure(&mut engine, "profile", "coding");
    assert_eq!(words(&mut engine), 1);
    configure(&mut engine, "profile", "copy");
    engine
        .configure(
            "en-US",
            engine.configuration_revision(),
            ConfigurationEvent::Invoke {
                id: "archive-profile".into(),
            },
        )
        .unwrap();
    assert!(!engine.profiles().available.contains(&"copy".into()));
    drop(engine);
    assert!(!config.join("profiles/work").exists());
    assert!(!data.join("profiles/work").exists());
    assert!(!data.join("profiles/copy").exists());
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    configure(&mut engine, "profile", "coding");
    assert_eq!(words(&mut engine), 1);
    assert_eq!(
        std::fs::read_dir(data.join("archived-profiles"))
            .unwrap()
            .count(),
        1
    );
}
