use super::*;
use crate::{Parser, Reading, SyllableTable};

fn word(table: &SyllableTable, reading: &[&str], text: &str, weight: u64) -> Lexeme {
    Lexeme::new(Reading::new(table, reading, reading).unwrap(), text, weight)
}

#[test]
fn duplicate_completions_leave_room_for_abbreviated_candidates() {
    let parser = Parser::standard();
    let table = parser.table();
    let lexicon = Lexicon::new(vec![
        word(table, &["shi"], "同", 10),
        word(table, &["shang"], "同", 10),
        word(table, &["shu"], "同", 10),
        word(table, &["shi", "hao"], "诗好", 10),
    ]);
    let candidates = Decoder::default().decode(&parser.parse("sh").unwrap(), &lexicon, 2);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.text.as_str())
            .collect::<Vec<_>>(),
        ["同", "诗好"]
    );
    assert!(candidates[1].abbreviated_syllables > 0);
}

#[test]
fn shared_composition_prefixes_match_fresh_search_for_each_tail() {
    let parser = Parser::standard();
    let table = parser.table();
    let lexicon = Lexicon::new(vec![
        word(table, &["ni"], "你", 10),
        word(table, &["hao"], "好", 10),
        word(table, &["shi"], "诗", 10),
        word(table, &["shang"], "商", 10),
        word(table, &["shu"], "书", 10),
        word(table, &["ni", "hao"], "你好", 100),
        word(table, &["ni", "hao", "shi"], "你好诗", 500),
    ]);
    for input in ["nihaosh", "ni'hao'sh"] {
        let mut graph = parser.parse(input).unwrap();
        graph.constrain_closed_syllables(|id| !lexicon.exact(&[id]).is_empty());
        for width in [1, 3, 16] {
            let decoder = Decoder::with_limits(256, width);
            let mut shared = Composition::default();
            for tail in ["shi", "shang", "shu", "shi"] {
                let path = ["ni", "hao", tail].map(|syllable| table.id(syllable).unwrap());
                let fresh = decoder.compose_path(
                    &graph,
                    &path,
                    &lexicon,
                    &mut Composition::default(),
                    &|_, _| 0,
                );
                assert!(!fresh.is_empty());
                assert_eq!(
                    decoder.compose_path(&graph, &path, &lexicon, &mut shared, &|_, _| 0),
                    fresh,
                    "{input}, {tail}, width={width}"
                );
            }
        }
    }
}

#[test]
fn initials_digraphs_and_mixed_full_syllables_recall_real_phrases() {
    let parser = Parser::standard();
    let table = parser.table();
    let lexicon = Lexicon::new(vec![
        word(table, &["ni", "hao"], "你好", 10),
        word(table, &["shi", "jie"], "世界", 10),
        word(table, &["dang", "qian"], "当前", 10),
        word(table, &["shu", "ru", "fa"], "输入法", 10),
        word(table, &["chu", "xian"], "出现", 10),
        word(table, &["zhong", "guo"], "中国", 10),
    ]);
    for (input, expected) in [
        ("nh", "你好"),
        ("nhao", "你好"),
        ("nih", "你好"),
        ("ni'h", "你好"),
        ("n'hao", "你好"),
        ("sj", "世界"),
        ("shj", "世界"),
        ("shij", "世界"),
        ("s'jie", "世界"),
        ("sh'jie", "世界"),
        ("dq", "当前"),
        ("dangq", "当前"),
        ("d'qian", "当前"),
        ("srf", "输入法"),
        ("shrf", "输入法"),
        ("cx", "出现"),
        ("chx", "出现"),
        ("zg", "中国"),
        ("zhg", "中国"),
    ] {
        let graph = parser.parse(input).unwrap();
        let candidates = Decoder::default().decode(&graph, &lexicon, 10);
        assert_eq!(
            candidates.first().map(|candidate| candidate.text.as_str()),
            Some(expected),
            "{input}: {candidates:?}"
        );
        let ids = candidates[0]
            .display_reading
            .split_whitespace()
            .map(|part| table.id(part).unwrap())
            .collect::<Vec<_>>();
        assert!(
            graph.syllable_edges(&ids).is_some(),
            "learning must resolve {input}"
        );
    }
}

#[test]
fn abbreviations_can_compose_characters_without_a_phrase_entry() {
    let parser = Parser::standard();
    let table = parser.table();
    let lexicon = Lexicon::new(vec![
        word(table, &["ni"], "你", 10),
        word(table, &["hao"], "好", 10),
    ]);
    for input in ["nh", "nhao", "n'h"] {
        let candidates = Decoder::default().decode(&parser.parse(input).unwrap(), &lexicon, 10);
        assert_eq!(candidates[0].text, "你好");
        assert_eq!(candidates[0].abbreviated_syllables, 1);
    }
}

#[test]
fn exact_matches_outrank_abbreviations_regardless_of_frequency() {
    let parser = Parser::standard();
    let table = parser.table();
    let lexicon = Lexicon::new(vec![
        word(table, &["fan"], "帆", 1),
        word(table, &["fa", "na"], "发那", u64::MAX),
        word(table, &["fei", "an"], "飞安", u64::MAX),
        word(table, &["shi", "jie"], "世界", 1),
    ]);
    let candidates = Decoder::default().decode(&parser.parse("fan").unwrap(), &lexicon, 20);
    assert_eq!(candidates[0].text, "帆");
    assert!(candidates.iter().any(|candidate| candidate.text == "飞安"));
    let candidates = Decoder::default().decode(&parser.parse("shijie").unwrap(), &lexicon, 20);
    assert_eq!(candidates[0].text, "世界");
}

#[test]
fn quoted_whole_syllables_stay_locked_with_an_abbreviated_tail() {
    let parser = Parser::standard();
    let table = parser.table();
    let lexicon = Lexicon::new(vec![
        word(table, &["xuan"], "宣", 1),
        word(table, &["xu"], "许", 10000),
        word(table, &["an"], "安", 1),
        word(table, &["shi", "jie"], "世界", 1),
    ]);
    let candidates = Decoder::default().decode(&parser.parse("xuan'an'sj").unwrap(), &lexicon, 20);
    assert_eq!(candidates[0].text, "宣安世界");
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.display_reading.starts_with("xuan an "))
    );
}

#[test]
fn dictionary_guided_search_does_not_lose_late_high_frequency_readings() {
    let parser = Parser::standard();
    let table = parser.table();
    let mut entries = Vec::new();
    for (_, first) in table.iter().filter(|(_, text)| text.starts_with('s')) {
        for (_, second) in table.iter().filter(|(_, text)| text.starts_with('s')) {
            entries.push(word(
                table,
                &[first, second],
                &format!("{first}-{second}"),
                1,
            ));
        }
    }
    assert!(entries.len() > 256);
    entries.push(word(table, &["suo", "suo"], "目标", 1_000_000));
    let limit = entries.len();
    let candidates =
        Decoder::default().decode(&parser.parse("ss").unwrap(), &Lexicon::new(entries), limit);
    let target = candidates
        .iter()
        .position(|candidate| candidate.text == "目标")
        .unwrap();
    let same_reading = candidates
        .iter()
        .position(|candidate| candidate.text == "suo-suo")
        .unwrap();
    assert!(target < same_reading);
}
