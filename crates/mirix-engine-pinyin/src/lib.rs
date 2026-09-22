//! Pinyin input method backed by the Mirix parser, lexicon, and decoder.

mod characters;
mod configuration;
mod engine;
mod extensions;
mod icon;
mod keys;
mod learning;
mod profiles;
mod punctuation;
mod vocabulary;

pub use engine::{DictionaryError, InputProfiles, PinyinEngine};
pub use profiles::ProfileError;
