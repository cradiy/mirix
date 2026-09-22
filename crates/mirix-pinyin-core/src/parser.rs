use crate::{SyllableId, SyllableTable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    Complete(SyllableId),
    Incomplete { completions: Vec<SyllableId> },
    Abbreviated { completions: Vec<SyllableId> },
    Boundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub start: usize,
    pub end: usize,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseGraph {
    input_len: usize,
    edges: Vec<Vec<Edge>>,
    accepts_input: bool,
    single_character: Vec<bool>,
}

impl ParseGraph {
    pub fn input_len(&self) -> usize {
        self.input_len
    }

    pub fn edges_from(&self, position: usize) -> &[Edge] {
        self.edges.get(position).map_or(&[], Vec::as_slice)
    }

    pub fn accepts_input(&self) -> bool {
        self.accepts_input
    }

    /// Lock apostrophe-terminated chunks only when their full syllable has a character.
    /// The open tail remains unrestricted. Apply before enumerating bounded paths.
    pub fn constrain_closed_syllables(
        &mut self,
        mut has_character: impl FnMut(SyllableId) -> bool,
    ) {
        let mut start = 0;
        for end in 0..self.input_len {
            if !self.edges[end]
                .iter()
                .any(|edge| matches!(edge.kind, EdgeKind::Boundary))
            {
                continue;
            }
            let whole = self.edges[start].iter().find_map(|edge| match edge.kind {
                EdgeKind::Complete(id) if edge.end == end => Some(id),
                _ => None,
            });
            if let Some(id) = whole.filter(|id| has_character(*id)) {
                self.edges[start]
                    .retain(|edge| edge.end == end && edge.kind == EdgeKind::Complete(id));
                self.single_character[start] = true;
            }
            start = end + 1;
        }
    }

    pub fn requires_single_character(&self, position: usize) -> bool {
        self.single_character
            .get(position)
            .copied()
            .unwrap_or(false)
    }

    /// Resolve a fixed reading against this graph, retaining input offsets and constraints.
    pub fn syllable_edges(&self, reading: &[SyllableId]) -> Option<Vec<&Edge>> {
        // Match a fixed reading with dynamic programming: s/sh and c/ch can
        // reach the same syllable using different byte lengths. Greedily
        // taking the first edge would lose valid readings.
        let mut paths: Vec<Option<(usize, Vec<&Edge>)>> = vec![None; self.input_len + 1];
        paths[0] = Some((0, Vec::new()));
        for id in reading {
            let mut next: Vec<Option<(usize, Vec<&Edge>)>> = vec![None; self.input_len + 1];
            for (position, path) in paths.iter().enumerate() {
                let Some((cost, path)) = path else {
                    continue;
                };
                let position = self.skip_boundary(position);
                for edge in self.edges_from(position) {
                    let penalty = match &edge.kind {
                        EdgeKind::Complete(value) if value == id => 0,
                        EdgeKind::Incomplete { completions } if completions.contains(id) => 1,
                        EdgeKind::Abbreviated { completions } if completions.contains(id) => 2,
                        _ => continue,
                    };
                    let cost = cost + penalty;
                    if next[edge.end].as_ref().is_none_or(|(best, _)| cost < *best) {
                        let mut path = path.clone();
                        path.push(edge);
                        next[edge.end] = Some((cost, path));
                    }
                }
            }
            paths = next;
        }
        paths
            .into_iter()
            .enumerate()
            .filter(|(position, _)| self.skip_boundary(*position) == self.input_len)
            .filter_map(|(_, path)| path)
            .min_by_key(|(cost, _)| *cost)
            .map(|(_, edges)| edges)
    }

    pub(crate) fn skip_boundary(&self, position: usize) -> usize {
        self.edges_from(position)
            .iter()
            .find_map(|edge| matches!(edge.kind, EdgeKind::Boundary).then_some(edge.end))
            .unwrap_or(position)
    }

    pub fn has_abbreviations(&self) -> bool {
        self.edges
            .iter()
            .flatten()
            .any(|edge| matches!(edge.kind, EdgeKind::Abbreviated { .. }))
    }

    pub(crate) fn can_finish_without_abbreviations(&self) -> Vec<bool> {
        let mut can_finish = vec![false; self.input_len + 1];
        can_finish[self.input_len] = true;
        for position in (0..self.input_len).rev() {
            can_finish[position] = self.edges_from(position).iter().any(|edge| {
                !matches!(edge.kind, EdgeKind::Abbreviated { .. }) && can_finish[edge.end]
            });
        }
        can_finish
    }

    /// Complete segmentations only. Paths ending in an incomplete syllable
    /// remain available through [`Self::edges_from`].
    pub fn complete_segmentations(&self, limit: usize) -> Vec<Vec<SyllableId>> {
        fn visit(
            graph: &ParseGraph,
            can_finish: &[bool],
            position: usize,
            path: &mut Vec<SyllableId>,
            output: &mut Vec<Vec<SyllableId>>,
            limit: usize,
        ) {
            if output.len() >= limit {
                return;
            }
            if position == graph.input_len {
                output.push(path.clone());
                return;
            }
            // Prefer longer complete syllables when the path budget is limited.
            for edge in graph.edges_from(position).iter().rev() {
                if !can_finish[edge.end] {
                    continue;
                }
                match edge.kind {
                    EdgeKind::Complete(id) => {
                        path.push(id);
                        visit(graph, can_finish, edge.end, path, output, limit);
                        path.pop();
                    }
                    EdgeKind::Boundary => visit(graph, can_finish, edge.end, path, output, limit),
                    EdgeKind::Incomplete { .. } | EdgeKind::Abbreviated { .. } => {}
                }
            }
        }

        // A partial tail can leave exponentially many complete prefixes with
        // no complete path. Discard those branches before enumerating paths.
        let mut can_finish = vec![false; self.input_len + 1];
        can_finish[self.input_len] = true;
        for position in (0..self.input_len).rev() {
            can_finish[position] = self.edges_from(position).iter().any(|edge| {
                matches!(edge.kind, EdgeKind::Complete(_) | EdgeKind::Boundary)
                    && can_finish[edge.end]
            });
        }
        let mut output = Vec::new();
        if can_finish[0] {
            visit(self, &can_finish, 0, &mut Vec::new(), &mut output, limit);
        }
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid pinyin character {character:?} at byte {index}")]
pub struct ParseError {
    pub index: usize,
    pub character: char,
}

#[derive(Debug, Clone)]
pub struct Parser {
    table: SyllableTable,
}

impl Parser {
    pub fn new(table: SyllableTable) -> Self {
        Self { table }
    }

    pub fn standard() -> Self {
        Self::new(SyllableTable::standard())
    }

    pub fn table(&self) -> &SyllableTable {
        &self.table
    }

    pub fn parse(&self, input: &str) -> Result<ParseGraph, ParseError> {
        for (index, character) in input.char_indices() {
            if !(character.is_ascii_lowercase() || character == '\'') {
                return Err(ParseError { index, character });
            }
        }

        let mut edges = vec![Vec::new(); input.len() + 1];
        let mut reachable = vec![false; input.len() + 1];
        reachable[0] = true;

        for start in 0..input.len() {
            if !reachable[start] {
                continue;
            }
            if input.as_bytes()[start] == b'\'' {
                if start > 0 && input.as_bytes()[start - 1] != b'\'' {
                    edges[start].push(Edge {
                        start,
                        end: start + 1,
                        kind: EdgeKind::Boundary,
                    });
                    reachable[start + 1] = true;
                }
                continue;
            }

            for end in start + 1..=input.len() {
                if input.as_bytes().get(end - 1) == Some(&b'\'') {
                    break;
                }
                let spelling = &input[start..end];
                let exact = self.table.id(spelling);
                let completions = self.table.completions(spelling);
                if exact.is_none() && completions.is_empty() {
                    break;
                }
                if let Some(id) = exact {
                    edges[start].push(Edge {
                        start,
                        end,
                        kind: EdgeKind::Complete(id),
                    });
                    reachable[end] = true;
                }
                if end == input.len() && !completions.is_empty() {
                    edges[start].push(Edge {
                        start,
                        end,
                        kind: EdgeKind::Incomplete {
                            completions: completions.to_vec(),
                        },
                    });
                    reachable[end] = true;
                }
                if end < input.len() && is_initial(spelling) && !completions.is_empty() {
                    edges[start].push(Edge {
                        start,
                        end,
                        kind: EdgeKind::Abbreviated {
                            completions: completions.to_vec(),
                        },
                    });
                    reachable[end] = true;
                }
            }
        }

        Ok(ParseGraph {
            input_len: input.len(),
            edges,
            accepts_input: reachable[input.len()],
            single_character: vec![false; input.len() + 1],
        })
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::standard()
    }
}

fn is_initial(spelling: &str) -> bool {
    matches!(
        spelling,
        "b" | "p"
            | "m"
            | "f"
            | "d"
            | "t"
            | "n"
            | "l"
            | "g"
            | "k"
            | "h"
            | "j"
            | "q"
            | "x"
            | "z"
            | "c"
            | "s"
            | "r"
            | "y"
            | "w"
            | "zh"
            | "ch"
            | "sh"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spellings(parser: &Parser, graph: &ParseGraph) -> Vec<Vec<String>> {
        graph
            .complete_segmentations(64)
            .into_iter()
            .map(|path| {
                path.into_iter()
                    .map(|id| parser.table().spelling(id).unwrap().to_owned())
                    .collect()
            })
            .collect()
    }

    fn incomplete_spellings(parser: &Parser, graph: &ParseGraph, start: usize) -> Vec<String> {
        graph
            .edges_from(start)
            .iter()
            .find_map(|edge| match &edge.kind {
                EdgeKind::Incomplete { completions } => Some(
                    completions
                        .iter()
                        .map(|id| parser.table().spelling(*id).unwrap().to_owned())
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn ambiguous_prefix_with_partial_tail_has_no_complete_segmentation() {
        let parser = Parser::standard();
        let input = format!("{}z", "xian".repeat(30));
        let graph = parser.parse(&input).unwrap();
        assert!(graph.accepts_input());
        assert!(graph.complete_segmentations(256).is_empty());
    }

    #[test]
    fn keeps_ambiguous_complete_segmentations() {
        let parser = Parser::standard();
        let graph = parser.parse("xian").unwrap();
        let paths = spellings(&parser, &graph);
        assert!(paths.contains(&vec!["xian".into()]));
        assert!(paths.contains(&vec!["xi".into(), "an".into()]));
    }

    #[test]
    fn explicit_apostrophe_is_a_hard_boundary() {
        let parser = Parser::standard();
        let graph = parser.parse("xi'an").unwrap();
        assert_eq!(
            spellings(&parser, &graph),
            vec![vec!["xi".to_owned(), "an".to_owned()]]
        );
    }

    #[test]
    fn represents_a_partial_first_syllable() {
        let parser = Parser::standard();
        let graph = parser.parse("d").unwrap();
        assert!(graph.accepts_input());
        let completions = incomplete_spellings(&parser, &graph, 0);
        assert!(completions.contains(&"da".into()));
        assert!(completions.contains(&"duo".into()));
    }

    #[test]
    fn represents_partial_syllable_after_complete_prefix() {
        let parser = Parser::standard();
        for (input, complete, partial_start, expected) in
            [("chux", "chu", 3, "xue"), ("zhez", "zhe", 3, "zhong")]
        {
            let graph = parser.parse(input).unwrap();
            assert!(graph.accepts_input(), "{input}");
            assert!(graph.edges_from(0).iter().any(|edge| {
                matches!(edge.kind, EdgeKind::Complete(id) if parser.table().spelling(id) == Some(complete))
                    && edge.end == partial_start
            }));
            assert!(
                incomplete_spellings(&parser, &graph, partial_start).contains(&expected.into())
            );
        }
    }

    #[test]
    fn exact_tail_can_also_remain_open_for_a_longer_syllable() {
        let parser = Parser::standard();
        let graph = parser.parse("an").unwrap();
        assert!(spellings(&parser, &graph).contains(&vec!["an".into()]));
        assert!(incomplete_spellings(&parser, &graph, 0).contains(&"ang".into()));
    }

    #[test]
    fn rejects_invalid_characters_without_panicking() {
        assert_eq!(
            Parser::standard().parse("ni3").unwrap_err(),
            ParseError {
                index: 2,
                character: '3'
            }
        );
    }

    #[test]
    fn reports_unparseable_ascii_without_an_error() {
        let graph = Parser::standard().parse("vvv").unwrap();
        assert!(!graph.accepts_input());
        assert!(graph.complete_segmentations(64).is_empty());
    }

    #[test]
    fn closed_chunks_lock_before_path_budget_without_locking_open_tail() {
        let parser = Parser::standard();
        let mut graph = parser.parse("xuan'an'shijie").unwrap();
        graph.constrain_closed_syllables(|_| true);
        let paths = spellings(&parser, &graph);
        assert!(!paths.is_empty());
        assert!(
            paths
                .iter()
                .all(|path| path[0] == "xuan" && path[1] == "an")
        );
        assert!(paths.iter().any(|path| path[2..] == ["shi", "jie"]));
        assert!(paths.iter().any(|path| path[2..] == ["shi", "ji", "e"]));
        assert_eq!(
            graph.complete_segmentations(1)[0][0],
            parser.table().id("xuan").unwrap()
        );
        let invalid = ["xu", "an", "an", "shi", "jie"].map(|part| parser.table().id(part).unwrap());
        assert!(graph.syllable_edges(&invalid).is_none());
    }

    #[test]
    fn unavailable_whole_character_keeps_internal_splits() {
        let parser = Parser::standard();
        let mut graph = parser.parse("xuan'an'").unwrap();
        graph.constrain_closed_syllables(|id| parser.table().spelling(id) == Some("an"));
        assert!(
            spellings(&parser, &graph)
                .iter()
                .any(|path| path == &["xu", "an", "an"])
        );
        assert!(!graph.requires_single_character(0));
        assert!(graph.requires_single_character(5));
    }

    #[test]
    fn every_available_closed_syllable_has_exactly_one_segmentation() {
        let parser = Parser::standard();
        for (id, spelling) in parser.table().iter() {
            let mut graph = parser.parse(&format!("{spelling}'")).unwrap();
            graph.constrain_closed_syllables(|candidate| candidate == id);
            assert_eq!(graph.complete_segmentations(64), [vec![id]], "{spelling}");
            assert!(graph.requires_single_character(0), "{spelling}");
        }
    }
}
