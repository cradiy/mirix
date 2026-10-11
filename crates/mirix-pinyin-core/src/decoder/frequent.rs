//! Whole-phrase recall independent of composition and match-quality cutoffs.
use std::{collections::HashMap, sync::Arc};

use super::{DecodedCandidate, EdgeKind, Lexeme, Lexicon, ParseGraph, SyllableId};

pub(super) fn recall(graph: &ParseGraph, lexicon: &Lexicon, limit: usize) -> Vec<DecodedCandidate> {
    let mut search = Search {
        graph,
        lexicon,
        limit,
        visited: HashMap::new(),
        candidates: Vec::new(),
        cutoff: None,
    };
    search.visit(0, &mut Vec::new(), false, 0, lexicon.entries());
    search.prune();
    search.candidates
}

struct Search<'a> {
    graph: &'a ParseGraph,
    lexicon: &'a Lexicon,
    limit: usize,
    visited: HashMap<(usize, Vec<SyllableId>), (bool, usize)>,
    candidates: Vec<DecodedCandidate>,
    cutoff: Option<i64>,
}

impl Search<'_> {
    fn visit(
        &mut self,
        position: usize,
        path: &mut Vec<SyllableId>,
        partial: bool,
        abbreviated: usize,
        prefix_entries: &[Arc<Lexeme>],
    ) {
        let position = self.graph.skip_boundary(position);
        let key = (position, path.clone());
        if self
            .visited
            .get(&key)
            .is_some_and(|&(was_partial, was_abbreviated)| {
                was_partial <= partial && was_abbreviated <= abbreviated
            })
        {
            return;
        }
        self.visited.insert(key, (partial, abbreviated));
        // A quoted complete syllable constrained to a character is not a word prefix.
        if self.graph.requires_single_character(position) {
            return;
        }
        for edge in self.graph.edges_from(position) {
            let (ids, partial, abbreviated): (&[SyllableId], _, _) = match &edge.kind {
                EdgeKind::Complete(id) => (std::slice::from_ref(id), partial, abbreviated),
                EdgeKind::Incomplete { completions } => (completions, true, abbreviated),
                EdgeKind::Abbreviated { completions } => (completions, partial, abbreviated + 1),
                EdgeKind::Boundary => continue,
            };
            for id in ids {
                let entries = Lexicon::extend_prefix(prefix_entries, path.len(), *id);
                path.push(*id);
                let end = self.graph.skip_boundary(edge.end);
                if end == self.graph.input_len() && path.len() > 1 {
                    for entry in entries
                        .iter()
                        .take_while(|entry| entry.reading.syllables().len() == path.len())
                    {
                        let score = self.lexicon.score(entry);
                        // Exact readings are frequency ordered. Once the best
                        // unique results fill the budget, a strictly lower score
                        // cannot displace them. Equal scores still need all tie
                        // breakers, including alternate readings of the same word.
                        if self.cutoff.is_some_and(|cutoff| score < cutoff) {
                            break;
                        }
                        if entry.text.chars().take(2).count() < 2 {
                            continue;
                        }
                        self.candidates.push(DecodedCandidate {
                            spelling_length: entry.reading.spelling_length(),
                            text: entry.text.clone(),
                            display_reading: entry.reading.display(),
                            score,
                            segments: 1,
                            syllable_count: path.len(),
                            completed_partial_syllable: partial,
                            abbreviated_syllables: abbreviated,
                        });
                        if self.candidates.len() >= self.limit.saturating_mul(4) {
                            self.prune();
                        }
                    }
                } else if end < self.graph.input_len()
                    && entries
                        .last()
                        .is_some_and(|entry| entry.reading.syllables().len() > path.len())
                {
                    self.visit(end, path, partial, abbreviated, entries);
                }
                path.pop();
            }
        }
    }

    fn prune(&mut self) {
        self.candidates.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.compare_match_kind(right))
                .then_with(|| left.text.cmp(&right.text))
                .then_with(|| left.display_reading.cmp(&right.display_reading))
        });
        super::retain_unique(&mut self.candidates, self.limit, |candidate| {
            (&candidate.text, 0)
        });
        self.cutoff = self
            .candidates
            .last()
            .filter(|_| self.candidates.len() == self.limit)
            .map(|candidate| candidate.score);
    }
}
