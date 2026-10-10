//! Recall whole dictionary entries by intersecting their prefixes with the input graph.
//! This search does not enumerate unrelated syllable segmentations or compose words.
use crate::{EdgeKind, Lexeme, Lexicon, ParseGraph, SyllableId};
use std::sync::Arc;

use super::{DecodedCandidate, prune_candidates};

pub(super) fn recall(
    graph: &ParseGraph,
    lexicon: &Lexicon,
    limit: usize,
    partial: bool,
) -> Vec<DecodedCandidate> {
    let mut search = Search {
        graph,
        lexicon,
        limit,
        partial,
        candidates: Vec::new(),
    };
    search.visit(0, &mut Vec::new(), false, lexicon.entries());
    prune_candidates(&mut search.candidates, limit);
    search.candidates
}

struct Search<'a> {
    graph: &'a ParseGraph,
    lexicon: &'a Lexicon,
    limit: usize,
    partial: bool,
    candidates: Vec<DecodedCandidate>,
}

impl Search<'_> {
    fn visit(
        &mut self,
        position: usize,
        path: &mut Vec<SyllableId>,
        partial: bool,
        prefix_entries: &[Arc<Lexeme>],
    ) {
        let position = self.graph.skip_boundary(position);
        let single = self.graph.requires_single_character(position);
        if single && !path.is_empty() {
            return;
        }
        for edge in self.graph.edges_from(position) {
            let (ids, partial): (&[SyllableId], bool) = match &edge.kind {
                EdgeKind::Complete(id) => (std::slice::from_ref(id), partial),
                EdgeKind::Incomplete { completions } if self.partial => (completions, true),
                _ => continue,
            };
            for id in ids {
                let entries = Lexicon::extend_prefix(prefix_entries, path.len(), *id);
                path.push(*id);
                let end = self.graph.skip_boundary(edge.end);
                if end == self.graph.input_len() && partial == self.partial {
                    for entry in entries
                        .iter()
                        .take_while(|entry| entry.reading.syllables().len() == path.len())
                    {
                        if single && entry.text.chars().count() != 1 {
                            continue;
                        }
                        self.candidates.push(DecodedCandidate {
                            spelling_length: entry.reading.spelling_length(),
                            text: entry.text.clone(),
                            display_reading: entry.reading.display(),
                            score: self.lexicon.score(entry),
                            segments: 1,
                            syllable_count: path.len(),
                            completed_partial_syllable: partial,
                            abbreviated_syllables: 0,
                        });
                        if self.candidates.len() >= self.limit.saturating_mul(4) {
                            prune_candidates(&mut self.candidates, self.limit);
                        }
                    }
                } else if !single
                    && end < self.graph.input_len()
                    && entries
                        .last()
                        .is_some_and(|entry| entry.reading.syllables().len() > path.len())
                {
                    self.visit(end, path, partial, entries);
                }
                path.pop();
            }
        }
    }
}
