use mirix_pinyin_core::{Decoder, Lexeme, Lexicon, Parser, Reading};

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
