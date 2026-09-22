//! Input-independent match priorities used before pruning and by mixed-source ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Exact,
    Prefix,
    Abbreviation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MatchQuality {
    kind: Kind,
    composed: bool,
    // For one input, ordering by full spelling length is equivalent to ordering
    // by missing letters. Exact matches always have distance zero.
    spelling_length: usize,
}

impl MatchQuality {
    pub fn extension(input: &str, key: &str) -> Self {
        Self::pinyin(input != key, 0, key.len(), 1)
    }

    pub(crate) fn pinyin(
        partial: bool,
        abbreviated: usize,
        spelling_length: usize,
        segments: usize,
    ) -> Self {
        let kind = if abbreviated > 0 {
            Kind::Abbreviation
        } else if partial {
            Kind::Prefix
        } else {
            Kind::Exact
        };
        Self {
            kind,
            composed: segments > 1,
            spelling_length: if kind == Kind::Exact {
                0
            } else {
                spelling_length
            },
        }
    }
}
