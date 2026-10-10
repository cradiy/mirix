use mirix_pinyin_core::{BigramModel, Decoder, Lexeme, Lexicon, Parser, Reading, Transition};

#[test]
fn word_pairs_override_unigrams_before_full_partial_and_initial_beam_pruning() {
    let parser = Parser::standard();
    let lexicon = Lexicon::new(
        [("wo", "我", 100), ("xie", "些", 100), ("xie", "写", 1)]
            .map(|(reading, text, frequency)| {
                Lexeme::new(
                    Reading::new(parser.table(), &[reading], &[reading]).unwrap(),
                    text,
                    frequency,
                )
            })
            .to_vec(),
    );
    let model = BigramModel::read("我\t写\t1000\n".as_bytes()).unwrap();
    let empty = BigramModel::default();
    let decoder = Decoder::with_limits(256, 1);
    for input in ["woxie", "woxi", "wxie", "wx"] {
        let graph = parser.parse(input).unwrap();
        let decode = |model: &BigramModel| {
            decoder.decode_with_scorer(&graph, &lexicon, 1, &|context, next| {
                model.adjustment(context.previous_word, &next.text, context.unigram_score)
            })
        };
        assert_eq!(decode(&empty), decoder.decode(&graph, &lexicon, 1));
        assert_eq!(decode(&empty)[0].text, "我些", "{input}");
        assert_eq!(decode(&model)[0].text, "我写", "{input}");
    }
}

#[test]
fn identical_prefixes_keep_distinct_last_word_boundaries() {
    let parser = Parser::standard();
    let lexicon = Lexicon::new(
        [
            (vec!["yan", "jiu", "sheng"], "研究生", 1000),
            (vec!["yan", "jiu"], "研究", 100),
            (vec!["sheng"], "生", 1),
            (vec!["huo"], "活", 100),
        ]
        .into_iter()
        .map(|(reading, text, frequency)| {
            Lexeme::new(
                Reading::new(parser.table(), &reading, &reading).unwrap(),
                text,
                frequency,
            )
        })
        .collect(),
    );
    // Both paths produce 研究生. Only 研究 + 生 can earn the 生 -> 活 score;
    // treating a suffix as a word would incorrectly reward 研究生 + 活 too.
    let score = |context: Transition<'_>, next: &Lexeme| {
        if context.previous_word == "生" && next.text == "活" {
            100_000
        } else {
            0
        }
    };
    for input in ["yanjiushenghuo", "yanjiushenghu", "yjshuo"] {
        let candidates = Decoder::default().decode_with_scorer(
            &parser.parse(input).unwrap(),
            &lexicon,
            8,
            &score,
        );
        let result = candidates
            .iter()
            .find(|candidate| candidate.text == "研究生活")
            .unwrap();
        assert_eq!(result.segments, 3, "{input}: {candidates:?}");
        assert!(result.score > 0);
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.text == "研究生活")
                .count(),
            1
        );
    }
}
