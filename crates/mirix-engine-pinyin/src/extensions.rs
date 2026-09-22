//! Non-pinyin lookup sources, kept separate from syllable lexicons and learning.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;

use crate::profiles::ProfileError;
use mirix_pinyin_core::MatchQuality;

#[derive(Debug, Clone, Copy)]
enum Mode {
    Exact,
    Completion,
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub text: String,
    weight: u64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Match<'a> {
    pub entry: &'a Entry,
    pub quality: MatchQuality,
}

impl Ord for Match<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.quality
            .cmp(&other.quality)
            .then_with(|| other.entry.weight.cmp(&self.entry.weight))
            .then_with(|| self.entry.text.cmp(&other.entry.text))
    }
}
impl PartialOrd for Match<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Match<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Match<'_> {}

#[derive(Default, Clone)]
pub(crate) struct Extensions {
    exact: BTreeMap<String, Vec<Entry>>,
    completion: BTreeMap<String, Vec<Entry>>,
}

impl Extensions {
    pub fn load(directory: &Path) -> Result<Self, ProfileError> {
        let mut extensions = Self::default();
        for (name, mode) in [
            ("exact.tsv", Mode::Exact),
            ("completion.tsv", Mode::Completion),
        ] {
            let path = directory.join(name);
            let source = match read_or_create_empty(&path) {
                Ok(source) => source,
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("{}: {error}", path.display()),
                    )
                    .into());
                }
            };
            for (index, raw) in source.lines().enumerate() {
                let line = raw.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                extensions.insert_line(raw, mode).map_err(|message| {
                    ProfileError::Extensions(format!("{}:{}: {message}", path.display(), index + 1))
                })?;
            }
        }
        Ok(extensions)
    }

    fn insert_line(&mut self, line: &str, mode: Mode) -> Result<(), String> {
        let (key, rest) = line
            .trim()
            .split_once(char::is_whitespace)
            .ok_or("expected input text [weight], separated by whitespace")?;
        if key.is_empty() || !key.bytes().all(|c| c.is_ascii_lowercase()) {
            return Err("input must contain lowercase ASCII letters".into());
        }
        let rest = rest.trim_start();
        let (text, weight) = if rest.starts_with('"') {
            let mut strings = serde_json::Deserializer::from_str(rest).into_iter::<String>();
            let text = strings
                .next()
                .ok_or("output text is missing")?
                .map_err(|error| format!("invalid quoted output: {error}"))?;
            let remaining = &rest[strings.byte_offset()..];
            if !remaining.is_empty() && !remaining.starts_with(char::is_whitespace) {
                return Err("expected whitespace after quoted output".into());
            }
            (text, remaining.trim())
        } else {
            let (text, remaining) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            if remaining.split_whitespace().count() > 1 {
                return Err("expected input text [weight]; put output containing whitespace in double quotes".into());
            }
            (text.to_owned(), remaining.trim())
        };
        if text.trim().is_empty() {
            return Err("output text must not be empty".into());
        }
        let weight = if weight.is_empty() { "0" } else { weight }
            .parse::<u64>()
            .map_err(|_| "weight must be an unsigned integer")?;
        let entries = match mode {
            Mode::Completion => &mut self.completion,
            Mode::Exact => &mut self.exact,
        };
        entries
            .entry(key.to_owned())
            .or_default()
            .push(Entry { text, weight });
        Ok(())
    }

    fn matches<'a>(&'a self, input: &str) -> impl Iterator<Item = Match<'a>> + 'a {
        let exact_quality = MatchQuality::extension(input, input);
        let input_len = input.len();
        let prefix = input.to_owned();
        self.exact
            .get(input)
            .into_iter()
            .flatten()
            .map(move |entry| Match {
                entry,
                quality: exact_quality,
            })
            .chain(
                self.completion
                    .range(prefix.clone()..)
                    .take_while(move |(key, _)| !prefix.is_empty() && key.starts_with(&prefix))
                    .flat_map(move |(key, entries)| {
                        let quality = MatchQuality::extension(&key[..input_len], key);
                        entries.iter().map(move |entry| Match { entry, quality })
                    }),
            )
    }

    pub fn query<'a>(
        shared: &'a Self,
        profile: &'a Self,
        input: &str,
        limit: usize,
    ) -> Vec<Match<'a>> {
        if limit == 0 || input.is_empty() {
            return Vec::new();
        }
        // Both containers hold at most limit unique outputs. Profile duplicates
        // can replace shared entries without reserving an additional slot.
        let mut best = BTreeSet::<Match<'a>>::new();
        let mut by_text = BTreeMap::<&str, Match<'a>>::new();
        for item in shared.matches(input).chain(profile.matches(input)) {
            if let Some(previous) = by_text.get(item.entry.text.as_str()).copied() {
                if item >= previous {
                    continue;
                }
                best.remove(&previous);
            } else if best.len() == limit {
                if best.last().is_some_and(|worst| item >= *worst) {
                    continue;
                }
                let worst = best.pop_last().unwrap();
                by_text.remove(worst.entry.text.as_str());
            }
            by_text.insert(item.entry.text.as_str(), item);
            best.insert(item);
        }
        best.into_iter().collect()
    }
}

fn read_or_create_empty(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(source) => return Ok(source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::File::options().write(true).create_new(true).open(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    // Another process may have created and populated it in the meantime.
    fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_selection_matches_exhaustive_sort_and_deduplication() {
        let mut shared = Extensions::default();
        let mut profile = Extensions::default();
        for (source_index, source) in [&mut shared, &mut profile].into_iter().enumerate() {
            for index in 0..400 {
                let suffix = "a".repeat(index % 13);
                let line = format!(
                    "go{suffix} result{} {}",
                    index % 91,
                    (index * 37 + source_index * 101) % 997
                );
                source.insert_line(&line, Mode::Completion).unwrap();
                if index % 7 == 0 {
                    source
                        .insert_line(&format!("go result{} {index}", index % 91), Mode::Exact)
                        .unwrap();
                }
            }
        }
        for input in ["go", "goa", "goaaaa", "missing"] {
            let mut exhaustive = shared
                .matches(input)
                .chain(profile.matches(input))
                .collect::<Vec<_>>();
            exhaustive.sort();
            let mut seen = BTreeSet::new();
            exhaustive.retain(|item| seen.insert(item.entry.text.as_str()));
            for limit in [0, 1, 9, 72, 200] {
                assert_eq!(
                    Extensions::query(&shared, &profile, input, limit),
                    exhaustive.iter().copied().take(limit).collect::<Vec<_>>(),
                    "{input}, limit={limit}"
                );
            }
        }
    }

    #[test]
    fn independent_sources_match_without_pinyin_and_preserve_output() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("completion.tsv"),
            "# words\ngoogle\tGoogle\t1000\r\ngood\tgood\n",
        )
        .unwrap();
        fs::write(
            root.path().join("exact.tsv"),
            "goo\tGoole\t1000\naddr\t\"Room 12, Main Street\"\nsmile\t👩‍💻\t100\nxiaolian\t👩‍💻\n",
        )
        .unwrap();
        let shared = Extensions::load(root.path()).unwrap();
        let profile = Extensions::default();
        let query = |input| Extensions::query(&shared, &profile, input, 72);
        assert_eq!(
            query("goo")
                .iter()
                .map(|item| item.entry.text.as_str())
                .collect::<Vec<_>>(),
            ["Goole", "good", "Google"]
        );
        assert_eq!(query("go").len(), 2);
        assert_eq!(query("smile")[0].entry.text, "👩‍💻");
        assert_eq!(query("xiaolian")[0].entry.text, "👩‍💻");
        assert_eq!(query("addr")[0].entry.text, "Room 12, Main Street");
        assert!(query("smi").is_empty());
        assert!(query("goo'").is_empty());
        assert!(query("").is_empty());
    }

    #[test]
    fn profile_merge_ranks_exact_words_before_prefixes_and_deduplicates() {
        let mut shared = Extensions::default();
        shared.insert_line("go\tgo", Mode::Completion).unwrap();
        shared
            .insert_line("google\tGoogle\t1000", Mode::Completion)
            .unwrap();
        let mut profile = Extensions::default();
        profile
            .insert_line("google\tGoogle\t2000", Mode::Completion)
            .unwrap();
        let matches = Extensions::query(&shared, &profile, "go", 72);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].entry.text, "go");
        assert_eq!(matches[1].entry.weight, 2000);
        assert_eq!(Extensions::query(&shared, &profile, "go", 1).len(), 1);
    }

    #[test]
    fn missing_files_are_empty_and_parse_errors_preserve_existing_content() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("extensions");
        assert!(Extensions::load(&directory).is_ok());
        assert_eq!(fs::read(directory.join("exact.tsv")).unwrap(), b"");
        assert_eq!(fs::read(directory.join("completion.tsv")).unwrap(), b"");
        let path = directory.join("exact.tsv");
        for (line, expected) in [
            ("goo", "input text"),
            ("goo\tGoole\t-1", "weight"),
            ("goo1\tGoole", "input"),
            ("goo\t\"   \"\t1", "text"),
            ("goo \"unfinished", "quoted output"),
            ("goo \"Goole\"1000", "whitespace"),
            ("goo Room 12 Main Street", "double quotes"),
        ] {
            let source = format!("# comment\n\n{line}\n");
            fs::write(&path, &source).unwrap();
            let error = Extensions::load(&directory).err().unwrap().to_string();
            assert!(error.contains(&format!("{}:3:", path.display())), "{error}");
            assert!(error.contains(expected), "{error}");
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(Extensions::load(&directory).is_err());
        assert!(path.is_dir());
    }

    #[test]
    fn all_sources_accept_mixed_unicode_whitespace_and_optional_weights() {
        let root = tempfile::tempdir().unwrap();
        for name in ["completion.tsv", "exact.tsv"] {
            fs::write(
                root.path().join(name),
                concat!(
                    "  # comment\r\n",
                    "\u{3000}goo  Goole  1000\r\n",
                    "goo\tGoogle 900\n",
                    "goo\u{00a0}good\u{2003}800\n",
                    "goo \t 😊\u{3000}700\n",
                    "goo\u{2009}NoWeight\u{2009}\n",
                ),
            )
            .unwrap();
        }
        let shared = Extensions::load(root.path()).unwrap();
        for source in [&shared.exact, &shared.completion] {
            let entries = source["goo"]
                .iter()
                .map(|item| (item.text.as_str(), item.weight))
                .collect::<Vec<_>>();
            assert_eq!(
                entries,
                [
                    ("Goole", 1000),
                    ("Google", 900),
                    ("good", 800),
                    ("😊", 700),
                    ("NoWeight", 0)
                ]
            );
        }
    }

    #[test]
    fn quoted_outputs_preserve_spaces_numbers_and_standard_string_escapes() {
        let mut extensions = Extensions::default();
        for (line, text, weight) in [
            (
                "addr \"Room 12, Main Street\" 1000",
                "Room 12, Main Street",
                1000,
            ),
            ("addr\u{3000}\"Room 12\"", "Room 12", 0),
            ("addr\t\"  Room  12  \"\t20", "  Room  12  ", 20),
            (r#"addr "say \"hi\"\nnext" 50"#, "say \"hi\"\nnext", 50),
        ] {
            extensions.insert_line(line, Mode::Exact).unwrap();
            let entry = extensions.exact["addr"].last().unwrap();
            assert_eq!(entry.text, text);
            assert_eq!(entry.weight, weight);
        }
    }
}
