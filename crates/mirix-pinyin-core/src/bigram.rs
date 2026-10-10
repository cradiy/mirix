//! Sparse word transitions with a unigram prior for unobserved pairs.
use std::collections::HashMap;
use std::io::BufRead;

const PRIOR_COUNT: f64 = 32.0;
const SCORE_SCALE: f64 = 1024.0;

#[derive(Default)]
pub struct BigramModel {
    rows: HashMap<String, Row>,
}

struct Row {
    log_total: f64,
    log_counts: HashMap<String, f64>,
}

#[derive(Debug, thiserror::Error)]
pub enum BigramError {
    #[error("line {line}: {source}")]
    Io { line: usize, source: std::io::Error },
    #[error("line {line}: expected previous word, next word and positive count separated by tabs")]
    InvalidRow { line: usize },
    #[error("line {line}: transition count overflow")]
    Overflow { line: usize },
}

impl BigramModel {
    /// Read `previous<TAB>next<TAB>count`. Blank lines and # comments are ignored;
    /// repeated pairs are accumulated. No corpus or segmentation is inferred.
    pub fn read(reader: impl BufRead) -> Result<Self, BigramError> {
        let mut rows = HashMap::<String, (u64, HashMap<String, u64>)>::new();
        for (index, line) in reader.lines().enumerate() {
            let number = index + 1;
            let line = line.map_err(|source| BigramError::Io {
                line: number,
                source,
            })?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let previous = fields.next().unwrap();
            let next = fields.next().unwrap_or_default();
            let count = fields.next().and_then(|count| count.parse::<u64>().ok());
            let word = |text: &str| !text.is_empty() && !text.chars().any(char::is_whitespace);
            if !word(previous)
                || !word(next)
                || count.is_none_or(|count| count == 0)
                || fields.next().is_some()
            {
                return Err(BigramError::InvalidRow { line: number });
            }
            let count = count.unwrap();
            let (total, counts) = rows.entry(previous.to_owned()).or_default();
            *total = total
                .checked_add(count)
                .ok_or(BigramError::Overflow { line: number })?;
            // The checked row total also bounds every pair count.
            *counts.entry(next.to_owned()).or_default() += count;
        }
        Ok(Self {
            rows: rows
                .into_iter()
                .map(|(previous, (total, counts))| {
                    (
                        previous,
                        Row {
                            log_total: (total as f64 + PRIOR_COUNT).ln(),
                            log_counts: counts
                                .into_iter()
                                .map(|(next, count)| (next, (count as f64).ln()))
                                .collect(),
                        },
                    )
                })
                .collect(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Replace the next word's unigram log probability with the smoothed
    /// conditional probability: (pair_count + 32 * P(next)) / (row_count + 32).
    /// Missing contexts preserve the unigram score exactly; unseen pairs remain
    /// possible. The return value is an adjustment, not a complete path score.
    pub fn adjustment(&self, previous: &str, next: &str, unigram_score: i64) -> i64 {
        let Some(row) = self.rows.get(previous) else {
            return 0;
        };
        let prior = PRIOR_COUNT.ln() + unigram_score as f64 / SCORE_SCALE;
        let numerator = match row.log_counts.get(next) {
            Some(&count) => {
                let high = count.max(prior);
                high + (-(count - prior).abs()).exp().ln_1p()
            }
            None => prior,
        };
        let conditional = ((numerator - row.log_total) * SCORE_SCALE).round() as i64;
        conditional.saturating_sub(unigram_score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothed_scores_preserve_backoff_and_combine_duplicate_evidence() {
        let model = BigramModel::read("喝\t水\t9\n喝\t茶\t1\n喝\t水\t10\n".as_bytes()).unwrap();
        let base = (0.1_f64.ln() * SCORE_SCALE).round() as i64;
        let probability = (base as f64 / SCORE_SCALE).exp();
        for (next, count) in [("水", 19.0), ("茶", 1.0), ("石头", 0.0)] {
            let expected = (((count + PRIOR_COUNT * probability) / (20.0 + PRIOR_COUNT)).ln()
                * SCORE_SCALE)
                .round() as i64;
            assert_eq!(base + model.adjustment("喝", next, base), expected);
        }
        assert_eq!(model.adjustment("未知", "水", base), 0);
        assert_eq!(BigramModel::default().adjustment("喝", "水", base), 0);
    }

    #[test]
    fn counts_are_checked_and_large_valid_counts_have_finite_scores() {
        let huge = format!("喝\t水\t{}\n", u64::MAX);
        let model = BigramModel::read(huge.as_bytes()).unwrap();
        for next in ["水", "茶"] {
            let score = -10000 + model.adjustment("喝", next, -10000);
            assert!((-100000..=0).contains(&score));
        }
        let overflow = format!("{huge}喝\t茶\t1\n");
        assert!(matches!(
            BigramModel::read(overflow.as_bytes()),
            Err(BigramError::Overflow { line: 2 })
        ));
        for row in ["喝\t水\t0", "喝\t水\t-1", "喝\t水\t1\textra", "喝\t\t1"] {
            assert!(matches!(
                BigramModel::read(row.as_bytes()),
                Err(BigramError::InvalidRow { line: 1 })
            ));
        }
    }
}
