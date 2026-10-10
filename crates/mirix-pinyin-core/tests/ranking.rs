use mirix_pinyin_core::{Decoder, Lexeme, Lexicon, Parser, Reading};

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
