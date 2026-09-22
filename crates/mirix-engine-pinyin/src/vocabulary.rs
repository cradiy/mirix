//! Explicit personal entries and preferences, independent of selection counts.
use std::collections::BTreeMap;

use mirix_engine_protocol::{VocabularyEntry, VocabularyOperation, VocabularyWord};
use mirix_pinyin_core::{Lexeme, Reading, SyllableTable, normalize_display_syllable};
use serde::{Deserialize, Serialize};

use crate::learning::Learning;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Preference {
    pub word: VocabularyWord,
    pub personal: bool,
    pub pinned: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Vocabulary {
    pub words: BTreeMap<String, Vec<Preference>>,
}

pub(crate) fn reading_key(reading: &str) -> String {
    reading
        .split_whitespace()
        .filter_map(normalize_display_syllable)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn normalize_word(
    word: VocabularyWord,
    table: &SyllableTable,
) -> Result<VocabularyWord, String> {
    if word.text.trim().is_empty() {
        return Err("word text must not be empty".into());
    }
    let display = word
        .reading
        .split(|c: char| c.is_whitespace() || c == '\'')
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let spelling = display
        .iter()
        .map(|part| {
            normalize_display_syllable(part).ok_or_else(|| format!("invalid syllable: {part}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let input = spelling.iter().map(String::as_str).collect::<Vec<_>>();
    let shown = display.iter().map(String::as_str).collect::<Vec<_>>();
    Reading::new(table, &input, &shown).map_err(|error| error.to_string())?;
    Ok(VocabularyWord {
        text: word.text,
        reading: display.join(" "),
    })
}

impl Vocabulary {
    pub fn candidates(
        &self,
        graph: &mirix_pinyin_core::ParseGraph,
        table: &SyllableTable,
        lexicon: &mirix_pinyin_core::Lexicon,
    ) -> Vec<mirix_pinyin_core::DecodedCandidate> {
        self.words
            .values()
            .flatten()
            .filter(|entry| (entry.personal || entry.pinned) && !entry.hidden)
            .filter_map(|entry| {
                crate::learning::recall_word(
                    &entry.word.text,
                    &entry.word.reading,
                    graph,
                    table,
                    lexicon,
                )
            })
            .collect()
    }

    pub fn preference(&self, text: &str, reading: &str) -> Option<&Preference> {
        self.words
            .get(&reading_key(reading))?
            .iter()
            .find(|entry| entry.word.text == text)
    }

    pub fn hidden(&self, text: &str, reading: &str) -> bool {
        self.preference(text, reading)
            .is_some_and(|entry| entry.hidden)
    }

    pub fn hidden_count(&self) -> usize {
        self.words
            .values()
            .flatten()
            .filter(|entry| entry.hidden)
            .count()
    }

    pub fn pinned(&self, text: &str, reading: &str) -> bool {
        self.preference(text, reading)
            .is_some_and(|entry| entry.pinned)
    }

    pub fn edit(&mut self, operation: &VocabularyOperation) -> bool {
        let (word, enabled) = match operation {
            VocabularyOperation::Add { word } => (word, true),
            VocabularyOperation::Remove { word } => {
                let key = reading_key(&word.reading);
                let Some(entries) = self.words.get_mut(&key) else {
                    return false;
                };
                let before = entries.len();
                entries.retain(|entry| entry.word.text != word.text);
                let changed = before != entries.len();
                if entries.is_empty() {
                    self.words.remove(&key);
                }
                return changed;
            }
            VocabularyOperation::Pin { word, enabled }
            | VocabularyOperation::Hide { word, enabled } => (word, *enabled),
            VocabularyOperation::List { .. } | VocabularyOperation::Forget { .. } => return false,
        };
        let key = reading_key(&word.reading);
        let existing = self.preference(&word.text, &word.reading).cloned();
        if existing.is_none() && !enabled {
            return false;
        }
        let mut entry = existing.clone().unwrap_or_else(|| Preference {
            word: word.clone(),
            personal: false,
            pinned: false,
            hidden: false,
        });
        match operation {
            VocabularyOperation::Add { .. } => {
                entry.personal = true;
                entry.word = word.clone();
            }
            VocabularyOperation::Pin { .. } => entry.pinned = enabled,
            VocabularyOperation::Hide { .. } => entry.hidden = enabled,
            _ => unreachable!(),
        }
        if existing.as_ref() == Some(&entry) {
            return false;
        }
        let entries = self.words.entry(key.clone()).or_default();
        entries.retain(|entry| entry.word.text != word.text);
        if entry.personal || entry.pinned || entry.hidden {
            entries.push(entry);
        }
        if entries.is_empty() {
            self.words.remove(&key);
        }
        true
    }

    pub fn lexemes(&self, table: &SyllableTable) -> Vec<Lexeme> {
        self.words
            .values()
            .flatten()
            .filter(|entry| entry.personal || entry.pinned)
            .filter_map(|entry| {
                let key = reading_key(&entry.word.reading);
                let input = key.split_whitespace().collect::<Vec<_>>();
                let display = entry.word.reading.split_whitespace().collect::<Vec<_>>();
                let reading = Reading::new(table, &input, &display).ok()?;
                Some(Lexeme::new(reading, entry.word.text.clone(), 0))
            })
            .collect()
    }

    pub fn list(&self, learning: &Learning, query: &str) -> Vec<VocabularyEntry> {
        let mut entries: BTreeMap<(String, String), VocabularyEntry> = BTreeMap::new();
        for word in learning.words.values().flatten() {
            let key = (word.text.clone(), reading_key(&word.reading));
            let entry = entries.entry(key).or_insert_with(|| VocabularyEntry {
                word: VocabularyWord {
                    text: word.text.clone(),
                    reading: word.reading.clone(),
                },
                count: 0,
                last_used: 0,
                personal: false,
                pinned: false,
                hidden: false,
            });
            for frequency in word.inputs.values() {
                entry.count = entry.count.saturating_add(frequency.count);
                entry.last_used = entry.last_used.max(frequency.last_used);
            }
        }
        for preference in self.words.values().flatten() {
            let entry = entries
                .entry((
                    preference.word.text.clone(),
                    reading_key(&preference.word.reading),
                ))
                .or_insert_with(|| VocabularyEntry {
                    word: preference.word.clone(),
                    count: 0,
                    last_used: 0,
                    personal: false,
                    pinned: false,
                    hidden: false,
                });
            if preference.personal {
                entry.word = preference.word.clone();
            }
            entry.personal = preference.personal;
            entry.pinned = preference.pinned;
            entry.hidden = preference.hidden;
        }
        entries
            .into_values()
            .filter(|entry| {
                entry.word.text.contains(query)
                    || entry.word.reading.contains(query)
                    || reading_key(&entry.word.reading)
                        .replace(' ', "")
                        .contains(query)
            })
            .collect()
    }
}
