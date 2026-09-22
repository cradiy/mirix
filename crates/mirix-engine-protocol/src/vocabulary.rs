//! Engine-owned personal vocabulary management, scoped to the current profile.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyWord {
    pub text: String,
    /// Engine-defined reading or input code. Interpretation belongs to the engine.
    pub reading: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VocabularyOperation {
    List {
        query: String,
        offset: usize,
        limit: usize,
    },
    Add {
        word: VocabularyWord,
    },
    Remove {
        word: VocabularyWord,
    },
    Forget {
        word: VocabularyWord,
    },
    Pin {
        word: VocabularyWord,
        enabled: bool,
    },
    Hide {
        word: VocabularyWord,
        enabled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyEntry {
    pub word: VocabularyWord,
    pub count: u64,
    pub last_used: u64,
    pub personal: bool,
    pub pinned: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VocabularyResult {
    Words {
        profile: String,
        entries: Vec<VocabularyEntry>,
        total: usize,
        offset: usize,
    },
    Updated {
        profile: String,
        changed: bool,
    },
}
