//! Bounded LRU storage for dictionary and learned sentence-transition decoding.
use std::{collections::VecDeque, mem::size_of};

use mirix_pinyin_core::DecodedCandidate;

const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 512 * 1024;

struct Entry {
    input: String,
    limit: usize,
    candidates: Vec<DecodedCandidate>,
    bytes: usize,
}

#[derive(Default)]
pub(super) struct DecodeCache {
    entries: VecDeque<Entry>,
    bytes: usize,
    epoch: u64,
}

impl DecodeCache {
    /// Scores use hourly time buckets so cached decay is deterministic.
    pub fn set_epoch(&mut self, epoch: u64) {
        if self.epoch != epoch {
            self.clear();
            self.epoch = epoch;
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn get(&mut self, input: &str, limit: usize) -> Option<Vec<DecodedCandidate>> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.input == input && entry.limit == limit)?;
        let entry = self.entries.remove(index)?;
        let candidates = entry.candidates.clone();
        self.entries.push_back(entry);
        Some(candidates)
    }

    pub fn insert(&mut self, input: &str, limit: usize, candidates: &[DecodedCandidate]) {
        // Avoid cloning oversized user dictionary outputs just to discard them.
        let estimate = size_of::<Entry>()
            + input.len()
            + std::mem::size_of_val(candidates)
            + candidates
                .iter()
                .map(|item| item.text.len() + item.display_reading.len())
                .sum::<usize>();
        if estimate > MAX_BYTES {
            return;
        }
        let input = input.to_owned();
        let candidates = candidates.to_vec();
        let bytes = size_of::<Entry>()
            + input.capacity()
            + candidates.capacity() * size_of::<DecodedCandidate>()
            + candidates
                .iter()
                .map(|item| item.text.capacity() + item.display_reading.capacity())
                .sum::<usize>();
        if bytes > MAX_BYTES {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            let Some(entry) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= entry.bytes;
        }
        self.entries.push_back(Entry {
            input,
            limit,
            candidates,
            bytes,
        });
        self.bytes += bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(text: String) -> DecodedCandidate {
        DecodedCandidate {
            text,
            display_reading: "nǐ".into(),
            spelling_length: 2,
            score: 0,
            segments: 1,
            syllable_count: 1,
            completed_partial_syllable: false,
            abbreviated_syllables: 0,
        }
    }

    #[test]
    fn recent_queries_survive_eviction_and_large_results_are_not_retained() {
        let mut cache = DecodeCache::default();
        for index in 0..MAX_ENTRIES {
            cache.insert(&index.to_string(), 72, &[candidate("你".into())]);
        }
        assert!(cache.get("0", 72).is_some());
        assert!(cache.get("0", 73).is_none());
        cache.insert("next", 72, &[candidate("你好".into())]);
        assert!(cache.get("1", 72).is_none());
        assert!(cache.get("0", 72).is_some());
        cache.insert("huge", 72, &[candidate("x".repeat(MAX_BYTES))]);
        assert!(cache.get("huge", 72).is_none());
        for index in 0..100 {
            cache.insert(
                &format!("long{index}"),
                72,
                &[candidate("x".repeat(MAX_BYTES / 8))],
            );
            assert!(cache.bytes <= MAX_BYTES);
            assert!(cache.entries.len() <= MAX_ENTRIES);
        }
        assert!(cache.get("0", 72).is_none());
        cache.set_epoch(1);
        cache.insert("sentence", 72, &[candidate("你好".into())]);
        cache.set_epoch(1);
        assert!(cache.get("sentence", 72).is_some());
        cache.set_epoch(2);
        assert!(cache.get("sentence", 72).is_none());
        cache.clear();
        assert!(cache.get("long99", 72).is_none());
    }
}
