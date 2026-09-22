use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::{
    Engine, KeyEvent, KeyState, Modifiers, VocabularyOperation as Op, VocabularyResult,
    VocabularyWord,
};

fn word(text: &str, reading: &str) -> VocabularyWord {
    VocabularyWord {
        text: text.into(),
        reading: reading.into(),
    }
}

fn setup() -> (tempfile::TempDir, PinyinEngine) {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    for profile in ["日常", "编程"] {
        std::fs::create_dir_all(config.join("profiles").join(profile)).unwrap();
    }
    std::fs::write(
        config.join("characters.tsv"),
        "你 nǐ ni 100\n泥 ní ni 10\n好 hǎo hao 100\n西 xī xi 100\n先 xiān xian 100\n安 ān an 100\n",
    )
    .unwrap();
    let engine = PinyinEngine::open(root.path().join("data"), config).unwrap();
    (root, engine)
}

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

fn choose(engine: &mut PinyinEngine, text: &str) {
    let id = engine
        .candidates()
        .iter()
        .find(|candidate| candidate.text == text)
        .unwrap()
        .id;
    assert_eq!(engine.select(id).commit.unwrap().text, text);
}

fn entries(engine: &mut PinyinEngine) -> Vec<mirix_engine_protocol::VocabularyEntry> {
    let VocabularyResult::Words { entries, .. } = engine
        .vocabulary(
            None,
            Op::List {
                query: String::new(),
                offset: 0,
                limit: 100,
            },
        )
        .unwrap()
        .value
    else {
        panic!()
    };
    entries
}

#[test]
fn forgetting_removes_counts_and_contexts_without_removing_shared_words() {
    let (root, mut engine) = setup();
    for _ in 0..3 {
        input(&mut engine, "ni");
        choose(&mut engine, "泥");
    }
    input(&mut engine, "ni");
    assert_eq!(engine.candidates()[0].text, "泥");
    engine
        .vocabulary(
            Some("日常"),
            Op::Forget {
                word: word("泥", "ni"),
            },
        )
        .unwrap();
    assert_eq!(engine.candidates()[0].text, "你");
    assert!(
        engine
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "泥")
    );
    assert!(entries(&mut engine).is_empty());
    drop(engine);
    let mut restored =
        PinyinEngine::open(root.path().join("data"), root.path().join("config")).unwrap();
    assert!(entries(&mut restored).is_empty());
}

#[test]
fn pin_hide_and_noop_updates_preserve_input_and_are_profile_scoped() {
    let (root, mut engine) = setup();
    input(&mut engine, "ni");
    let stale = engine.candidates()[0].id;
    engine
        .vocabulary(
            None,
            Op::Pin {
                word: word("泥", "ni"),
                enabled: true,
            },
        )
        .unwrap();
    assert_eq!(engine.candidates()[0].text, "泥");
    assert!(engine.select(stale).commit.is_none());
    let snapshot = engine.candidates().to_vec();
    assert!(matches!(
        engine
            .vocabulary(
                None,
                Op::Pin {
                    word: word("泥", "ní"),
                    enabled: true
                }
            )
            .unwrap()
            .value,
        VocabularyResult::Updated { changed: false, .. }
    ));
    assert_eq!(engine.candidates(), snapshot);
    engine
        .vocabulary(
            None,
            Op::Hide {
                word: word("泥", "ni"),
                enabled: true,
            },
        )
        .unwrap();
    assert!(
        !engine
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "泥")
    );
    engine.activate_profile("编程").unwrap();
    assert!(
        engine
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "泥")
    );
    let snapshot = engine.candidates().to_vec();
    assert!(
        engine
            .vocabulary(
                Some("日常"),
                Op::Hide {
                    word: word("你", "ni"),
                    enabled: true
                }
            )
            .is_err()
    );
    assert_eq!(engine.candidates(), snapshot);
    drop(engine);
    let mut restored =
        PinyinEngine::open(root.path().join("data"), root.path().join("config")).unwrap();
    restored.activate_profile("日常").unwrap();
    input(&mut restored, "ni");
    assert!(
        !restored
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "泥")
    );
    restored
        .vocabulary(
            None,
            Op::Hide {
                word: word("泥", "ni"),
                enabled: false,
            },
        )
        .unwrap();
    assert_eq!(restored.candidates()[0].text, "泥");
    restored
        .vocabulary(
            None,
            Op::Pin {
                word: word("泥", "ni"),
                enabled: false,
            },
        )
        .unwrap();
    assert_eq!(restored.candidates()[0].text, "你");
}

#[test]
fn personal_words_recall_abbreviations_without_fabricating_usage_and_can_be_removed() {
    let (root, mut engine) = setup();
    let entry = word("你好呀", " nǐ\t hǎo'yā ");
    engine.vocabulary(None, Op::Add { word: entry }).unwrap();
    assert_eq!(entries(&mut engine)[0].count, 0);
    assert!(entries(&mut engine)[0].personal);
    input(&mut engine, "nhy");
    choose(&mut engine, "你好呀");
    assert_eq!(entries(&mut engine)[0].count, 1);
    engine
        .vocabulary(
            None,
            Op::Remove {
                word: word("你好呀", "ni hao ya"),
            },
        )
        .unwrap();
    input(&mut engine, "nhy");
    assert!(
        !engine
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "你好呀")
    );
    drop(engine);
    let mut restored =
        PinyinEngine::open(root.path().join("data"), root.path().join("config")).unwrap();
    assert!(entries(&mut restored).is_empty());
}

#[test]
fn hidden_characters_keep_syllable_boundaries_and_can_still_compose_other_words() {
    let (_root, mut engine) = setup();
    engine
        .vocabulary(
            None,
            Op::Hide {
                word: word("先", "xian"),
                enabled: true,
            },
        )
        .unwrap();
    input(&mut engine, "xian'");
    assert!(engine.candidates().is_empty(), "must not split into xi an");
    input(&mut engine, "xianhao");
    assert!(
        engine
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "先好")
    );
}

#[test]
fn pins_are_recalled_before_candidate_truncation_but_cannot_override_exact_or_quote_constraints() {
    let (_root, mut engine) = setup();
    for index in 0..100 {
        engine.extend_dictionary(&format!("ni\tnǐ\t词{index}\t100\n"));
    }
    engine
        .vocabulary(
            None,
            Op::Pin {
                word: word("目标", "ni"),
                enabled: true,
            },
        )
        .unwrap();
    input(&mut engine, "ni");
    // The exact single characters retain their structural priority.
    assert_eq!(engine.candidates()[0].text, "你");
    assert!(
        engine
            .candidates()
            .iter()
            .take(3)
            .any(|candidate| candidate.text == "目标")
    );
    engine
        .vocabulary(
            None,
            Op::Pin {
                word: word("西安", "xi an"),
                enabled: true,
            },
        )
        .unwrap();
    input(&mut engine, "xian");
    assert_eq!(engine.candidates()[0].text, "先");
    input(&mut engine, "xian'");
    assert!(
        !engine
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "西安")
    );
}

#[test]
fn malformed_additions_do_not_change_state_and_lists_are_paginated() {
    let (_root, mut engine) = setup();
    input(&mut engine, "ni");
    let snapshot = engine.candidates().to_vec();
    assert!(
        engine
            .vocabulary(
                None,
                Op::Add {
                    word: word("bad", "unknown")
                }
            )
            .is_err()
    );
    assert_eq!(engine.candidates(), snapshot);
    for index in 0..3 {
        engine
            .vocabulary(
                None,
                Op::Add {
                    word: word(&format!("词{index}"), "ci"),
                },
            )
            .unwrap();
    }
    let VocabularyResult::Words {
        entries,
        total,
        offset,
        ..
    } = engine
        .vocabulary(
            None,
            Op::List {
                query: "ci".into(),
                offset: 1,
                limit: 1,
            },
        )
        .unwrap()
        .value
    else {
        panic!()
    };
    assert_eq!((total, offset, entries.len()), (3, 1, 1));
    assert_eq!(entries[0].word.text, "词1");
}
