use mirix_pinyin_core::{Decoder, Lexeme, Lexicon, Parser, Reading};

#[test]
fn bounded_frequent_recall_matches_exhaustive_ranking_with_ties_and_duplicate_words() {
    let parser = Parser::standard();
    let mut entries = Vec::new();
    for (group, syllables) in [
        ["hai", "shi"],
        ["hai", "shang"],
        ["hao", "shi"],
        ["hong", "se"],
    ]
    .into_iter()
    .enumerate()
    {
        let reading = Reading::new(parser.table(), &syllables, &syllables).unwrap();
        for index in 0..200 {
            // Duplicate text across readings, tied scores and stronger matches
            // reached later must not leave the bounded result short or misranked.
            let text = if index % 3 == 0 {
                format!("共享{index}")
            } else {
                format!("词{group}-{index}")
            };
            entries.push(Lexeme::new(
                reading.clone(),
                text,
                (200 - index / 4 + group * 20) as u64,
            ));
        }
        // Different frequencies can round to the same normalized score.
        for (text, frequency) in [("优先乙", 1_000_001), ("优先甲", 1_000_000)] {
            entries.push(Lexeme::new(reading.clone(), text, frequency));
        }
    }
    let lexicon = Lexicon::new(entries);
    let decoder = Decoder::default();
    for input in ["haishi", "hais", "hs", "h's", "haish", "hshi"] {
        let graph = parser.parse(input).unwrap();
        let exhaustive = decoder.frequent_phrases(&graph, &lexicon, lexicon.len() + 1);
        assert!(!exhaustive.is_empty(), "{input}");
        for limit in [1, 2, 9, 72, 150] {
            let bounded = decoder.frequent_phrases(&graph, &lexicon, limit);
            assert_eq!(
                bounded,
                exhaustive[..limit.min(exhaustive.len())],
                "{input}, limit={limit}"
            );
        }
    }
}

#[test]
fn frequent_phrase_recall_is_not_cut_off_by_segmentation_order_or_composition_budget() {
    let parser = Parser::standard();
    let table = parser.table();
    let mut entries = Vec::new();
    for (_, first) in table.iter().filter(|(_, text)| text.starts_with('s')) {
        for (_, second) in table.iter().filter(|(_, text)| text.starts_with('s')) {
            entries.push(Lexeme::new(
                Reading::new(table, &[first, second], &[first, second]).unwrap(),
                format!("{first}-{second}"),
                1,
            ));
        }
    }
    for (text, frequency) in [("琐琐", 1_000_000), ("缩缩", 500_000)] {
        entries.push(Lexeme::new(
            Reading::new(table, &["suo", "suo"], &["suo", "suo"]).unwrap(),
            text,
            frequency,
        ));
    }
    let lexicon = Lexicon::new(entries);
    for input in ["ss", "s's", "suos", "ssuo"] {
        let candidates =
            Decoder::with_limits(1, 1).frequent_phrases(&parser.parse(input).unwrap(), &lexicon, 2);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            ["琐琐", "缩缩"]
        );
    }
}

#[test]
fn closer_completions_survive_candidate_pruning() {
    let parser = Parser::standard();
    let entry = |spelling, text: String, frequency| {
        Lexeme::new(
            Reading::new(parser.table(), &[spelling], &[spelling]).unwrap(),
            text,
            frequency,
        )
    };
    let mut words = (0..80)
        .map(|index| {
            entry(
                "cheng",
                char::from_u32(0x4e00 + index).unwrap().to_string(),
                10_000,
            )
        })
        .collect::<Vec<_>>();
    words.push(entry("ca", "擦".into(), 1));
    let lexicon = Lexicon::new(words);
    for limit in [1, 9, 72] {
        let candidates = Decoder::default().decode(&parser.parse("c").unwrap(), &lexicon, limit);
        assert_eq!(candidates.len(), limit);
        assert_eq!(candidates[0].text, "擦");
        assert_eq!(candidates[0].spelling_length, 2);
    }
}
