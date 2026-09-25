use std::{fs, path::Path};

use mirix_engine_pinyin::PinyinEngine;
use mirix_engine_protocol::{ConfigurationEvent, Engine, KeyEvent, KeyState, Modifiers, keysym};

fn write(path: impl AsRef<Path>, text: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
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

fn reload(
    engine: &mut PinyinEngine,
) -> Result<mirix_engine_protocol::ActionResult<Option<String>>, String> {
    engine.configure(
        "en-US",
        engine.configuration_revision(),
        ConfigurationEvent::Invoke {
            id: "reload-dictionaries".into(),
        },
    )
}

fn words(engine: &PinyinEngine) -> Vec<&str> {
    engine
        .candidates()
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect()
}

#[test]
fn reload_replaces_all_sources_without_losing_composition_or_learning() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let data = root.path().join("data");
    write(config.join("characters.tsv"), "你\tni\t10\n尼\tni\t5\n");
    write(config.join("extensions/exact.tsv"), "ni shared 100\n");
    write(
        config.join("profiles/日常/extensions/exact.tsv"),
        "ni active 50\n",
    );
    write(
        config.join("profiles/Work/extensions/exact.tsv"),
        "ni work 50\n",
    );
    let mut engine = PinyinEngine::open(&data, &config).unwrap();
    for ch in "ni".chars() {
        engine.process_key(&key(ch));
    }
    let learned = engine
        .candidates()
        .iter()
        .find(|c| c.text == "尼")
        .unwrap()
        .id;
    engine.select(learned);
    for ch in "ni".chars() {
        engine.process_key(&key(ch));
    }
    engine.process_key(&KeyEvent {
        keysym: keysym::RIGHT,
        text: None,
        ..key(' ')
    });
    let selected = engine.candidates()[engine.selected_candidate()]
        .text
        .clone();
    let revision = engine.configuration_revision();

    write(
        config.join("characters.tsv"),
        "你\tni\t10\n尼\tni\t5\n拟\tni\t4\n",
    );
    write(config.join("extensions/exact.tsv"), "ni replacement 200\n");
    write(
        config.join("profiles/日常/dictionaries/words.tsv"),
        "ni\tnǐ\t倪\t20\n",
    );
    write(
        config.join("profiles/Work/extensions/exact.tsv"),
        "ni updated 50\n",
    );
    assert!(reload(&mut engine).unwrap().value.is_some());
    assert_eq!(
        engine.candidates()[engine.selected_candidate()].text,
        selected
    );
    assert!(engine.configuration_revision() > revision);
    let current = words(&engine);
    assert!(!current.contains(&"shared"));
    for word in ["replacement", "active", "拟", "倪"] {
        assert!(current.contains(&word), "{current:?}");
    }
    assert!(
        current.iter().position(|word| *word == "尼")
            < current.iter().position(|word| *word == "你")
    );
    engine.activate_profile("Work").unwrap();
    assert!(words(&engine).contains(&"updated"));
    assert!(!words(&engine).contains(&"work"));
    assert!(!words(&engine).contains(&"倪"));
    let output = engine.process_key(&KeyEvent {
        keysym: keysym::RETURN,
        text: None,
        ..key(' ')
    });
    assert_eq!(output.commit.unwrap().text, "ni");
}

#[test]
fn failed_reload_keeps_every_live_source_and_can_be_retried() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    write(config.join("characters.tsv"), "你\tni\t10\n");
    write(config.join("extensions/exact.tsv"), "ni original 100\n");
    write(
        config.join("profiles/Other/extensions/exact.tsv"),
        "ni other 50\n",
    );
    let mut engine = PinyinEngine::open(root.path().join("data"), &config).unwrap();
    for ch in "ni".chars() {
        engine.process_key(&key(ch));
    }
    let candidates = engine.candidates().to_vec();
    let selected = engine.selected_candidate();
    let revision = engine.configuration_revision();
    write(config.join("extensions/exact.tsv"), "ni replacement 100\n");
    let invalid = config.join("profiles/Other/extensions/exact.tsv");
    write(&invalid, "ni bad not-a-weight\n");
    let error = reload(&mut engine).unwrap_err();
    assert!(error.contains("Other/extensions/exact.tsv:1"), "{error}");
    assert_eq!(engine.candidates(), candidates);
    assert_eq!(engine.selected_candidate(), selected);
    assert_eq!(engine.configuration_revision(), revision);
    write(&invalid, "ni repaired 50\n");
    reload(&mut engine).unwrap();
    assert!(words(&engine).contains(&"replacement"));
    engine.activate_profile("Other").unwrap();
    assert!(words(&engine).contains(&"repaired"));
}
