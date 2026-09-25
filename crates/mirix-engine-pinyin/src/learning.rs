//! Per-profile selection statistics and bounded committed-text context.
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use mirix_pinyin_core::{
    DecodedCandidate, EdgeKind, Lexeme, Lexicon, ParseGraph, Reading, SyllableTable,
    normalize_display_syllable,
};
use serde::{Deserialize, Serialize};

mod index;
mod sentence;
use index::RecallIndex;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Frequency {
    pub count: u64,
    pub last_used: u64,
}

impl Frequency {
    fn record(&mut self, now: u64) {
        self.count = self.count.saturating_add(1);
        self.last_used = now;
    }

    fn weight(&self, now: u64) -> u64 {
        // A month of inactivity halves the influence, not the stored count.
        let months = now.saturating_sub(self.last_used) / (30 * 24 * 60 * 60);
        self.count
            .saturating_mul(16)
            .checked_shr(months.min(64) as u32)
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LearnedWord {
    pub text: String,
    pub reading: String,
    pub inputs: BTreeMap<String, Frequency>,
    pub contexts: BTreeMap<String, BTreeMap<String, Frequency>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Learning {
    // Canonical full spelling indexes recall; actual input retains apostrophes.
    pub words: BTreeMap<String, Vec<LearnedWord>>,
    #[serde(skip)]
    index: OnceLock<RecallIndex>,
}

impl Clone for Learning {
    fn clone(&self) -> Self {
        // Persistence snapshots need records, not a duplicate search index.
        Self {
            words: self.words.clone(),
            index: OnceLock::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub input: String,
    pub reading: String,
    pub text: String,
    pub context: String,
    pub time: u64,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn suffix(text: &str, count: usize) -> String {
    let start = text
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map_or(0, |(i, _)| i);
    text[start..].to_owned()
}

fn spelling(reading: &str) -> Option<String> {
    reading
        .split_whitespace()
        .map(normalize_display_syllable)
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.concat())
}

pub(crate) fn recall_word(
    text: &str,
    reading: &str,
    graph: &ParseGraph,
    table: &SyllableTable,
    lexicon: &Lexicon,
) -> Option<DecodedCandidate> {
    let ids = reading
        .split_whitespace()
        .map(|part| table.id(&normalize_display_syllable(part)?))
        .collect::<Option<Vec<_>>>()?;
    let edges = graph.syllable_edges(&ids)?;
    // Personal words obey the same single-character constraints as dictionary words.
    let characters = text.chars().collect::<Vec<_>>();
    if edges.iter().enumerate().any(|(index, edge)| {
        graph.requires_single_character(edge.start)
            && (characters.len() != ids.len()
                || !lexicon.exact(&[ids[index]]).iter().any(|entry| {
                    let mut chars = entry.text.chars();
                    chars.next() == characters.get(index).copied() && chars.next().is_none()
                }))
    }) {
        return None;
    }
    Some(DecodedCandidate {
        spelling_length: ids
            .iter()
            .map(|id| table.spelling(*id).map(str::len))
            .sum::<Option<usize>>()?,
        text: text.to_owned(),
        display_reading: reading.to_owned(),
        score: lexicon.word_score(&ids, text),
        segments: 1,
        syllable_count: ids.len(),
        completed_partial_syllable: edges
            .iter()
            .any(|edge| matches!(edge.kind, EdgeKind::Incomplete { .. })),
        abbreviated_syllables: edges
            .iter()
            .filter(|edge| matches!(edge.kind, EdgeKind::Abbreviated { .. }))
            .count(),
    })
}

impl Learning {
    pub fn prepare_index(&self) {
        self.index.get_or_init(|| RecallIndex::new(&self.words));
    }

    pub fn forget(&mut self, word: &mirix_engine_protocol::VocabularyWord) -> bool {
        let reading = crate::vocabulary::reading_key(&word.reading);
        let key = reading.replace(' ', "");
        let Some(words) = self.words.get_mut(&key) else {
            return false;
        };
        let before = words.len();
        words.retain(|item| {
            item.text != word.text || crate::vocabulary::reading_key(&item.reading) != reading
        });
        let changed = before != words.len();
        if words.is_empty() {
            self.words.remove(&key);
        }
        if changed {
            self.index.take();
        }
        changed
    }

    pub fn single_characters(&self, table: &SyllableTable) -> Vec<Lexeme> {
        self.words
            .values()
            .flatten()
            .filter_map(|word| {
                if word.text.chars().count() != 1 || word.reading.split_whitespace().count() != 1 {
                    return None;
                }
                let spelling = normalize_display_syllable(&word.reading)?;
                let reading = Reading::new(table, &[&spelling], &[&word.reading]).ok()?;
                Some(Lexeme::new(reading, word.text.clone(), 0))
            })
            .collect()
    }

    pub fn record(&mut self, selection: &Selection) {
        let Some(key) = spelling(&selection.reading) else {
            return;
        };
        let words = self.words.entry(key.clone()).or_default();
        let index = words
            .iter()
            .position(|word| word.text == selection.text && word.reading == selection.reading);
        let word = match index {
            Some(index) => &mut words[index],
            None => {
                words.push(LearnedWord {
                    text: selection.text.clone(),
                    reading: selection.reading.clone(),
                    inputs: BTreeMap::new(),
                    contexts: BTreeMap::new(),
                });
                words.last_mut().unwrap()
            }
        };
        word.inputs
            .entry(selection.input.clone())
            .or_default()
            .record(selection.time);
        for length in 1..=selection.context.chars().count().min(4) {
            word.contexts
                .entry(selection.input.clone())
                .or_default()
                .entry(suffix(&selection.context, length))
                .or_default()
                .record(selection.time);
        }
        if index.is_none()
            && let Some(cache) = self.index.get_mut()
        {
            cache.insert(&selection.reading, &selection.text, key, words.len() - 1);
        }
    }

    pub fn candidates(
        &self,
        input: &str,
        graph: &ParseGraph,
        table: &SyllableTable,
        lexicon: &Lexicon,
    ) -> Vec<DecodedCandidate> {
        if input.is_empty() {
            return Vec::new();
        }
        self.index
            .get_or_init(|| RecallIndex::new(&self.words))
            .matches(graph, table)
            .into_iter()
            .filter_map(|(key, offset)| {
                let word = &self.words[key][*offset];
                recall_word(&word.text, &word.reading, graph, table, lexicon)
            })
            .collect()
    }

    pub fn boost(
        &self,
        input: &str,
        candidate: &DecodedCandidate,
        context: &str,
        time: u64,
    ) -> i64 {
        let Some(key) = spelling(&candidate.display_reading) else {
            return 0;
        };
        let Some(words) = self.words.get(&key) else {
            return 0;
        };
        let reading = crate::vocabulary::reading_key(&candidate.display_reading);
        let context_suffixes = (1..=context.chars().count().min(4))
            .map(|length| suffix(context, length))
            .collect::<Vec<_>>();
        words
            .iter()
            // Match normalized syllables, not display accents, while preserving
            // syllable boundaries (xi an and xian are different readings).
            .filter(|word| {
                word.text == candidate.text
                    && crate::vocabulary::reading_key(&word.reading) == reading
            })
            .fold(BoostEvidence::default(), |mut evidence, word| {
                for (keys, frequency) in &word.inputs {
                    let weight = frequency.weight(time);
                    evidence.personal = evidence.personal.saturating_add(weight);
                    if keys == input {
                        evidence.input = evidence.input.saturating_add(weight);
                    }
                }
                for entries in word.contexts.values() {
                    for (suffix, support) in context_suffixes.iter().zip(&mut evidence.context) {
                        if let Some(frequency) = entries.get(suffix) {
                            *support = support.saturating_add(frequency.weight(time));
                        }
                    }
                }
                evidence
            })
            .score()
    }
}

#[derive(Default)]
struct BoostEvidence {
    personal: u64,
    input: u64,
    context: [u64; 4],
}

impl BoostEvidence {
    fn score(self) -> i64 {
        if self.personal == 0 && self.input == 0 && self.context.iter().all(|weight| *weight < 32) {
            return 0;
        }
        // Frequency weights use 16 units per effective selection. Logarithmic,
        // bounded boosts share the dictionary's 1024-unit natural-log scale.
        let log_count = |weight: u64, cap: f64| (weight as f64 / 16.0).min(cap).ln_1p();
        let personal = 1536.0 * log_count(self.personal, 256.0);
        let input = 512.0 * log_count(self.input, 256.0);
        // Suffixes overlap: use the strongest evidence, never sum them. Require
        // two effective observations, including evidence across input spellings.
        let context = self
            .context
            .into_iter()
            .enumerate()
            .filter(|(_, weight)| *weight >= 32)
            .map(|(index, weight)| 2048.0 * (1.0 + index as f64 * 0.25) * log_count(weight, 64.0))
            .fold(0.0, f64::max);
        (personal + input + context).round() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirix_pinyin_core::Parser;

    fn learn(learning: &mut Learning, reading: &str, text: &str) {
        learning.record(&Selection {
            input: spelling(reading).unwrap(),
            reading: reading.into(),
            text: text.into(),
            context: String::new(),
            time: 10,
        });
    }

    #[test]
    fn indexed_recall_matches_exhaustive_graph_validation() {
        let parser = Parser::standard();
        let parts = [
            "shi", "si", "shang", "shan", "an", "chang", "chan", "xi", "xian", "xuan", "nv", "hao",
        ];
        let mut learning = Learning::default();
        for first in parts {
            for second in parts {
                for third in ["jie", "an", "ni"] {
                    let reading = format!("{first} {second} {third}");
                    learn(&mut learning, &reading, &format!("词{reading}"));
                }
            }
        }
        for (reading, text) in [
            ("shì jiè", "世界"),
            ("xī ān", "西安"),
            ("xuān ān", "轩安"),
            ("xiān", "先"),
            ("nǚ hái", "女孩"),
        ] {
            learn(&mut learning, reading, text);
        }
        let lexicon = mirix_pinyin_core::import_character_tsv(
            "轩\txuan\t10\n安\tan\t10\n西\txi\t10\n",
            parser.table(),
        )
        .lexicon;
        for input in [
            "s",
            "sh",
            "sj",
            "ssj",
            "shishij",
            "shishijie",
            "chchj",
            "xian",
            "xi'a",
            "xi'an'",
            "xuan'an'",
            "xuan'an'shijie",
            "n",
            "nvh",
            "nvhai",
        ] {
            let mut graph = parser.parse(input).unwrap();
            graph.constrain_closed_syllables(|id| !lexicon.exact(&[id]).is_empty());
            let expected = learning
                .words
                .values()
                .flatten()
                .filter_map(|word| {
                    recall_word(&word.text, &word.reading, &graph, parser.table(), &lexicon)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                learning.candidates(input, &graph, parser.table(), &lexicon),
                expected,
                "{input}"
            );
        }
        let graph = parser.parse("shishijie").unwrap();
        let matched = learning
            .index
            .get()
            .unwrap()
            .matches(&graph, parser.table());
        assert!(!matched.is_empty());
        assert!(matched.len() < learning.words.len() / 10);
    }

    #[test]
    fn recall_index_tracks_additions_forgetting_and_loaded_records() {
        let parser = Parser::standard();
        let graph = parser.parse("nh").unwrap();
        let lexicon = Lexicon::default();
        let recall = |learning: &Learning| {
            learning
                .candidates("nh", &graph, parser.table(), &lexicon)
                .into_iter()
                .map(|c| c.text)
                .collect::<Vec<_>>()
        };
        let mut learning = Learning::default();
        learn(&mut learning, "nǐ hǎo", "你好");
        assert_eq!(recall(&learning), ["你好"]);
        learn(&mut learning, "ní hào", "泥号");
        learn(&mut learning, "nǐ hǎo", "你好");
        assert_eq!(recall(&learning), ["你好", "泥号"]);
        assert!(learning.forget(&mirix_engine_protocol::VocabularyWord {
            text: "你好".into(),
            reading: "nǐ hǎo".into()
        }));
        assert_eq!(recall(&learning), ["泥号"]);
        let loaded: Learning =
            serde_json::from_str(&serde_json::to_string(&learning).unwrap()).unwrap();
        assert_eq!(recall(&loaded), ["泥号"]);
        assert_eq!(recall(&learning.clone()), ["泥号"]);
        learn(&mut learning, "nǚ hái", "女孩");
        assert_eq!(recall(&learning), ["泥号", "女孩"]);
    }

    #[test]
    fn learned_full_reading_recall_respects_boundaries_and_partial_tail() {
        let parser = Parser::standard();
        let mut learning = Learning::default();
        learning.record(&Selection {
            input: "xi'a".into(),
            reading: "xī ān".into(),
            text: "西安".into(),
            context: String::new(),
            time: 10,
        });
        for input in ["xian", "xi'an", "xi'a"] {
            let candidates = learning.candidates(
                input,
                &parser.parse(input).unwrap(),
                parser.table(),
                &Lexicon::default(),
            );
            assert_eq!(candidates.len(), 1);
            assert_eq!(candidates[0].display_reading, "xī ān");
            assert_eq!(candidates[0].completed_partial_syllable, input == "xi'a");
        }
        assert!(
            learning
                .candidates(
                    "xia'n",
                    &parser.parse("xia'n").unwrap(),
                    parser.table(),
                    &Lexicon::default()
                )
                .is_empty()
        );
        assert_eq!(learning.words["xian"][0].inputs["xi'a"].count, 1);
    }

    #[test]
    fn recent_frequency_decays_and_unicode_suffix_is_character_based() {
        let frequency = Frequency {
            count: 2,
            last_used: 1,
        };
        assert_eq!(frequency.weight(1), 32);
        assert_eq!(frequency.weight(1 + 30 * 24 * 60 * 60), 16);
        assert_eq!(frequency.weight(u64::MAX), 0);
        assert_eq!(suffix("甲乙丙丁戊", 4), "乙丙丁戊");
        assert_eq!(suffix("你好", 4), "你好");
        assert_eq!(suffix("", 4), "");
    }

    #[test]
    fn boosts_decay_and_saturate_without_counting_overlapping_contexts_twice() {
        let mut learning = Learning::default();
        let parser = Parser::standard();
        let graph = parser.parse("sj").unwrap();
        let candidate = recall_word(
            "世界",
            "shì jiè",
            &graph,
            parser.table(),
            &Lexicon::default(),
        )
        .unwrap();
        for input in ["shijie", "sj"] {
            learning.record(&Selection {
                input: input.into(),
                reading: "shì jiè".into(),
                text: "世界".into(),
                context: "你好".into(),
                time: 10,
            });
        }
        let plain = learning.boost("shjie", &candidate, "", 10);
        let associated = learning.boost("shjie", &candidate, "你好", 10);
        assert!(associated > plain);
        assert!(learning.boost("sj", &candidate, "", 10) > plain);
        assert_eq!(learning.boost("sj", &candidate, "你好", u64::MAX), 0);
        let word = &mut learning.words.get_mut("shijie").unwrap()[0];
        for entries in word.contexts.values_mut() {
            entries.remove("好");
        }
        assert_eq!(learning.boost("shjie", &candidate, "你好", 10), associated);
        assert_eq!(
            BoostEvidence {
                personal: 256 * 16,
                input: 256 * 16,
                context: [64 * 16; 4]
            }
            .score(),
            BoostEvidence {
                personal: u64::MAX,
                input: u64::MAX,
                context: [u64::MAX; 4]
            }
            .score()
        );
    }
}
