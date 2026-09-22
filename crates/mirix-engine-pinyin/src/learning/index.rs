//! Intersect learned reading prefixes with the parser graph before recalling words.
use std::collections::{BTreeMap, HashMap, HashSet};

use mirix_pinyin_core::{EdgeKind, ParseGraph, SyllableTable, normalize_display_syllable};

use super::LearnedWord;

#[derive(Debug, Default)]
struct Node {
    children: BTreeMap<String, usize>,
    words: Vec<(String, usize)>,
}

#[derive(Debug)]
pub(super) struct RecallIndex {
    nodes: Vec<Node>,
    by_text: HashMap<String, Vec<(String, usize)>>,
}

impl RecallIndex {
    pub fn new(words: &BTreeMap<String, Vec<LearnedWord>>) -> Self {
        let mut index = Self {
            nodes: vec![Node::default()],
            by_text: HashMap::new(),
        };
        for (key, entries) in words {
            for (offset, word) in entries.iter().enumerate() {
                index.insert(&word.reading, &word.text, key.clone(), offset);
            }
        }
        index
    }

    pub fn insert(&mut self, reading: &str, text: &str, key: String, offset: usize) {
        let Some(parts) = reading
            .split_whitespace()
            .map(normalize_display_syllable)
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        if parts.is_empty() {
            return;
        }
        let mut node = 0;
        for part in parts {
            node = if let Some(child) = self.nodes[node].children.get(&part) {
                *child
            } else {
                let child = self.nodes.len();
                self.nodes.push(Node::default());
                self.nodes[node].children.insert(part, child);
                child
            };
        }
        self.by_text
            .entry(text.to_owned())
            .or_default()
            .push((key.clone(), offset));
        self.nodes[node].words.push((key, offset));
    }

    pub fn matching_text(&self, text: &str) -> &[(String, usize)] {
        self.by_text.get(text).map_or(&[], Vec::as_slice)
    }

    pub fn matches(&self, graph: &ParseGraph, table: &SyllableTable) -> Vec<&(String, usize)> {
        let mut pending = vec![(0, 0)];
        let mut visited = HashSet::new();
        let mut words = Vec::new();
        while let Some((position, node)) = pending.pop() {
            // s/sh and c/ch can reach the same trie node at the same offset.
            if !visited.insert((position, node)) {
                continue;
            }
            if position == graph.input_len() {
                words.extend(&self.nodes[node].words);
                continue;
            }
            for edge in graph.edges_from(position) {
                let ids = match &edge.kind {
                    EdgeKind::Boundary => {
                        pending.push((edge.end, node));
                        continue;
                    }
                    EdgeKind::Complete(id) => std::slice::from_ref(id),
                    EdgeKind::Incomplete { completions }
                    | EdgeKind::Abbreviated { completions } => completions.as_slice(),
                };
                for id in ids {
                    if let Some(child) = table
                        .spelling(*id)
                        .and_then(|spelling| self.nodes[node].children.get(spelling))
                    {
                        pending.push((edge.end, *child));
                    }
                }
            }
        }
        // Preserve deterministic recall order; ranking and closed-syllable
        // character constraints remain the responsibility of the existing decoder.
        words.sort_unstable();
        words
    }
}
