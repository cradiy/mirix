use std::fs;
use std::path::Path;

use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::{Engine, EngineOutput, KeyEvent, KeyState, Modifiers};

fn key(text: char) -> KeyEvent {
    KeyEvent {
        keycode: 0,
        keysym: text as u32,
        state: KeyState::Press,
        mods: Modifiers::default(),
        text: Some(text.to_string()),
    }
}

fn symbol(engine: &mut PinyinEngine, keysym: u32) -> EngineOutput {
    engine.process_key(&KeyEvent {
        keysym,
        text: None,
        ..key(' ')
    })
}

fn type_text(engine: &mut PinyinEngine, input: &str) {
    for character in input.chars() {
        engine.process_key(&key(character));
    }
}

fn words(engine: &PinyinEngine) -> Vec<&str> {
    engine
        .candidates()
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect()
}

fn write(path: impl AsRef<Path>, text: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn config(root: &Path) -> std::path::PathBuf {
    let config = root.join("config");
    write(config.join("characters.tsv"), "你\tni\t10\n好\thao\t8\n");
    config
}

#[test]
fn extensions_use_normal_selection_cancellation_and_pagination() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    write(
        config.join("extensions/completion.tsv"),
        "google\tGoogle\t1000\ngood\tgood\t800\n",
    );
    write(
        config.join("extensions/exact.tsv"),
        "goo\tGoole\t1000\nsmile\t👩‍💻\t100\n",
    );
    let mut engine = PinyinEngine::from_config(&config).unwrap();
    type_text(&mut engine, "goo");
    assert_eq!(words(&engine), ["Goole", "good", "Google"]);
    assert_eq!(engine.process_key(&key(' ')).commit.unwrap().text, "Goole");
    assert!(engine.candidates().is_empty());

    type_text(&mut engine, "goo");
    engine.set_candidate_pages(vec![1, 3]);
    engine.process_key(&key('='));
    assert_eq!(engine.process_key(&key('1')).commit.unwrap().text, "good");

    type_text(&mut engine, "smile");
    let id = engine.candidates()[0].id;
    assert_eq!(engine.select(id).commit.unwrap().text, "👩‍💻");
    assert!(engine.select(id).commit.is_none());

    type_text(&mut engine, "goo");
    assert_eq!(
        engine.process_key(&key(',')).commit.unwrap().text,
        "Goole，"
    );
    type_text(&mut engine, "goo");
    assert_eq!(symbol(&mut engine, 0xff0d).commit.unwrap().text, "goo");
    type_text(&mut engine, "goo");
    let id = engine.candidates()[0].id;
    symbol(&mut engine, 0xff1b);
    assert!(engine.candidates().is_empty());
    assert!(engine.select(id).commit.is_none());
    type_text(&mut engine, "goo");
    symbol(&mut engine, 0xff08);
    assert_eq!(words(&engine), ["good", "Google"]);
    engine.reset();
    assert!(engine.candidates().is_empty());
}

#[test]
fn profile_extensions_merge_with_shared_data_and_reload_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let data = root.path().join("data");
    write(
        config.join("extensions/completion.tsv"),
        "google\tGoogle\t1000\n",
    );
    write(
        config.join("profiles/日常/extensions/completion.tsv"),
        "good\tgood\t800\n",
    );
    write(
        config.join("profiles/编程/extensions/completion.tsv"),
        "google\tGoogle\t2000\ngolang\tGolang\t1500\n",
    );
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    type_text(&mut engine, "go");
    assert_eq!(words(&engine), ["good", "Google"]);
    let id = engine.candidates()[0].id;
    engine.activate_profile("编程").unwrap();
    assert!(engine.select(id).commit.is_none());
    assert_eq!(words(&engine), ["Google", "Golang"]);
    assert_eq!(engine.process_key(&key(' ')).commit.unwrap().text, "Google");
    drop(engine);
    assert!(!data.join("profiles/编程/learning.json").exists());
    write(
        config.join("profiles/编程/extensions/completion.tsv"),
        "godot\tGodot\t3000\n",
    );
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    assert_eq!(engine.profiles().active.as_deref(), Some("编程"));
    type_text(&mut engine, "go");
    assert_eq!(words(&engine), ["Godot", "Google"]);
    engine.activate_profile("日常").unwrap();
    assert_eq!(words(&engine), ["good", "Google"]);
}

#[test]
fn pinyin_and_extensions_deduplicate_without_cross_learning() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let data = root.path().join("data");
    write(
        config.join("extensions/exact.tsv"),
        "ni\t你\t1000\nhao\t😊\t100\n",
    );
    write(
        config.join("extensions/completion.tsv"),
        "nice\tnice\t100\nhao\t😊\t200\n",
    );
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    type_text(&mut engine, "ni");
    assert_eq!(words(&engine), ["你", "nice"]);
    // An explicit mapping owns this result, even when its text is also in the pinyin dictionary.
    assert!(engine.candidates()[0].reading.is_none());
    engine.process_key(&key(' '));
    type_text(&mut engine, "hao");
    assert_eq!(words(&engine), ["😊", "好"]);
    engine.process_key(&key('1'));
    drop(engine);
    assert!(!data.join("profiles/日常/learning.json").exists());

    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    type_text(&mut engine, "hao");
    engine.process_key(&key('2'));
    drop(engine);
    let learned = fs::read_to_string(data.join("profiles/日常/learning.json")).unwrap();
    assert!(learned.contains("好"));
    assert!(!learned.contains("你"));
    assert!(!learned.contains("😊"));
}

#[test]
fn all_extension_sources_and_pinyin_remain_in_the_same_candidate_list() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    write(
        config.join("extensions/exact.tsv"),
        "ni  自定义内容  1000\nni\u{3000}😊\u{00a0}100\n",
    );
    write(
        config.join("extensions/completion.tsv"),
        "nice\t nice 800\n",
    );
    let mut engine = PinyinEngine::from_config(&config).unwrap();
    type_text(&mut engine, "ni");
    assert_eq!(words(&engine), ["自定义内容", "😊", "你", "nice"]);
    // The pinyin result is still selectable even when every extension source matches.
    assert_eq!(engine.process_key(&key('3')).commit.unwrap().text, "你");
}

#[test]
fn exact_extensions_precede_a_full_pinyin_list_and_both_remain_pageable() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let words = (0..80)
        .map(|i| format!("ni\titem{i:03}\t{i}\n"))
        .collect::<String>();
    let pinyin = (0..80)
        .map(|i| format!("ni\tnǐ\t拼音{i:03}\t{i}\n"))
        .collect::<String>();
    write(config.join("extensions/completion.tsv"), &words);
    write(config.join("dictionaries/test.tsv"), &pinyin);
    let mut engine = PinyinEngine::from_config(&config).unwrap();
    type_text(&mut engine, "ni");
    assert_eq!(engine.candidates().len(), 144);
    assert_eq!(engine.candidates()[0].text, "item079");
    assert_eq!(engine.candidates()[71].text, "item008");
    assert_eq!(engine.candidates()[72].text, "你");
    engine.set_candidate_pages((9..=144).step_by(9).collect());
    for _ in 0..8 {
        engine.process_key(&key('='));
    }
    assert_eq!(engine.process_key(&key('1')).commit.unwrap().text, "你");
}

#[test]
fn complete_pinyin_outranks_prefixes_and_retains_its_learning_metadata() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let dictionary = (0..80)
        .map(|i| format!("ni\tnǐ\t拼音{i:03}\t{i}\n"))
        .collect::<String>();
    write(config.join("dictionaries/sample.tsv"), &dictionary);
    let mut baseline = PinyinEngine::from_config(&config).unwrap();
    type_text(&mut baseline, "ni");
    assert_eq!(baseline.candidates().len(), 72);

    write(
        config.join("extensions/completion.tsv"),
        "nice 拼音079 9999\nnice First 300\nnice Second 200\nnice Third 100\n",
    );
    let mut engine = PinyinEngine::from_config(&config).unwrap();
    type_text(&mut engine, "ni");
    assert_eq!(&words(&engine)[..2], ["你", "拼音079"]);
    assert_eq!(&words(&engine)[72..], ["First", "Second", "Third"]);
    assert_eq!(engine.candidates().len(), 75);
    assert_eq!(engine.candidates()[74].text, "Third");
    let pinyin = engine
        .candidates()
        .iter()
        .filter(|candidate| candidate.reading.is_some());
    for (actual, expected) in pinyin.zip(baseline.candidates()) {
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.reading, expected.reading);
    }
    let id = engine.candidates()[72].id;
    assert_eq!(engine.select(id).commit.unwrap().text, "First");
}

#[test]
fn mixed_candidates_rank_by_completion_distance_without_source_quotas() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    write(
        config.join("extensions/completion.tsv"),
        "ha Exact 1\nhas Has 30\nhat Hat 20\nham Ham 10\nhallo Hallo 99999\n",
    );
    let mut engine = PinyinEngine::from_config(&config).unwrap();
    type_text(&mut engine, "ha");
    assert_eq!(
        words(&engine),
        ["Exact", "Has", "Hat", "Ham", "好", "Hallo"]
    );
    assert!(engine.candidates()[4].reading.is_some());
    // Retyping the full pinyin promotes its exact match above English prefixes.
    engine.process_key(&key('o'));
    assert_eq!(words(&engine), ["好"]);
}

#[test]
fn startup_seeds_shared_and_profile_files_without_overwriting_entries() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    let data = root.path().join("data");
    let shared = "addr \"Room 12, Main Street\" 100\n";
    write(config.join("extensions/exact.tsv"), shared);
    drop(PinyinEngine::open(&data, &config).unwrap());
    assert_eq!(
        fs::read_to_string(config.join("extensions/exact.tsv")).unwrap(),
        shared
    );
    assert!(
        fs::read(config.join("extensions/completion.tsv"))
            .unwrap()
            .is_empty()
    );
    for name in ["exact.tsv", "completion.tsv"] {
        assert!(
            fs::read(config.join("profiles/日常/extensions").join(name))
                .unwrap()
                .is_empty()
        );
    }

    let profile = "smile 😊 50\n";
    write(
        config.join("profiles/Work/extensions/completion.tsv"),
        profile,
    );
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    assert_eq!(
        fs::read_to_string(config.join("profiles/Work/extensions/completion.tsv")).unwrap(),
        profile
    );
    assert!(
        fs::read(config.join("profiles/Work/extensions/exact.tsv"))
            .unwrap()
            .is_empty()
    );
    engine.activate_profile("Work").unwrap();
    type_text(&mut engine, "smi");
    assert!(words(&engine).contains(&"😊"));
    assert_eq!(
        fs::read_to_string(config.join("extensions/exact.tsv")).unwrap(),
        shared
    );
}

#[test]
fn exact_and_completion_entries_share_weights_and_allow_any_output() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path());
    write(
        config.join("extensions/exact.tsv"),
        "smile Text 10\nsmile 😊 20\n",
    );
    write(
        config.join("extensions/completion.tsv"),
        "smile 😊 100\nsmile \"Have a nice day\" 50\n",
    );
    let mut engine = PinyinEngine::from_config(&config).unwrap();
    type_text(&mut engine, "smi");
    assert_eq!(words(&engine), ["😊", "Have a nice day"]);
    type_text(&mut engine, "le");
    assert_eq!(words(&engine), ["😊", "Have a nice day", "Text"]);
}
