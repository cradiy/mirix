//! Intersect the input graph with actual dictionary prefixes before composing.
use super::{
    CandidateRank, DecodedCandidate, Decoder, EdgeKind, Lexeme, Lexicon, ParseGraph, SyllableId,
    match_rank, prune_candidates,
};
use crate::MatchQuality;
use std::cmp::Reverse;

struct WordMatch<'a> {
    end: usize,
    entry: &'a Lexeme,
    abbreviated: usize,
    partial: bool,
}

struct Search<'a> {
    graph: &'a ParseGraph,
    lexicon: &'a Lexicon,
    remaining: usize,
    words: Vec<WordMatch<'a>>,
}

impl<'a> Search<'a> {
    fn visit(
        &mut self,
        position: usize,
        path: &mut Vec<SyllableId>,
        abbreviated: usize,
        partial: bool,
    ) {
        if self.remaining == 0 {
            return;
        }
        let position = self.graph.skip_boundary(position);
        let single = self.graph.requires_single_character(position);
        if single && !path.is_empty() {
            return;
        }
        for edge in self.graph.edges_from(position).iter().rev() {
            let (ids, abbreviated, partial): (&[SyllableId], _, _) = match &edge.kind {
                EdgeKind::Complete(id) => (std::slice::from_ref(id), abbreviated, partial),
                EdgeKind::Abbreviated { completions } => (completions, abbreviated + 1, partial),
                EdgeKind::Incomplete { completions } => (completions, abbreviated, true),
                EdgeKind::Boundary => continue,
            };
            for id in ids {
                if self.remaining == 0 {
                    return;
                }
                self.remaining -= 1;
                path.push(*id);
                let entries = self.lexicon.with_prefix(path);
                let end = self.graph.skip_boundary(edge.end);
                for entry in entries
                    .iter()
                    .take_while(|entry| entry.reading.syllables().len() == path.len())
                {
                    if !single || entry.text.chars().count() == 1 {
                        self.words.push(WordMatch {
                            end,
                            entry,
                            abbreviated,
                            partial,
                        });
                    }
                }
                if !single
                    && entries
                        .last()
                        .is_some_and(|entry| entry.reading.syllables().len() > path.len())
                {
                    self.visit(end, path, abbreviated, partial);
                }
                path.pop();
            }
        }
    }
}

impl Decoder {
    pub(super) fn decode_abbreviated(
        &self,
        graph: &ParseGraph,
        lexicon: &Lexicon,
        limit: usize,
        scorer: &impl Fn(&str, &Lexeme) -> i64,
    ) -> Vec<DecodedCandidate> {
        let mut beams = vec![Vec::new(); graph.input_len() + 1];
        beams[0].push(DecodedCandidate {
            spelling_length: 0,
            text: String::new(),
            display_reading: String::new(),
            score: 0,
            segments: 0,
            syllable_count: 0,
            completed_partial_syllable: false,
            abbreviated_syllables: 0,
        });
        let width = self.beam_width.max(limit);
        let mut cutoffs: Vec<Option<CandidateRank>> = vec![None; beams.len()];
        let mut search = Search {
            graph,
            lexicon,
            remaining: self.max_paths.saturating_mul(64),
            words: Vec::new(),
        };
        for start in 0..graph.input_len() {
            if beams[start].is_empty() {
                continue;
            }
            prune_candidates(&mut beams[start], width);
            let states = std::mem::take(&mut beams[start]);
            search.visit(start, &mut Vec::new(), 0, false);
            for word in search.words.drain(..) {
                let display = word.entry.reading.display();
                let score = lexicon.score(word.entry);
                let characters = word.entry.text.chars().take(2).count();
                for state in &states {
                    let score = score.saturating_add(scorer(&state.text, word.entry));
                    let next_score = state.score.saturating_add(score);
                    let segments = state.segments + 1;
                    let syllables = state.syllable_count + word.entry.reading.syllables().len();
                    let partial = state.completed_partial_syllable || word.partial;
                    let abbreviated = state.abbreviated_syllables + word.abbreviated;
                    let spelling_length =
                        state.spelling_length + word.entry.reading.spelling_length();
                    let rank = (
                        MatchQuality::pinyin(partial, abbreviated, spelling_length, segments),
                        match_rank(
                            syllables,
                            characters + state.text.chars().take(2).count(),
                            segments,
                            partial,
                            abbreviated,
                        ),
                        Reverse(next_score),
                        segments,
                    );
                    // The cutoff exists only after retaining `width` unique
                    // candidates. Equal ranks must still reach lexical sorting.
                    if cutoffs[word.end].is_some_and(|cutoff| rank > cutoff) {
                        continue;
                    }
                    let mut text = state.text.clone();
                    text.push_str(&word.entry.text);
                    let mut reading = state.display_reading.clone();
                    if !reading.is_empty() {
                        reading.push(' ');
                    }
                    reading.push_str(&display);
                    beams[word.end].push(DecodedCandidate {
                        spelling_length,
                        text,
                        display_reading: reading,
                        score: next_score,
                        segments,
                        syllable_count: syllables,
                        completed_partial_syllable: partial,
                        abbreviated_syllables: abbreviated,
                    });
                }
                if beams[word.end].len() >= width.saturating_mul(2) {
                    prune_candidates(&mut beams[word.end], width);
                    cutoffs[word.end] = beams[word.end]
                        .last()
                        .filter(|_| beams[word.end].len() == width)
                        .map(DecodedCandidate::rank);
                }
            }
        }
        let mut candidates = std::mem::take(&mut beams[graph.input_len()]);
        candidates.retain(|candidate| candidate.abbreviated_syllables > 0);
        prune_candidates(&mut candidates, limit);
        candidates
    }
}
