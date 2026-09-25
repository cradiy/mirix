//! Query-local transition scores shared by repeated beam expansions.
use std::cell::RefCell;
use std::collections::HashMap;

use mirix_pinyin_core::{Lexeme, SyllableId, normalize_display_syllable};

use super::{BoostEvidence, Learning, RecallIndex};

type ContextScores = HashMap<String, i64>;
type ReadingScores = HashMap<Vec<SyllableId>, ContextScores>;

impl Learning {
    /// Reuse submitted-context evidence inside one decode. The cache stores only
    /// encountered learned words and is dropped with the scorer, so selections,
    /// profile changes and time decay cannot leave stale transition scores.
    pub fn sentence_scorer(&self, time: u64) -> impl Fn(&str, &Lexeme) -> i64 + '_ {
        let cache = RefCell::new(HashMap::<String, ReadingScores>::new());
        move |prefix, next| {
            if prefix.is_empty() || self.words.is_empty() {
                return 0;
            }
            let index = self.index.get_or_init(|| RecallIndex::new(&self.words));
            let matches = index.matching_text(&next.text);
            if matches.is_empty() {
                return 0;
            }
            let mut cache = cache.borrow_mut();
            if let Some(scores) = cache
                .get(&next.text)
                .and_then(|readings| readings.get(next.reading.syllables()))
            {
                return context_score(prefix, scores);
            }

            let reading = next
                .reading
                .display_syllables()
                .iter()
                .map(|part| normalize_display_syllable(part))
                .collect::<Vec<_>>();
            let mut weights = HashMap::<&str, u64>::new();
            for (key, offset) in matches {
                let word = &self.words[key][*offset];
                if word.contexts.is_empty()
                    || !word
                        .reading
                        .split_whitespace()
                        .map(normalize_display_syllable)
                        .eq(reading.iter().cloned())
                {
                    continue;
                }
                for entries in word.contexts.values() {
                    for (context, frequency) in entries {
                        let weight = weights.entry(context.as_str()).or_default();
                        *weight = weight.saturating_add(frequency.weight(time));
                    }
                }
            }
            let scores = weights
                .into_iter()
                .filter_map(|(context, weight)| {
                    let length = context.chars().take(5).count();
                    if !(1..=4).contains(&length) || weight < 32 {
                        return None;
                    }
                    let mut evidence = BoostEvidence::default();
                    evidence.context[length - 1] = weight;
                    Some((context.to_owned(), evidence.score()))
                })
                .collect::<ContextScores>();
            let score = context_score(prefix, &scores);
            cache
                .entry(next.text.clone())
                .or_default()
                .insert(next.reading.syllables().to_vec(), scores);
            score
        }
    }
}

fn context_score(prefix: &str, scores: &ContextScores) -> i64 {
    prefix
        .char_indices()
        .rev()
        .take(4)
        .filter_map(|(start, _)| scores.get(&prefix[start..]))
        .copied()
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::Selection;
    use mirix_pinyin_core::{Reading, SyllableTable};

    // Independent evaluation of the scoring rule, without the lookup index or
    // compiled score tables: sum support, then take the strongest suffix.
    fn direct_score(learning: &Learning, prefix: &str, next: &Lexeme, time: u64) -> i64 {
        let mut evidence = BoostEvidence::default();
        for word in learning.words.values().flatten().filter(|word| {
            word.text == next.text
                && word
                    .reading
                    .split_whitespace()
                    .map(normalize_display_syllable)
                    .eq(next
                        .reading
                        .display_syllables()
                        .iter()
                        .map(|part| normalize_display_syllable(part)))
        }) {
            for entries in word.contexts.values() {
                for ((start, _), support) in prefix
                    .char_indices()
                    .rev()
                    .take(4)
                    .zip(&mut evidence.context)
                {
                    if let Some(frequency) = entries.get(&prefix[start..]) {
                        *support = support.saturating_add(frequency.weight(time));
                    }
                }
            }
        }
        evidence.score()
    }

    #[test]
    fn compiled_scores_match_context_evidence_across_readings_and_decay() {
        let table = SyllableTable::standard();
        let mut learning = Learning::default();
        for (text, reading, context) in [
            ("西安", "xī ān", "去过"),
            ("西安", "xi an", "过"),
            ("西安", "xiān", "另一个"),
            ("女孩", "nǚ hái", "一位"),
            ("女孩", "nu\u{308} hai", "位"),
        ] {
            for input in ["full", "abbr"] {
                learning.record(&Selection {
                    text: text.into(),
                    reading: reading.into(),
                    input: input.into(),
                    context: context.into(),
                    time: 10,
                });
            }
        }
        let next = [
            ("西安", vec!["xi", "an"]),
            ("西安", vec!["xian"]),
            ("女孩", vec!["nv", "hai"]),
            ("不存在", vec!["ni"]),
        ]
        .map(|(text, parts)| Lexeme::new(Reading::new(&table, &parts, &parts).unwrap(), text, 10));
        for time in [10, 10 + 30 * 24 * 3600, u64::MAX] {
            let score = learning.sentence_scorer(time);
            // Visit each cached reading with different prefixes; do not cache
            // a single prefix's result as the entire word's transition score.
            for _ in 0..2 {
                for prefix in ["", "去过", "我们去过", "另一个", "一位", "那是一位", "无关"]
                {
                    for word in &next {
                        assert_eq!(
                            score(prefix, word),
                            direct_score(&learning, prefix, word, time)
                        );
                    }
                }
            }
        }
        let score = learning.sentence_scorer(10);
        assert!(score("去过", &next[0]) > 0);
        assert_eq!(score("去过", &next[1]), 0);
        assert!(score("一位", &next[2]) > 0);
    }
}
