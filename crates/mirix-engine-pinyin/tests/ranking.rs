use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::{
    Engine, KeyEvent, KeyState, Modifiers, VocabularyOperation, VocabularyWord,
};

const WORDS: &str = "haishi\thai shi\t还是\t10000\n\
haishi\thai shi\t海事\t9000\n\
huashi\thua shi\t花市\t8000\n\
heshi\the shi\t合适\t7000\n\
hesi\the si\t赫斯\t500\n\
hashi\tha shi\t哈市\t100\n\
haisi\thai si\t海丝\t100\n\
haishi\thai shi\t海狮\t100\n";

fn input(engine: &mut PinyinEngine, text: &str) {
    engine.reset();
    for letter in text.chars() {
        engine.process_key(&KeyEvent {
            keycode: 0,
            keysym: letter as u32,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: Some(letter.to_string()),
        });
    }
}

fn texts(engine: &PinyinEngine) -> Vec<&str> {
    engine
        .candidates()
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect()
}

fn empty_engine() -> (tempfile::TempDir, PinyinEngine) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("characters.tsv"), "").unwrap();
    let engine = PinyinEngine::from_config(root.path()).unwrap();
    (root, engine)
}

#[test]
fn two_frequent_phrases_lead_full_initial_and_mixed_inputs_without_duplicates() {
    let (_root, mut engine) = empty_engine();
    assert!(engine.extend_dictionary(WORDS).is_empty());
    for spelling in ["haishi", "hs", "hais", "hshi", "h's", "hs", "hais"] {
        input(&mut engine, spelling);
        let words = texts(&engine);
        assert_eq!(&words[..2], ["还是", "海事"], "{spelling}: {words:?}");
        assert_eq!(
            words.iter().collect::<std::collections::HashSet<_>>().len(),
            words.len()
        );
        assert_eq!(engine.candidates()[0].reading.as_deref(), Some("hai shi"));
    }
    input(&mut engine, "hais");
    assert_eq!(&texts(&engine)[..3], ["还是", "海事", "海丝"]);
    input(&mut engine, "hs");
    assert_eq!(&texts(&engine)[..3], ["还是", "海事", "赫斯"]);
    assert!(
        engine
            .extend_dictionary("haishi\thai shi\t海事\t20000\n")
            .is_empty()
    );
    input(&mut engine, "hs");
    assert_eq!(&texts(&engine)[..2], ["海事", "还是"]);
}

#[test]
fn frequent_recall_survives_a_full_shorter_completion_list() {
    let (_root, mut engine) = empty_engine();
    let mut dictionary = (0..100)
        .map(|i| format!("haisi\thai si\t短词{i}\t1\n"))
        .collect::<String>();
    dictionary.push_str(WORDS);
    assert!(engine.extend_dictionary(&dictionary).is_empty());
    input(&mut engine, "hais");
    assert_eq!(engine.candidates().len(), 72);
    assert_eq!(&texts(&engine)[..2], ["还是", "海事"]);
    let selected = engine.candidates()[0].id;
    assert_eq!(engine.select(selected).commit.unwrap().text, "还是");
}

#[test]
fn preferred_phrases_respect_extensions_hiding_pins_and_learning() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    std::fs::create_dir_all(config.join("extensions")).unwrap();
    std::fs::write(config.join("extensions/exact.tsv"), "hs\t自定义\t1\n").unwrap();
    let mut engine = PinyinEngine::open(root.path().join("data"), config).unwrap();
    assert!(engine.extend_dictionary(WORDS).is_empty());
    input(&mut engine, "hs");
    assert_eq!(&texts(&engine)[..3], ["自定义", "还是", "海事"]);
    assert!(engine.candidates()[0].reading.is_none());
    engine
        .vocabulary(
            None,
            VocabularyOperation::Hide {
                word: VocabularyWord {
                    text: "还是".into(),
                    reading: "hai shi".into(),
                },
                enabled: true,
            },
        )
        .unwrap();
    assert_eq!(&texts(&engine)[..3], ["自定义", "海事", "花市"]);
    assert!(!texts(&engine).contains(&"还是"));
    engine
        .vocabulary(
            None,
            VocabularyOperation::Pin {
                word: VocabularyWord {
                    text: "合适".into(),
                    reading: "he shi".into(),
                },
                enabled: true,
            },
        )
        .unwrap();
    assert_eq!(&texts(&engine)[..3], ["自定义", "合适", "海事"]);
    engine
        .vocabulary(
            None,
            VocabularyOperation::Pin {
                word: VocabularyWord {
                    text: "合适".into(),
                    reading: "he shi".into(),
                },
                enabled: false,
            },
        )
        .unwrap();
    assert!(
        engine
            .extend_dictionary("haishi\thai shi\t海狮\t8000\n")
            .is_empty()
    );
    input(&mut engine, "hais");
    assert_eq!(texts(&engine)[0], "海事");
    for _ in 0..12 {
        input(&mut engine, "haishi");
        let id = engine
            .candidates()
            .iter()
            .find(|candidate| candidate.text == "海狮")
            .unwrap()
            .id;
        engine.select(id);
    }
    input(&mut engine, "hais");
    assert_eq!(texts(&engine)[0], "海狮");
}
