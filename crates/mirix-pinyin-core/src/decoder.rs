use std::cmp::{Ordering, Reverse};
use std::collections::HashSet;

use crate::{EdgeKind, Lexeme, Lexicon, MatchQuality, ParseGraph, SyllableId};

mod abbreviation;
mod frequent;
mod whole_words;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCandidate {
    pub spelling_length: usize,
    pub text: String,
    pub display_reading: String,
    pub score: i64,
    pub segments: usize,
    pub syllable_count: usize,
    pub completed_partial_syllable: bool,
    pub abbreviated_syllables: usize,
}

impl DecodedCandidate {
    pub fn match_quality(&self) -> MatchQuality {
        MatchQuality::pinyin(
            self.completed_partial_syllable,
            self.abbreviated_syllables,
            self.spelling_length,
            self.segments,
        )
    }
    /// Structural priorities shared by dictionary decoding and personal recall.
    pub fn compare_match_kind(&self, other: &Self) -> Ordering {
        self.match_quality()
            .cmp(&other.match_quality())
            .then_with(|| self.match_rank().cmp(&other.match_rank()))
    }

    fn match_rank(&self) -> (bool, bool, bool, usize) {
        match_rank(
            self.syllable_count,
            self.text.chars().take(2).count(),
            self.segments,
            self.completed_partial_syllable,
            self.abbreviated_syllables,
        )
    }

    fn rank(&self) -> CandidateRank {
        (
            self.match_quality(),
            self.match_rank(),
            Reverse(self.score),
            self.segments,
        )
    }
}

// Text is deliberately excluded: equal ranks still need full lexical comparison.
type CandidateRank = (MatchQuality, (bool, bool, bool, usize), Reverse<i64>, usize);

fn match_rank(
    syllables: usize,
    characters: usize,
    segments: usize,
    partial: bool,
    abbreviated: usize,
) -> (bool, bool, bool, usize) {
    (
        partial || abbreviated > 0,
        !(syllables == 1 && characters == 1),
        !(segments == 1 && syllables > 1 && characters > 1),
        abbreviated,
    )
}

#[derive(Debug, Clone, Copy)]
pub struct Decoder {
    max_paths: usize,
    beam_width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BeamState {
    spelling_length: usize,
    text: String,
    display_reading: String,
    score: i64,
    segments: usize,
}

/// Scratch space for adjacent paths in one decode; never shared across queries.
#[derive(Default)]
struct Composition {
    prefix: Vec<(SyllableId, bool)>,
    beams: Vec<Vec<BeamState>>,
}

impl Decoder {
    /// Recall existing multi-syllable phrases by frequency, including initials
    /// and mixed spelling, without letting closer completions fill the budget.
    pub fn frequent_phrases(
        &self,
        graph: &ParseGraph,
        lexicon: &Lexicon,
        limit: usize,
    ) -> Vec<DecodedCandidate> {
        if limit == 0 || graph.input_len() == 0 || !graph.accepts_input() {
            return Vec::new();
        }
        let mut graph = graph.clone();
        graph.constrain_closed_syllables(|id| {
            lexicon
                .exact(&[id])
                .iter()
                .any(|entry| entry.text.chars().count() == 1)
        });
        frequent::recall(&graph, lexicon, limit)
    }

    pub fn new(max_paths: usize) -> Self {
        Self::with_limits(max_paths, 32)
    }

    pub fn with_limits(max_paths: usize, beam_width: usize) -> Self {
        Self {
            max_paths: max_paths.max(1),
            beam_width: beam_width.max(1),
        }
    }

    pub fn decode(
        &self,
        graph: &ParseGraph,
        lexicon: &Lexicon,
        limit: usize,
    ) -> Vec<DecodedCandidate> {
        self.decode_with_scorer(graph, lexicon, limit, &|_, _| 0)
    }

    /// Add a sentence transition score before beam pruning. The prefix contains
    /// only text generated in this query; the scorer must return zero for an
    /// empty prefix and remain deterministic throughout one decode.
    pub fn decode_with_scorer(
        &self,
        graph: &ParseGraph,
        lexicon: &Lexicon,
        limit: usize,
        scorer: &impl Fn(&str, &Lexeme) -> i64,
    ) -> Vec<DecodedCandidate> {
        if limit == 0 || graph.input_len() == 0 || !graph.accepts_input() {
            return Vec::new();
        }

        let mut constrained = graph.clone();
        constrained.constrain_closed_syllables(|id| {
            lexicon
                .exact(&[id])
                .iter()
                .any(|entry| entry.text.chars().count() == 1)
        });
        let graph = &constrained;

        let decoder = Self {
            beam_width: self.beam_width.max(limit),
            ..*self
        };
        let mut matches = Vec::new();
        let mut composition = Composition::default();
        let complete_paths = graph.complete_segmentations(self.max_paths);
        // A whole syllable is the longest complete path's first choice. Read
        // its characters directly: beam pruning must not discard them before
        // the single-character priority is applied. Explicit boundaries are
        // already enforced by the graph (e.g. xi'an is not xian).
        if let Some(path) = complete_paths.first().filter(|path| path.len() == 1) {
            matches.extend(
                lexicon
                    .exact(path)
                    .iter()
                    .filter(|entry| entry.text.chars().count() == 1)
                    .map(|entry| DecodedCandidate {
                        spelling_length: entry.reading.spelling_length(),
                        text: entry.text.clone(),
                        display_reading: entry.reading.display(),
                        score: lexicon.score(entry),
                        segments: 1,
                        syllable_count: 1,
                        completed_partial_syllable: false,
                        abbreviated_syllables: 0,
                    }),
            );
            prune_candidates(&mut matches, limit);
            // No combination or completion can outrank these exact characters.
            if matches.len() == limit {
                return matches;
            }
        }
        // A dictionary word must not depend on where its segmentation happens
        // to fall in the bounded composition search.
        matches.extend(whole_words::recall(graph, lexicon, limit, false));
        prune_candidates(&mut matches, limit);
        if matches.len() == limit {
            return matches;
        }
        // Complete paths have their own budget: completions must never crowd
        // them out before ranking has a chance to compare candidates.
        for path in complete_paths {
            matches.extend(
                decoder
                    .compose_path(graph, &path, lexicon, &mut composition, scorer)
                    .into_iter()
                    .map(|state| DecodedCandidate {
                        spelling_length: state.spelling_length,
                        text: state.text,
                        display_reading: state.display_reading,
                        score: state.score,
                        segments: state.segments,
                        syllable_count: path.len(),
                        completed_partial_syllable: false,
                        abbreviated_syllables: 0,
                    }),
            );
        }
        prune_candidates(&mut matches, limit);
        // Every completion/abbreviation ranks below every exact match. Once
        // exact decoding fills the requested unique results, neither search
        // can contribute to the result (personal recall is applied upstream).
        if matches.len() == limit {
            return matches;
        }
        matches.extend(whole_words::recall(graph, lexicon, limit, true));
        // Existing words precede compositions within the same match class.
        prune_candidates(&mut matches, limit);
        if matches.len() == limit {
            return matches;
        }
        let mut path = Vec::new();
        let mut visited_paths = 0;
        let can_finish = graph.can_finish_without_abbreviations();
        decoder.visit(
            graph,
            &can_finish,
            lexicon,
            0,
            &mut path,
            false,
            &mut visited_paths,
            &mut matches,
            &mut composition,
            scorer,
        );

        // A full set of unique exact/prefix matches cannot be displaced by
        // abbreviations. Prune duplicates before deciding whether it is full.
        prune_candidates(&mut matches, limit);
        if matches.len() == limit {
            return matches;
        }

        if graph.has_abbreviations() {
            matches.extend(decoder.decode_abbreviated(graph, lexicon, limit, scorer));
        }
        prune_candidates(&mut matches, limit);
        matches
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        graph: &ParseGraph,
        can_finish: &[bool],
        lexicon: &Lexicon,
        position: usize,
        path: &mut Vec<SyllableId>,
        completed_partial_syllable: bool,
        visited_paths: &mut usize,
        matches: &mut Vec<DecodedCandidate>,
        composition: &mut Composition,
        scorer: &impl Fn(&str, &Lexeme) -> i64,
    ) {
        if *visited_paths >= self.max_paths || !can_finish[position] {
            return;
        }
        if position == graph.input_len() {
            *visited_paths += 1;
            if !completed_partial_syllable {
                return;
            }
            matches.extend(
                self.compose_path(graph, path, lexicon, composition, scorer)
                    .into_iter()
                    .map(|state| DecodedCandidate {
                        spelling_length: state.spelling_length,
                        text: state.text,
                        display_reading: state.display_reading,
                        score: state.score,
                        segments: state.segments,
                        syllable_count: path.len(),
                        completed_partial_syllable,
                        abbreviated_syllables: 0,
                    }),
            );
            return;
        }

        for edge in graph.edges_from(position).iter().rev() {
            match &edge.kind {
                EdgeKind::Complete(id) => {
                    path.push(*id);
                    self.visit(
                        graph,
                        can_finish,
                        lexicon,
                        edge.end,
                        path,
                        completed_partial_syllable,
                        visited_paths,
                        matches,
                        composition,
                        scorer,
                    );
                    path.pop();
                }
                EdgeKind::Incomplete { completions } => {
                    for completion in completions {
                        path.push(*completion);
                        self.visit(
                            graph,
                            can_finish,
                            lexicon,
                            edge.end,
                            path,
                            true,
                            visited_paths,
                            matches,
                            composition,
                            scorer,
                        );
                        path.pop();
                        if *visited_paths >= self.max_paths {
                            break;
                        }
                    }
                }
                EdgeKind::Boundary => self.visit(
                    graph,
                    can_finish,
                    lexicon,
                    edge.end,
                    path,
                    completed_partial_syllable,
                    visited_paths,
                    matches,
                    composition,
                    scorer,
                ),
                EdgeKind::Abbreviated { .. } => {}
            }
        }
    }

    fn compose_path(
        &self,
        graph: &ParseGraph,
        path: &[SyllableId],
        lexicon: &Lexicon,
        composition: &mut Composition,
        scorer: &impl Fn(&str, &Lexeme) -> i64,
    ) -> Vec<BeamState> {
        let Some(edges) = graph.syllable_edges(path) else {
            return Vec::new();
        };
        let single = edges
            .iter()
            .map(|edge| graph.requires_single_character(edge.start))
            .collect::<Vec<_>>();
        let prefix = path
            .iter()
            .copied()
            .zip(single.iter().copied())
            .collect::<Vec<_>>();
        let common = composition
            .prefix
            .iter()
            .zip(&prefix)
            .take_while(|(left, right)| left == right)
            .count();
        composition.prefix = prefix;
        let beams = &mut composition.beams;
        if beams.is_empty() {
            beams.push(vec![BeamState {
                spelling_length: 0,
                text: String::new(),
                display_reading: String::new(),
                score: 0,
                segments: 0,
            }]);
        }
        beams.truncate(common + 1);

        // Each destination receives sources in the same order as forward
        // expansion. Keep completed prefixes so a new tail does not rebuild them.
        for end in common + 1..=path.len() {
            let mut destination = Vec::new();
            for start in 0..end {
                let source_states = &beams[start];
                if source_states.is_empty() {
                    continue;
                }
                let requires_character = single[start..end].iter().any(|required| *required);
                if requires_character && end != start + 1 {
                    continue;
                }
                let mut cutoff = destination
                    .last()
                    .filter(|_| destination.len() == self.beam_width)
                    .map(|state: &BeamState| (state.score, state.segments));
                for lexeme in lexicon.exact(&path[start..end]) {
                    if requires_character && lexeme.text.chars().count() != 1 {
                        continue;
                    }
                    let score = lexicon.score(lexeme);
                    let reading = lexeme.reading.display();
                    for state in source_states {
                        let score = score.saturating_add(scorer(&state.text, lexeme));
                        let next_score = state.score.saturating_add(score);
                        let next_segments = state.segments + 1;
                        // Only reject a strictly worse rank; tied ranks still
                        // need their text for deduplication and lexical ordering.
                        if cutoff.is_some_and(|(score, segments)| {
                            compare_beam_rank(next_score, next_segments, score, segments).is_gt()
                        }) {
                            continue;
                        }
                        destination.push(extend_state(state, lexeme, &reading, score));
                    }
                    // Keep the same best unique states without materializing the
                    // entire dictionary-by-beam product before pruning.
                    if destination.len() >= self.beam_width.saturating_mul(4) {
                        prune_beam(&mut destination, self.beam_width);
                        cutoff = destination
                            .last()
                            .filter(|_| destination.len() == self.beam_width)
                            .map(|state| (state.score, state.segments));
                    }
                }
                prune_beam(&mut destination, self.beam_width);
            }
            beams.push(destination);
        }
        beams[path.len()].clone()
    }
}

fn extend_state(state: &BeamState, lexeme: &Lexeme, reading: &str, score: i64) -> BeamState {
    let mut text = String::with_capacity(state.text.len() + lexeme.text.len());
    text.push_str(&state.text);
    text.push_str(&lexeme.text);

    let mut display_reading = String::with_capacity(
        state.display_reading.len()
            + usize::from(!state.display_reading.is_empty())
            + reading.len(),
    );
    display_reading.push_str(&state.display_reading);
    if !display_reading.is_empty() {
        display_reading.push(' ');
    }
    display_reading.push_str(reading);

    BeamState {
        spelling_length: state.spelling_length + lexeme.reading.spelling_length(),
        text,
        display_reading,
        score: state.score.saturating_add(score),
        segments: state.segments + 1,
    }
}

fn prune_beam(beam: &mut Vec<BeamState>, limit: usize) {
    beam.sort_by(|left, right| {
        compare_beam_rank(left.score, left.segments, right.score, right.segments)
            .then_with(|| left.text.cmp(&right.text))
    });
    retain_unique_text(beam, limit, |state| &state.text);
}

fn compare_beam_rank(
    left_score: i64,
    left_segments: usize,
    right_score: i64,
    right_segments: usize,
) -> Ordering {
    (right_segments == 1)
        .cmp(&(left_segments == 1))
        .then_with(|| right_score.cmp(&left_score))
        .then_with(|| left_segments.cmp(&right_segments))
}

fn prune_candidates(candidates: &mut Vec<DecodedCandidate>, limit: usize) {
    candidates.sort_by(compare_candidates);
    retain_unique_text(candidates, limit, |candidate| &candidate.text);
}

/// Keep the first `limit` unique texts in rank order without cloning their strings.
fn retain_unique_text<T>(items: &mut Vec<T>, limit: usize, text: impl Fn(&T) -> &str) {
    let capacity = limit.min(items.len());
    let mut seen = HashSet::with_capacity(capacity);
    let mut kept = Vec::with_capacity(capacity);
    for (index, item) in items.iter().enumerate() {
        if kept.len() == limit {
            break;
        }
        if seen.insert(text(item)) {
            kept.push(index);
        }
    }
    drop(seen);
    // Indices increase monotonically: swapping into the compacted prefix cannot
    // disturb any source index that has not yet been moved.
    for (destination, &source) in kept.iter().enumerate() {
        items.swap(destination, source);
    }
    items.truncate(kept.len());
}

fn compare_candidates(left: &DecodedCandidate, right: &DecodedCandidate) -> Ordering {
    left.rank()
        .cmp(&right.rank())
        .then_with(|| left.text.cmp(&right.text))
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
#[path = "decoder/abbreviation_tests.rs"]
mod abbreviation_tests;

#[cfg(test)]
mod tests {
    use crate::{Lexeme, Parser, Reading, SyllableTable};

    use super::*;

    #[test]
    fn ranked_dedup_preserves_first_occurrence_and_limit() {
        let ranked = (0..256)
            .map(|index| (format!("词{}", (index * 17) % 37), index))
            .collect::<Vec<_>>();
        for count in [0, 1, 16, 256] {
            for limit in [0, 1, 9, 36, 72, 512] {
                let mut expected = Vec::new();
                for item in &ranked[..count] {
                    if !expected
                        .iter()
                        .any(|previous: &&(String, usize)| previous.0 == item.0)
                    {
                        expected.push(item);
                    }
                }
                expected.truncate(limit);
                let mut actual = ranked[..count].to_vec();
                retain_unique_text(&mut actual, limit, |item| &item.0);
                assert_eq!(actual.iter().collect::<Vec<_>>(), expected);
            }
        }
    }

    fn lexeme(
        table: &SyllableTable,
        input: &[&str],
        display: &[&str],
        text: &str,
        frequency: u64,
    ) -> Lexeme {
        Lexeme::new(
            Reading::new(table, input, display).unwrap(),
            text,
            frequency,
        )
    }

    #[test]
    fn completes_only_the_editable_tail_and_returns_full_readings() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["zhe", "zhong"], &["zhè", "zhǒng"], "这种", 80),
            lexeme(table, &["zhe", "ze"], &["zhé", "zé"], "折则", 1),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("zhez").unwrap(), &lexicon, 10);
        assert_eq!(candidates[0].text, "折则");
        assert_eq!(candidates[0].display_reading, "zhé zé");
        assert!(candidates.iter().any(|candidate| candidate.text == "这种"));
        assert!(candidates[0].completed_partial_syllable);
    }

    #[test]
    fn follows_ambiguous_complete_segmentations() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["xian"], &["xiān"], "先", 20),
            lexeme(table, &["xi", "an"], &["xī", "ān"], "西安", 30),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("xian").unwrap(), &lexicon, 10);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            ["先", "西安"]
        );
    }

    #[test]
    fn composes_multiple_lexemes_when_no_phrase_entry_exists() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["ni"], &["nǐ"], "你", 100),
            lexeme(table, &["hao"], &["hǎo"], "好", 90),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("nihao").unwrap(), &lexicon, 10);
        assert_eq!(candidates[0].text, "你好");
        assert_eq!(candidates[0].display_reading, "nǐ hǎo");
        assert_eq!(candidates[0].segments, 2);
    }

    #[test]
    fn a_frequent_whole_phrase_outranks_fragmented_words() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(
                table,
                &["shu", "ru", "fa"],
                &["shū", "rù", "fǎ"],
                "输入法",
                500,
            ),
            lexeme(table, &["shu", "ru"], &["shū", "rù"], "输入", 100),
            lexeme(table, &["fa"], &["fǎ"], "法", 100),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("shurufa").unwrap(), &lexicon, 10);
        assert_eq!(candidates[0].text, "输入法");
        assert_eq!(candidates[0].segments, 1);
    }

    #[test]
    fn respects_candidate_and_path_limits() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["da"], &["dà"], "大", 30),
            lexeme(table, &["de"], &["de"], "的", 50),
            lexeme(table, &["di"], &["dì"], "地", 40),
        ]);
        let candidates = Decoder::new(256).decode(&parser.parse("d").unwrap(), &lexicon, 2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].text, "的");
    }

    #[test]
    fn complete_syllable_always_precedes_a_partial_alternative() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["fan"], &["fān"], "完整", 1),
            lexeme(table, &["fa"], &["fā"], "分段", 1_000_000),
            lexeme(table, &["na"], &["ná"], "补全", 1_000_000),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("fan").unwrap(), &lexicon, 10);

        assert_eq!(candidates[0].text, "完整");
        assert!(!candidates[0].completed_partial_syllable);
        assert!(
            candidates
                .iter()
                .skip(1)
                .any(|candidate| candidate.completed_partial_syllable)
        );
    }

    #[test]
    fn complete_syllable_survives_a_one_path_budget() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["fan"], &["fān"], "帆", 1),
            lexeme(table, &["fa"], &["fā"], "发", 1_000_000),
            lexeme(table, &["na"], &["ná"], "拿", 1_000_000),
        ]);
        let candidates = Decoder::new(1).decode(&parser.parse("fan").unwrap(), &lexicon, 9);
        assert_eq!(candidates[0].text, "帆");
        assert!(!candidates[0].completed_partial_syllable);
    }

    #[test]
    fn requested_candidates_are_not_truncated_to_the_default_beam_width() {
        let parser = Parser::standard();
        let lexicon = Lexicon::new(
            (0..80)
                .map(|index| {
                    lexeme(
                        parser.table(),
                        &["shi"],
                        &["shì"],
                        &format!("候选{index}"),
                        index,
                    )
                })
                .collect(),
        );
        let candidates = Decoder::default().decode(&parser.parse("shi").unwrap(), &lexicon, 72);
        assert_eq!(candidates.len(), 72);
        assert_eq!(candidates[0].text, "候选79");
        assert_eq!(candidates[71].text, "候选8");
    }

    #[test]
    fn extreme_frequencies_do_not_overflow_during_composition() {
        let parser = Parser::standard();
        let lexicon = Lexicon::new(vec![
            lexeme(parser.table(), &["ni"], &["nǐ"], "你", u64::MAX),
            lexeme(parser.table(), &["hao"], &["hǎo"], "好", u64::MAX),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("nihao").unwrap(), &lexicon, 9);
        assert_eq!(candidates[0].text, "你好");
        assert_eq!(candidates[0].score, -1420);
    }

    #[test]
    fn every_whole_syllable_keeps_its_character_before_beam_pruning() {
        let parser = Parser::standard();
        let decoder = Decoder::with_limits(1, 1);
        for (_, input) in parser.table().iter() {
            let lexicon = Lexicon::new(vec![
                lexeme(parser.table(), &[input], &[input], "字", 0),
                // A dictionary may contain multi-character text for one reading.
                // Even extreme frequency must not evict the exact character.
                lexeme(parser.table(), &[input], &[input], "多字候选", u64::MAX),
            ]);
            let candidates = decoder.decode(&parser.parse(input).unwrap(), &lexicon, 1);
            assert_eq!(candidates[0].text, "字", "whole syllable: {input}");
            assert_eq!(candidates[0].display_reading, input);
        }
    }

    #[test]
    fn exact_characters_are_deduplicated_before_the_early_return() {
        let parser = Parser::standard();
        let lexicon = Lexicon::new(vec![
            lexeme(parser.table(), &["a"], &["ā"], "啊", 100),
            lexeme(parser.table(), &["a"], &["ā"], "阿", 90),
            lexeme(parser.table(), &["a"], &["á"], "啊", 80),
            lexeme(parser.table(), &["a"], &["ǎ"], "呵", 70),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("a").unwrap(), &lexicon, 3);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            ["啊", "阿", "呵"]
        );
        assert_eq!(candidates[0].display_reading, "ā");
    }

    #[test]
    fn exact_characters_precede_combinations_but_explicit_boundaries_win() {
        let parser = Parser::standard();
        let lexicon = Lexicon::new(vec![
            lexeme(parser.table(), &["xian"], &["xiān"], "先", 0),
            lexeme(parser.table(), &["xian"], &["xiàn"], "现", 1),
            lexeme(
                parser.table(),
                &["xi", "an"],
                &["xī", "ān"],
                "西安",
                u64::MAX,
            ),
        ]);
        let decoder = Decoder::default();
        let candidates = decoder.decode(&parser.parse("xian").unwrap(), &lexicon, 3);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            ["现", "先", "西安"]
        );
        let candidates = decoder.decode(&parser.parse("xi'an").unwrap(), &lexicon, 1);
        assert_eq!(candidates[0].text, "西安");
        assert_eq!(candidates[0].syllable_count, 2);
        let candidates = decoder.decode(&parser.parse("xian'").unwrap(), &lexicon, 1);
        assert_eq!(candidates[0].text, "现");
    }

    #[test]
    fn closed_syllables_only_compose_single_characters_even_against_high_frequency_aliases() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["xuan"], &["xuān"], "宣", 1),
            lexeme(table, &["an"], &["ān"], "安", 1),
            lexeme(table, &["xu"], &["xǔ"], "许", u64::MAX),
            lexeme(table, &["xuan"], &["xuān"], "多字宏", u64::MAX),
            lexeme(table, &["shi", "jie"], &["shì", "jiè"], "世界", 100),
            lexeme(table, &["shi"], &["shì"], "世", 1),
            lexeme(table, &["ji"], &["jī"], "机", 1),
            lexeme(table, &["e"], &["é"], "额", 1),
        ]);
        let decoder = Decoder::default();
        let closed = decoder.decode(&parser.parse("xuan'an'").unwrap(), &lexicon, 20);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].text, "宣安");
        assert_eq!(closed[0].display_reading, "xuān ān");
        assert_eq!(closed[0].segments, 2);
        let single = decoder.decode(&parser.parse("xuan'").unwrap(), &lexicon, 20);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].text, "宣");
        let mixed = decoder.decode(&parser.parse("xuan'an'shijie").unwrap(), &lexicon, 20);
        assert!(
            mixed
                .iter()
                .all(|candidate| candidate.display_reading.starts_with("xuān ān "))
        );
        assert!(mixed.iter().any(|candidate| candidate.text == "宣安世界"));
        assert!(mixed.iter().any(|candidate| candidate.text == "宣安世机额"));
        let partial = decoder.decode(&parser.parse("xuan'an'shij").unwrap(), &lexicon, 20);
        assert!(partial.iter().any(|candidate| candidate.text == "宣安世界"));
        assert!(
            partial
                .iter()
                .all(|candidate| candidate.display_reading.starts_with("xuān ān "))
        );
    }

    #[test]
    fn closed_chunk_without_a_single_character_falls_back_to_composition() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["xu"], &["xǔ"], "许", 1),
            lexeme(table, &["an"], &["ān"], "安", 1),
        ]);
        let candidates =
            Decoder::default().decode(&parser.parse("xuan'an'").unwrap(), &lexicon, 10);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].text, "许安安");
    }

    #[test]
    fn existing_phrase_survives_beam_pruning_before_frequent_character_combinations() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["shi"], &["shì"], "是", 1_000_000),
            lexeme(table, &["jie"], &["jiè"], "界", 1_000_000),
            lexeme(table, &["shi", "jie"], &["shì", "jiè"], "世界", 1),
            lexeme(table, &["shi", "ji", "e"], &["shì", "jī", "é"], "世机额", 0),
        ]);
        let candidates =
            Decoder::with_limits(256, 1).decode(&parser.parse("shijie").unwrap(), &lexicon, 1);
        assert_eq!(candidates[0].text, "世界");
        assert_eq!(candidates[0].segments, 1);
    }

    #[test]
    fn log_weights_do_not_reward_splitting_into_more_characters() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["shi"], &["shì"], "世", 1000),
            lexeme(table, &["jie"], &["jiè"], "界", 1000),
            lexeme(table, &["ji"], &["jī"], "机", 1000),
            lexeme(table, &["e"], &["é"], "额", 1000),
        ]);
        let candidates = Decoder::default().decode(&parser.parse("shijie").unwrap(), &lexicon, 10);
        assert_eq!(candidates[0].text, "世界");
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.text == "世机额")
        );
        assert!(candidates.iter().all(|candidate| candidate.score < 0));
    }

    #[test]
    fn sentence_scores_participate_before_full_partial_and_abbreviation_pruning() {
        let parser = Parser::standard();
        let table = parser.table();
        let lexicon = Lexicon::new(vec![
            lexeme(table, &["wo"], &["wǒ"], "我", 100),
            lexeme(table, &["xie"], &["xiē"], "些", 100),
            lexeme(table, &["xie"], &["xiě"], "写", 1),
        ]);
        let decoder = Decoder::with_limits(256, 1);
        for input in ["woxie", "woxi", "wxie", "wx"] {
            let graph = parser.parse(input).unwrap();
            assert_eq!(decoder.decode(&graph, &lexicon, 1)[0].text, "我些");
            let scored = decoder.decode_with_scorer(&graph, &lexicon, 1, &|prefix, next| {
                if prefix == "我" && next.text == "写" {
                    5000
                } else {
                    0
                }
            });
            assert_eq!(scored[0].text, "我写", "{input}");
        }
    }

    #[test]
    fn whole_dictionary_phrase_is_not_lost_to_segmentation_budget() {
        let parser = Parser::standard();
        let syllables = ["xi", "an"].repeat(9);
        let phrase = "西安".repeat(9);
        let lexicon = Lexicon::new(vec![
            lexeme(parser.table(), &["xian"], &["xiān"], "先", 100),
            lexeme(parser.table(), &syllables, &syllables, &phrase, 1),
        ]);
        let graph = parser.parse(&"xian".repeat(9)).unwrap();
        for budget in [1, 256] {
            let decoder = Decoder::with_limits(budget, 1);
            let candidates = decoder.decode(&graph, &lexicon, 1);
            assert_eq!(candidates[0].text, phrase);
            let partial = parser.parse(&format!("{}xia", "xian".repeat(8))).unwrap();
            let candidates = decoder.decode(&partial, &lexicon, 1);
            assert_eq!(candidates[0].text, phrase);
            assert!(candidates[0].completed_partial_syllable);
            let locked = parser.parse(&"xian'".repeat(9)).unwrap();
            let candidates = decoder.decode(&locked, &lexicon, 1);
            assert_eq!(candidates[0].text, "先".repeat(9));
        }
    }
}
