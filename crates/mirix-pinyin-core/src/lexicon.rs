use std::cmp::Ordering;
use std::sync::Arc;

use crate::{SyllableId, SyllableTable};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadingError {
    #[error("reading has no syllables")]
    Empty,
    #[error("input and display readings have different syllable counts")]
    SyllableCount,
    #[error("unknown input syllable {0:?}")]
    UnknownSyllable(String),
    #[error("invalid display syllable {0:?}")]
    InvalidDisplay(String),
    #[error("display syllable {display:?} does not match input syllable {input:?}")]
    DisplayMismatch { input: String, display: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    spelling_length: usize,
    syllables: Vec<SyllableId>,
    display_syllables: Vec<String>,
}

impl Reading {
    pub fn new(
        table: &SyllableTable,
        input_syllables: &[&str],
        display_syllables: &[&str],
    ) -> Result<Self, ReadingError> {
        if input_syllables.is_empty() {
            return Err(ReadingError::Empty);
        }
        if input_syllables.len() != display_syllables.len() {
            return Err(ReadingError::SyllableCount);
        }

        let mut syllables = Vec::with_capacity(input_syllables.len());
        let mut display = Vec::with_capacity(display_syllables.len());
        for (&input, &shown) in input_syllables.iter().zip(display_syllables) {
            let Some(id) = table.id(input) else {
                return Err(ReadingError::UnknownSyllable(input.to_owned()));
            };
            let normalized = normalize_display_syllable(shown)
                .ok_or_else(|| ReadingError::InvalidDisplay(shown.to_owned()))?;
            if normalized != input {
                return Err(ReadingError::DisplayMismatch {
                    input: input.to_owned(),
                    display: shown.to_owned(),
                });
            }
            syllables.push(id);
            display.push(shown.to_owned());
        }

        Ok(Self {
            spelling_length: input_syllables.iter().map(|part| part.len()).sum(),
            syllables,
            display_syllables: display,
        })
    }

    pub fn syllables(&self) -> &[SyllableId] {
        &self.syllables
    }

    pub fn spelling_length(&self) -> usize {
        self.spelling_length
    }

    pub fn display_syllables(&self) -> &[String] {
        &self.display_syllables
    }

    pub fn display(&self) -> String {
        self.display_syllables.join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lexeme {
    pub reading: Reading,
    pub text: String,
    pub frequency: u64,
}

impl Lexeme {
    pub fn new(reading: Reading, text: impl Into<String>, frequency: u64) -> Self {
        Self {
            reading,
            text: text.into(),
            frequency,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Lexicon {
    // A clone shares the ordered snapshot. Composed snapshots retain only
    // entry handles, while each snapshot keeps its own normalized weight.
    entries: Arc<Vec<Arc<Lexeme>>>,
    // Only populated first syllables: sparse tables stay small and clones
    // share the index together with the immutable entries.
    first_syllables: Arc<[(SyllableId, usize)]>,
    log_total_weight: f64,
}

impl IntoIterator for Lexicon {
    type Item = Arc<Lexeme>;
    type IntoIter = std::vec::IntoIter<Arc<Lexeme>>;

    fn into_iter(self) -> Self::IntoIter {
        Arc::unwrap_or_clone(self.entries).into_iter()
    }
}

impl FromIterator<Arc<Lexeme>> for Lexicon {
    fn from_iter<T: IntoIterator<Item = Arc<Lexeme>>>(iter: T) -> Self {
        let mut entries: Vec<_> = iter.into_iter().collect();
        // Group identity before frequency; equal entries must be adjacent to deduplicate.
        entries.sort_by(|left, right| {
            left.reading
                .syllables
                .cmp(&right.reading.syllables)
                .then_with(|| left.text.cmp(&right.text))
                .then_with(|| right.frequency.cmp(&left.frequency))
                .then_with(|| {
                    left.reading
                        .display_syllables
                        .cmp(&right.reading.display_syllables)
                })
        });
        entries.dedup_by(|next, previous| {
            next.reading.syllables == previous.reading.syllables && next.text == previous.text
        });
        entries.sort_by(|left, right| compare_lexemes(left, right));
        let mut first_syllables = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let first = entry.reading.syllables[0];
            if first_syllables.last().is_none_or(|&(id, _)| id != first) {
                first_syllables.push((first, index));
            }
        }
        let log_total_weight = entries
            .iter()
            .map(|entry| entry.frequency as f64 + 1.0)
            .sum::<f64>()
            .max(1.0)
            .ln();
        Self {
            entries: Arc::new(entries),
            first_syllables: first_syllables.into(),
            log_total_weight,
        }
    }
}

impl Lexicon {
    pub fn new(entries: Vec<Lexeme>) -> Self {
        entries.into_iter().map(Arc::new).collect()
    }

    /// Smoothed log probability in fixed-point units; additional tokens cannot add positive mass.
    pub(crate) fn score(&self, entry: &Lexeme) -> i64 {
        (((entry.frequency as f64 + 1.0).ln() - self.log_total_weight) * 1024.0).round() as i64
    }

    /// Score an independently recalled word on the same scale as dictionary decoding.
    /// Words absent from the dictionary receive the smoothed zero-frequency prior.
    pub fn word_score(&self, reading: &[SyllableId], text: &str) -> i64 {
        self.exact(reading)
            .iter()
            .filter(|entry| entry.text == text)
            .map(|entry| self.score(entry))
            .max()
            .unwrap_or_else(|| (-self.log_total_weight * 1024.0).round() as i64)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Arc<Lexeme>] {
        &self.entries
    }

    pub fn exact(&self, reading: &[SyllableId]) -> &[Arc<Lexeme>] {
        let Some((&first, tail)) = reading.split_first() else {
            return &[];
        };
        let entries = self.first_syllable(first);
        let start = entries.partition_point(|entry| &entry.reading.syllables[1..] < tail);
        let end =
            start + entries[start..].partition_point(|entry| &entry.reading.syllables[1..] == tail);
        &entries[start..end]
    }

    pub fn with_prefix(&self, prefix: &[SyllableId]) -> &[Arc<Lexeme>] {
        let Some((&first, tail)) = prefix.split_first() else {
            return &self.entries;
        };
        let entries = self.first_syllable(first);
        if tail.is_empty() {
            return entries;
        }
        let start = entries.partition_point(|entry| &entry.reading.syllables[1..] < tail);
        let end = start
            + entries[start..]
                .partition_point(|entry| entry.reading.syllables[1..].starts_with(tail));
        &entries[start..end]
    }

    fn first_syllable(&self, syllable: SyllableId) -> &[Arc<Lexeme>] {
        let Ok(index) = self
            .first_syllables
            .binary_search_by_key(&syllable, |&(id, _)| id)
        else {
            return &[];
        };
        let start = self.first_syllables[index].1;
        let end = self
            .first_syllables
            .get(index + 1)
            .map_or(self.entries.len(), |&(_, start)| start);
        &self.entries[start..end]
    }

    /// Extend a known prefix range by one syllable. Entries must already share
    /// the first `depth` syllables; within that range only the next ID matters.
    pub(crate) fn extend_prefix(
        entries: &[Arc<Lexeme>],
        depth: usize,
        syllable: SyllableId,
    ) -> &[Arc<Lexeme>] {
        let id = Some(&syllable);
        let start = entries.partition_point(|entry| entry.reading.syllables.get(depth) < id);
        let end = start
            + entries[start..].partition_point(|entry| entry.reading.syllables.get(depth) == id);
        &entries[start..end]
    }
}

fn compare_lexemes(left: &Lexeme, right: &Lexeme) -> Ordering {
    left.reading
        .syllables
        .cmp(&right.reading.syllables)
        .then_with(|| right.frequency.cmp(&left.frequency))
        .then_with(|| left.text.cmp(&right.text))
}

/// Convert a complete display syllable to its keyboard spelling. Tone marks are
/// removed and `ü` becomes `v`; incomplete spellings are not accepted here.
pub fn normalize_display_syllable(display: &str) -> Option<String> {
    let mut normalized = String::with_capacity(display.len());
    for character in display.chars() {
        let replacement = match character {
            'a'..='z' => {
                normalized.push(character);
                continue;
            }
            'ā' | 'á' | 'ǎ' | 'à' => 'a',
            'ē' | 'é' | 'ě' | 'è' | 'ê' => 'e',
            'ī' | 'í' | 'ǐ' | 'ì' => 'i',
            'ō' | 'ó' | 'ǒ' | 'ò' => 'o',
            'ū' | 'ú' | 'ǔ' | 'ù' => 'u',
            'ü' | 'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' => 'v',
            'ḿ' => 'm',
            'ń' | 'ň' | 'ǹ' => 'n',
            // Tone marks in decomposed Unicode form.
            '\u{0300}' | '\u{0301}' | '\u{0304}' | '\u{030c}' => continue,
            // Diaeresis in decomposed `u` + U+0308 form.
            '\u{0308}' if normalized.ends_with('u') => {
                normalized.pop();
                'v'
            }
            _ => return None,
        };
        normalized.push(replacement);
    }
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(table: &SyllableTable, input: &[&str], display: &[&str]) -> Reading {
        Reading::new(table, input, display).unwrap()
    }

    #[test]
    fn validates_and_formats_complete_display_reading() {
        let table = SyllableTable::standard();
        let reading = reading(&table, &["shu", "ru", "fa"], &["shū", "rù", "fǎ"]);
        assert_eq!(reading.display(), "shū rù fǎ");
        assert_eq!(normalize_display_syllable("lüè"), Some("lve".into()));
        assert_eq!(
            normalize_display_syllable("lu\u{308}e\u{300}"),
            Some("lve".into())
        );
    }

    #[test]
    fn rejects_a_display_reading_for_a_different_syllable() {
        let table = SyllableTable::standard();
        assert_eq!(
            Reading::new(&table, &["shi"], &["shū"]),
            Err(ReadingError::DisplayMismatch {
                input: "shi".into(),
                display: "shū".into(),
            })
        );
    }

    #[test]
    fn exact_and_prefix_queries_keep_frequency_order() {
        let table = SyllableTable::standard();
        let shu = table.id("shu").unwrap();
        let ru = table.id("ru").unwrap();
        let fa = table.id("fa").unwrap();
        let lexicon = Lexicon::new(vec![
            Lexeme::new(reading(&table, &["shu"], &["shū"]), "书", 10),
            Lexeme::new(reading(&table, &["shu"], &["shù"]), "数", 30),
            Lexeme::new(
                reading(&table, &["shu", "ru", "fa"], &["shū", "rù", "fǎ"]),
                "输入法",
                20,
            ),
        ]);

        assert_eq!(
            lexicon
                .exact(&[shu])
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            ["数", "书"]
        );
        assert_eq!(lexicon.with_prefix(&[shu]).len(), 3);
        assert_eq!(
            lexicon.exact(&[shu, ru, fa])[0].reading.display(),
            "shū rù fǎ"
        );
    }

    #[test]
    fn indexed_queries_match_scanning_for_sparse_and_composed_snapshots() {
        let table = SyllableTable::new(["a", "hao", "ni", "shi", "zuo"]).unwrap();
        let mut queries = vec![Vec::new()];
        let mut entries = Vec::new();
        let ids = table.iter().map(|(id, _)| id).collect::<Vec<_>>();
        for length in 1..=3 {
            for mut code in 0..ids.len().pow(length) {
                let syllables = (0..length)
                    .map(|_| {
                        let id = ids[code % ids.len()];
                        code /= ids.len();
                        id
                    })
                    .collect::<Vec<_>>();
                let spelling = syllables
                    .iter()
                    .map(|&id| table.spelling(id).unwrap())
                    .collect::<Vec<_>>();
                if [ids[1], ids[3]].contains(&syllables[0]) && syllables.last() != Some(&ids[2]) {
                    for frequency in [3, 9, 1] {
                        entries.push(Lexeme::new(
                            reading(&table, &spelling, &spelling),
                            format!("{}-{frequency}", spelling.join("")),
                            frequency,
                        ));
                    }
                }
                queries.push(syllables);
            }
        }
        let lexicon = Lexicon::new(entries);
        let overlay = Lexicon::new(vec![Lexeme::new(
            reading(&table, &["zuo"], &["zuo"]),
            "做",
            50,
        )]);
        let composed: Lexicon = lexicon.clone().into_iter().chain(overlay).collect();
        for snapshot in [Lexicon::default(), lexicon.clone(), composed, lexicon] {
            for query in &queries {
                let exact = snapshot
                    .entries()
                    .iter()
                    .filter(|entry| entry.reading.syllables() == query)
                    .cloned()
                    .collect::<Vec<_>>();
                let prefix = snapshot
                    .entries()
                    .iter()
                    .filter(|entry| entry.reading.syllables().starts_with(query))
                    .cloned()
                    .collect::<Vec<_>>();
                assert_eq!(snapshot.exact(query), exact, "exact {query:?}");
                assert_eq!(snapshot.with_prefix(query), prefix, "prefix {query:?}");
            }
        }
    }

    #[test]
    fn duplicate_entries_keep_the_highest_frequency() {
        let table = SyllableTable::standard();
        let shu = table.id("shu").unwrap();
        let lexicon = Lexicon::new(vec![
            Lexeme::new(reading(&table, &["shu"], &["shū"]), "书", 10),
            Lexeme::new(reading(&table, &["shu"], &["shū"]), "书", 40),
            Lexeme::new(reading(&table, &["shu"], &["shù"]), "树", 20),
        ]);
        assert_eq!(lexicon.len(), 2);
        assert_eq!(lexicon.exact(&[shu])[0].frequency, 40);
        assert_eq!(lexicon.exact(&[shu])[0].text, "书");
        assert!(lexicon.score(&lexicon.exact(&[shu])[0]) < 0);
    }

    #[test]
    fn composed_snapshots_share_entries_but_keep_source_weights_and_lifetimes() {
        let table = SyllableTable::standard();
        let shu = table.id("shu").unwrap();
        let source = Lexicon::new(vec![
            Lexeme::new(reading(&table, &["shu"], &["shū"]), "书", 10),
            Lexeme::new(reading(&table, &["shu"], &["shù"]), "树", 20),
        ]);
        let overlay = Lexicon::new(vec![Lexeme::new(
            reading(&table, &["shu"], &["shū"]),
            "书",
            100,
        )]);
        let snapshot = source.clone();
        let composed: Lexicon = source.clone().into_iter().chain(overlay).collect();
        let tree = &source.exact(&[shu])[0];
        assert!(Arc::ptr_eq(tree, &composed.exact(&[shu])[1]));
        assert_eq!(composed.exact(&[shu])[0].frequency, 100);
        assert_eq!(source.exact(&[shu])[1].frequency, 10);
        assert!(source.score(tree) > composed.score(tree));
        let tree = Arc::downgrade(tree);
        drop(source);
        drop(composed);
        assert_eq!(snapshot.exact(&[shu])[0].text, "树");
        assert!(tree.upgrade().is_some());
        drop(snapshot);
        assert!(tree.upgrade().is_none());
    }
}
