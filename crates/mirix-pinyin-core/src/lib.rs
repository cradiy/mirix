//! Input-method-independent pinyin algorithms.
//!
//! This crate does not depend on Mirix engine traits or its process protocol.

mod bigram;
mod decoder;
mod lexicon;
mod matching;
pub use matching::MatchQuality;
mod parser;
mod source;
mod syllables;

pub use bigram::{BigramError, BigramModel};
pub use decoder::{DecodedCandidate, Decoder, Transition};
pub use lexicon::{Lexeme, Lexicon, Reading, ReadingError, normalize_display_syllable};
pub use parser::{Edge, EdgeKind, ParseError, ParseGraph, Parser};
pub use source::{ImportIssue, ImportResult, import_character_tsv, import_tsv};
pub use syllables::{SyllableId, SyllableTable, TableError};
