use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use mirix_engine_protocol::{
    ActionResult, Candidate, CandidateId, Commit, Engine, EngineMenu, EngineOutput, KeyEvent,
    MenuEntry, PunctuationMode, keysym,
};
use mirix_pinyin_core::{Decoder, ImportIssue, Lexicon, Parser, import_character_tsv, import_tsv};

use crate::extensions::Extensions;
use crate::keys::{input_letter, is_apostrophe, selection_index};
use crate::learning::{self, Selection};
use crate::profiles::{ProfileError, Profiles};
use crate::punctuation::Punctuation;
use mirix_pinyin_core::MatchQuality;

const PREFIX_CANDIDATE_LIMIT: usize = 72;

mod cache;
mod reload;

struct PendingCandidate {
    text: String,
    reading: Option<String>,
    quality: MatchQuality,
}

#[derive(Debug, thiserror::Error)]
pub enum DictionaryError {
    #[error("read dictionary: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputProfiles {
    pub active: Option<String>,
    pub available: Vec<String>,
}

pub struct PinyinEngine {
    config_directory: Option<std::path::PathBuf>,
    menu_label: String,
    configuration_revision: u64,
    show_reading: bool,
    parser: Parser,
    decoder: Decoder,
    decode_cache: cache::DecodeCache,
    core_lexicon: Lexicon,
    active_lexicon: Lexicon,
    extensions: Extensions,
    profiles: Profiles,
    history: String,
    punctuation: Punctuation,
    punctuation_switch_key: Option<u32>,
    buffer: String,
    candidates: Vec<Candidate>,
    candidate_readings: HashMap<CandidateId, String>,
    selected: usize,
    page_ends: Vec<usize>,
    next_candidate_id: u64,
}

impl PinyinEngine {
    pub fn profiles(&self) -> InputProfiles {
        InputProfiles {
            active: Some(self.profiles.active.clone()),
            available: self.profiles.entries.keys().cloned().collect(),
        }
    }

    pub fn activate_profile(&mut self, name: &str) -> Result<(), String> {
        if self.profiles.active == name {
            return Ok(());
        }
        if !self.profiles.activate(name) {
            return Err(format!("unknown input profile: {name}"));
        }
        self.history.clear();
        self.punctuation = Punctuation::default();
        self.rebuild_active_lexicon();
        self.refresh_candidates();
        self.configuration_revision = self.configuration_revision.wrapping_add(1);
        Ok(())
    }

    fn set_punctuation_mode(&mut self, mode: PunctuationMode) {
        if self.profiles.punctuation_mode != mode {
            self.profiles.set_punctuation_mode(mode);
            self.punctuation = Punctuation::default();
            self.configuration_revision = self.configuration_revision.wrapping_add(1);
        }
    }
    fn empty() -> Self {
        Self {
            config_directory: None,
            decode_cache: cache::DecodeCache::default(),
            configuration_revision: 0,
            show_reading: true,
            menu_label: "Profiles".into(),
            parser: Parser::standard(),
            decoder: Decoder::default(),
            core_lexicon: Lexicon::default(),
            active_lexicon: Lexicon::default(),
            extensions: Extensions::default(),
            profiles: Profiles::memory(),
            history: String::new(),
            punctuation: Punctuation::default(),
            punctuation_switch_key: None,
            buffer: String::new(),
            candidates: Vec::new(),
            candidate_readings: HashMap::new(),
            selected: 0,
            page_ends: Vec::new(),
            next_candidate_id: 1,
        }
    }

    /// Load editable character and shared dictionaries, without personal learning storage.
    pub fn from_config(config: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let mut engine = Self::load_shared_sources(config.as_ref())?;
        engine.rebuild_active_lexicon();
        Ok(engine)
    }

    fn load_shared_sources(config: &Path) -> Result<Self, ProfileError> {
        let mut engine = Self::empty();
        engine.config_directory = Some(config.to_path_buf());
        let path = config.join("config.toml");
        let document =
            mirix_config::read(&path).map_err(|error| std::io::Error::other(error.to_string()))?;
        engine.show_reading = match document.get("show_reading") {
            None => true,
            Some(value) => value.as_bool().ok_or_else(|| {
                std::io::Error::other(format!(
                    "{}: show_reading must be a boolean",
                    path.display()
                ))
            })?,
        };
        let characters = crate::characters::load(config, engine.parser.table())?;
        let words = crate::profiles::load_dictionaries(
            &config.join("dictionaries"),
            engine.parser.table(),
        )?;
        engine.core_lexicon = characters.into_iter().chain(words).collect();
        engine.extensions = Extensions::load(&config.join("extensions"))?;
        Ok(engine)
    }

    pub fn open(root: impl AsRef<Path>, config: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let mut engine = Self::load_shared_sources(config.as_ref())?;
        engine.profiles = Profiles::open(root.as_ref(), config.as_ref(), engine.parser.table())?;
        engine.rebuild_active_lexicon();
        Ok(engine)
    }

    pub fn open_default() -> Result<Self, ProfileError> {
        let root = dirs::data_dir().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "user data directory unavailable",
            )
        })?;
        Self::open(
            root.join("mirix/input-method-data/pinyin"),
            Self::default_config_dir()?,
        )
    }

    pub fn default_config_dir() -> Result<std::path::PathBuf, ProfileError> {
        dirs::config_dir()
            .map(|root| root.join("mirix/input-methods/pinyin"))
            .ok_or_else(|| std::io::Error::other("user configuration directory unavailable").into())
    }

    /// Load shared words without opening or modifying personal learning storage.
    pub fn load_shared_dictionaries(
        &mut self,
        directory: impl AsRef<Path>,
    ) -> Result<(), ProfileError> {
        let words = crate::profiles::load_dictionaries(directory.as_ref(), self.parser.table())?;
        self.merge_lexicon(words);
        Ok(())
    }

    fn rebuild_active_lexicon(&mut self) {
        self.decode_cache.clear();
        let profile = self.profiles.current();
        let mut personal = profile.learning.single_characters(self.parser.table());
        personal.extend(profile.vocabulary.lexemes(self.parser.table()));
        self.active_lexicon = if profile.lexicon.is_empty() && personal.is_empty() {
            self.core_lexicon.clone()
        } else {
            self.core_lexicon
                .entries()
                .iter()
                .chain(profile.lexicon.entries())
                .cloned()
                .chain(personal.into_iter().map(Arc::new))
                .collect()
        };
    }

    pub fn load_dictionary_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<ImportIssue>, DictionaryError> {
        let source = std::fs::read_to_string(path)?;
        Ok(self.extend_dictionary(&source))
    }

    pub fn extend_dictionary(&mut self, source: &str) -> Vec<ImportIssue> {
        let imported = import_tsv(source, self.parser.table());
        self.merge_lexicon(imported.lexicon);
        imported.issues
    }

    pub fn load_character_dictionary_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<ImportIssue>, DictionaryError> {
        let source = std::fs::read_to_string(path)?;
        Ok(self.extend_character_dictionary(&source))
    }

    pub fn extend_character_dictionary(&mut self, source: &str) -> Vec<ImportIssue> {
        let imported = import_character_tsv(source, self.parser.table());
        self.merge_lexicon(imported.lexicon);
        imported.issues
    }

    fn merge_lexicon(&mut self, lexicon: Lexicon) {
        self.core_lexicon = std::mem::take(&mut self.core_lexicon)
            .into_iter()
            .chain(lexicon)
            .collect();
        self.rebuild_active_lexicon();
    }

    fn refresh_candidates(&mut self) {
        if self.buffer.is_empty() {
            self.clear();
            return;
        }
        self.page_ends.clear();
        let extensions = Extensions::query(
            &self.extensions,
            &self.profiles.current().extensions,
            &self.buffer,
            PREFIX_CANDIDATE_LIMIT,
        );
        // Stable sorting preserves each source's tie-breakers. An explicit
        // extension wins only when its match quality equals the pinyin result.
        let mut candidates = extensions
            .into_iter()
            .map(|item| PendingCandidate {
                text: item.entry.text.clone(),
                reading: None,
                quality: item.quality,
            })
            .collect::<Vec<_>>();
        candidates.extend(self.pinyin_candidates());
        candidates.sort_by_key(|candidate| candidate.quality);
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|candidate| seen.insert(candidate.text.clone()));

        self.candidate_readings.clear();
        self.candidates = candidates
            .into_iter()
            .map(|candidate| {
                let id = CandidateId(self.next_candidate_id);
                self.next_candidate_id = self.next_candidate_id.wrapping_add(1).max(1);
                if let Some(reading) = &candidate.reading {
                    self.candidate_readings.insert(id, reading.clone());
                }
                Candidate {
                    keep_open: false,
                    id,
                    text: candidate.text,
                    comment: None,
                    reading: candidate.reading.filter(|_| self.show_reading),
                }
            })
            .collect();
        self.selected = 0;
    }

    fn pinyin_candidates(&mut self) -> Vec<PendingCandidate> {
        let mut candidates = Vec::new();
        if let Ok(mut graph) = self.parser.parse(&self.buffer) {
            graph.constrain_closed_syllables(|id| {
                self.active_lexicon
                    .exact(&[id])
                    .iter()
                    .any(|entry| entry.text.chars().count() == 1)
            });
            let vocabulary = &self.profiles.current().vocabulary;
            // Suppression applies to output words, not their characters inside
            // other words or the existence checks used by quoted syllables.
            let limit = PREFIX_CANDIDATE_LIMIT.saturating_add(vocabulary.hidden_count());
            let learning = &self.profiles.current().learning;
            let now = learning::now();
            let epoch = now / 3600;
            self.decode_cache.set_epoch(epoch);
            let mut decoded = match self.decode_cache.get(&self.buffer, limit) {
                Some(candidates) => candidates,
                None => {
                    let decoded = if learning.words.is_empty() {
                        self.decoder.decode(&graph, &self.active_lexicon, limit)
                    } else {
                        self.decoder.decode_with_scorer(
                            &graph,
                            &self.active_lexicon,
                            limit,
                            &|prefix, next| learning.sentence_boost(prefix, next, epoch * 3600),
                        )
                    };
                    self.decode_cache.insert(&self.buffer, limit, &decoded);
                    decoded
                }
            };
            decoded.extend(learning.candidates(
                &self.buffer,
                &graph,
                self.parser.table(),
                &self.active_lexicon,
            ));
            decoded.extend(vocabulary.candidates(
                &graph,
                self.parser.table(),
                &self.active_lexicon,
            ));
            decoded.retain(|candidate| {
                !vocabulary.hidden(&candidate.text, &candidate.display_reading)
            });
            let mut ranked = decoded
                .into_iter()
                .map(|candidate| {
                    let score = candidate.score.saturating_add(learning.boost(
                        &self.buffer,
                        &candidate,
                        &self.history,
                        now,
                    ));
                    let pinned = vocabulary.pinned(&candidate.text, &candidate.display_reading);
                    let quality = candidate.match_quality();
                    (candidate, score, pinned, quality)
                })
                .collect::<Vec<_>>();
            ranked.sort_by(
                |(left, left_score, left_pin, left_quality),
                 (right, right_score, right_pin, right_quality)| {
                    left_quality
                        .cmp(right_quality)
                        .then_with(|| left.compare_match_kind(right))
                        .then_with(|| right_pin.cmp(left_pin))
                        .then_with(|| right_score.cmp(left_score))
                        .then_with(|| left.segments.cmp(&right.segments))
                        .then_with(|| left.text.cmp(&right.text))
                },
            );
            for (decoded, _, _, quality) in ranked {
                push_unique_candidate(
                    &mut candidates,
                    &decoded.text,
                    Some(decoded.display_reading),
                    quality,
                );
                if candidates.len() == PREFIX_CANDIDATE_LIMIT {
                    break;
                }
            }
        }

        candidates
    }

    fn preedit(&self) -> EngineOutput {
        EngineOutput {
            consumed: true,
            candidates_changed: true,
            ..EngineOutput::preedit(self.buffer.clone())
        }
    }

    fn clear(&mut self) {
        self.page_ends.clear();
        self.buffer.clear();
        self.candidates.clear();
        self.candidate_readings.clear();
        self.selected = 0;
    }

    fn take_candidate(&mut self, index: usize) -> Option<Commit> {
        let candidate = self.candidates.get(index)?;
        let text = candidate.text.clone();
        if let Some(reading) = self.candidate_readings.get(&candidate.id) {
            self.profiles.record(Selection {
                input: self.buffer.clone(),
                reading: reading.clone(),
                text: text.clone(),
                context: self.history.clone(),
                time: learning::now(),
            });
            if !self.history.is_empty() {
                self.decode_cache.clear();
            }
            self.history.push_str(&text);
            self.history = learning::suffix(&self.history, 4);
        } else {
            self.history.clear();
        }
        self.clear();
        Some(Commit { text })
    }

    fn take_candidate_by_id(&mut self, id: CandidateId) -> Option<Commit> {
        let index = self
            .candidates
            .iter()
            .position(|candidate| candidate.id == id)?;
        self.take_candidate(index)
    }

    fn commit_selected(&mut self) -> EngineOutput {
        if let Some(commit) = self.take_candidate(self.selected) {
            EngineOutput {
                consumed: true,
                commit: Some(commit),
                candidates_changed: true,
                ..EngineOutput::clear_preedit()
            }
        } else {
            self.history.clear();
            let raw = std::mem::take(&mut self.buffer);
            self.clear();
            EngineOutput::commit(raw)
        }
    }

    fn commit_selected_and_pass(&mut self) -> EngineOutput {
        let mut output = self.commit_selected();
        self.history.clear();
        output.consumed = false;
        output
    }

    fn commit_punctuation(&mut self, text: &str) -> EngineOutput {
        let mut output = if self.buffer.is_empty() {
            EngineOutput::commit(text)
        } else {
            // Record only the selected word, then append punctuation to the
            // same application commit so the original key is not forwarded.
            let mut output = self.commit_selected();
            if let Some(commit) = &mut output.commit {
                commit.text.push_str(text);
            }
            output
        };
        self.history.clear();
        self.punctuation.end_number();
        output.candidates_changed = true;
        output
    }

    fn move_selection(&mut self, target: usize) -> EngineOutput {
        if !self.candidates.is_empty() {
            self.selected = target.min(self.candidates.len() - 1);
        }
        EngineOutput {
            consumed: true,
            candidates_changed: true,
            ..EngineOutput::default()
        }
    }

    fn current_page(&self) -> std::ops::Range<usize> {
        mirix_engine_protocol::candidate_page(self.selected, self.candidates.len(), &self.page_ends)
    }

    fn turn_page(&mut self, forward: bool) -> EngineOutput {
        let page = self.current_page();
        let target = if forward {
            if page.end >= self.candidates.len() {
                return self.move_selection(self.selected);
            }
            page.end
        } else {
            mirix_engine_protocol::candidate_page(
                page.start.saturating_sub(1),
                self.candidates.len(),
                &self.page_ends,
            )
            .start
        };
        self.move_selection(target)
    }
}

fn push_unique_candidate(
    candidates: &mut Vec<PendingCandidate>,
    word: &str,
    reading: Option<String>,
    quality: MatchQuality,
) {
    if !candidates.iter().any(|candidate| candidate.text == word) {
        candidates.push(PendingCandidate {
            text: word.to_owned(),
            reading,
            quality,
        });
    }
}

impl Engine for PinyinEngine {
    fn name(&self) -> &str {
        "pinyin"
    }
    fn tray_icon(&self) -> Option<std::sync::Arc<mirix_engine_protocol::EngineIcon>> {
        Some(super::icon::get())
    }
    fn configuration_revision(&self) -> u64 {
        self.configuration_revision
            .wrapping_add(self.profiles.storage_revision())
    }

    fn configuration(
        &mut self,
        locale: &str,
    ) -> Result<mirix_engine_protocol::Configuration, String> {
        let page = super::configuration::describe(
            locale,
            &self.profiles,
            self.config_directory.is_some(),
            self.show_reading,
        )?;
        self.menu_label = super::configuration::localizer(locale)?.text("profile");
        Ok(page)
    }

    fn configure(
        &mut self,
        locale: &str,
        revision: u64,
        event: mirix_engine_protocol::ConfigurationEvent,
    ) -> Result<ActionResult<Option<String>>, String> {
        use mirix_engine_protocol::{ConfigurationEvent, ConfigurationValue};
        if revision != self.configuration_revision() {
            return Err("configuration changed; refresh before applying".into());
        }
        self.configuration(locale)?.validate_event(&event)?;
        let text = super::configuration::localizer(locale)?;
        let message = match event {
            ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::Bool(value),
            } if id == "show-reading" => {
                if self.show_reading != value {
                    if let Some(directory) = &self.config_directory {
                        mirix_config::update(&directory.join("config.toml"), |document| {
                            document["show_reading"] = mirix_config::value(value);
                            Ok(())
                        })
                        .map_err(|error| error.to_string())?;
                    }
                    self.show_reading = value;
                    for candidate in &mut self.candidates {
                        candidate.reading = self
                            .candidate_readings
                            .get(&candidate.id)
                            .filter(|_| value)
                            .cloned();
                    }
                    self.configuration_revision = self.configuration_revision.wrapping_add(1);
                }
                Ok(Some(text.text("reading-updated")))
            }
            ConfigurationEvent::Invoke { id } if id == "reload-dictionaries" => {
                self.reload_dictionaries()
                    .map_err(|error| error.to_string())?;
                Ok(Some(text.text("dictionaries-reloaded")))
            }
            ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::String(value),
            } if id == "profile-shortcut" => {
                self.profiles.set_menu_shortcut(value.parse()?);
                self.configuration_revision = self.configuration_revision.wrapping_add(1);
                Ok(Some(text.text("shortcut-updated")))
            }
            ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::String(value),
            } if matches!(id.as_str(), "rename-profile" | "duplicate-profile") => {
                let action = if id == "rename-profile" {
                    super::profiles::ProfileAction::Rename
                } else {
                    super::profiles::ProfileAction::Duplicate
                };
                let message = self.profiles.manage(action, &value)?;
                self.history.clear();
                self.rebuild_active_lexicon();
                self.refresh_candidates();
                self.configuration_revision = self.configuration_revision.wrapping_add(1);
                Ok(Some(message))
            }
            ConfigurationEvent::Invoke { id } if id == "archive-profile" => {
                let message = self
                    .profiles
                    .manage(super::profiles::ProfileAction::Archive, "")?;
                self.history.clear();
                self.rebuild_active_lexicon();
                self.refresh_candidates();
                self.configuration_revision = self.configuration_revision.wrapping_add(1);
                Ok(Some(message))
            }
            ConfigurationEvent::Invoke { id } if id == "retry-save" => {
                self.profiles.retry_save();
                Ok(Some(text.text("save-retry-queued")))
            }
            ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::String(value),
            } if id == "profile" => {
                self.activate_profile(&value)?;
                Ok(Some(format!("{}: {value}", text.text("profile-selected"))))
            }
            ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::String(value),
            } if id == "create-profile" => {
                let name = self.profiles.create(&value)?;
                self.activate_profile(&name)?;
                Ok(Some(format!("{}: {name}", text.text("profile-created"))))
            }
            ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::String(value),
            } if id == "punctuation" => {
                let mode = match value.as_str() {
                    "full" => PunctuationMode::Full,
                    "half" => PunctuationMode::Half,
                    _ => return Err("invalid punctuation mode".into()),
                };
                self.set_punctuation_mode(mode);
                Ok(Some(text.text("punctuation-updated")))
            }
            _ => Err("unknown configuration operation".to_owned()),
        }?;
        Ok(ActionResult::new(
            message,
            EngineOutput {
                candidates_changed: true,
                ..EngineOutput::default()
            },
        ))
    }
    fn vocabulary(
        &mut self,
        profile: Option<&str>,
        mut operation: mirix_engine_protocol::VocabularyOperation,
    ) -> Result<ActionResult<mirix_engine_protocol::VocabularyResult>, String> {
        use mirix_engine_protocol::{VocabularyOperation, VocabularyResult};
        if profile.is_some_and(|name| name != self.profiles.active) {
            return Err(format!(
                "current profile is {:?}, not {:?}",
                self.profiles.active,
                profile.unwrap()
            ));
        }
        if let VocabularyOperation::List {
            query,
            offset,
            limit,
        } = operation
        {
            let profile = self.profiles.current();
            let entries = profile.vocabulary.list(&profile.learning, &query);
            let total = entries.len();
            let offset = offset.min(total);
            return Ok(ActionResult::new(
                VocabularyResult::Words {
                    profile: self.profiles.active.clone(),
                    entries: entries
                        .into_iter()
                        .skip(offset)
                        .take(limit.clamp(1, 100))
                        .collect(),
                    total,
                    offset,
                },
                EngineOutput::default(),
            ));
        }
        let word = match &mut operation {
            VocabularyOperation::Add { word }
            | VocabularyOperation::Remove { word }
            | VocabularyOperation::Forget { word }
            | VocabularyOperation::Pin { word, .. }
            | VocabularyOperation::Hide { word, .. } => word,
            VocabularyOperation::List { .. } => unreachable!(),
        };
        *word = crate::vocabulary::normalize_word(word.clone(), self.parser.table())?;
        let changed = self.profiles.edit_vocabulary(operation);
        if changed {
            self.rebuild_active_lexicon();
            self.refresh_candidates();
        }
        Ok(ActionResult::new(
            VocabularyResult::Updated {
                profile: self.profiles.active.clone(),
                changed,
            },
            EngineOutput {
                candidates_changed: changed,
                ..EngineOutput::default()
            },
        ))
    }

    fn punctuation_mode(&self) -> Option<PunctuationMode> {
        Some(self.profiles.punctuation_mode)
    }

    fn menus(&self) -> Vec<EngineMenu> {
        vec![EngineMenu {
            id: "profiles".into(),
            label: self.menu_label.clone(),
            shortcut: self.profiles.menu_shortcut,
            entries: self
                .profiles
                .entries
                .keys()
                .map(|name| MenuEntry {
                    id: name.clone(),
                    label: name.clone(),
                })
                .collect(),
            active: Some(self.profiles.active.clone()),
        }]
    }
    fn select_menu(&mut self, menu: &str, entry: &str) -> Result<EngineOutput, String> {
        if menu != "profiles" {
            return Err("unknown menu".into());
        }
        if self.profiles.active == entry {
            return Ok(EngineOutput::consume());
        }
        self.activate_profile(entry)?;
        Ok(self.preedit())
    }

    fn process_key(&mut self, ev: &KeyEvent) -> EngineOutput {
        let is_period = ev.keysym == u32::from('.') || ev.text.as_deref() == Some(".");
        if self.punctuation_switch_key.is_some()
            && (is_period || (ev.keycode != 0 && self.punctuation_switch_key == Some(ev.keycode)))
        {
            if !ev.is_press() {
                self.punctuation_switch_key = None;
            }
            return EngineOutput::consume();
        }
        if !ev.is_press() {
            return EngineOutput::pass();
        }
        // Modifier presses alone must not cancel the pending composition.
        if matches!(ev.keysym, 0xffe1..=0xffee) {
            return EngineOutput::pass();
        }
        if is_period && ev.mods.ctrl && !ev.mods.shift && !ev.mods.alt && !ev.mods.logo {
            self.punctuation_switch_key = Some(ev.keycode);
            self.set_punctuation_mode(match self.profiles.punctuation_mode {
                PunctuationMode::Full => PunctuationMode::Half,
                PunctuationMode::Half => PunctuationMode::Full,
            });
            return EngineOutput {
                consumed: true,
                candidates_changed: true,
                ..EngineOutput::default()
            };
        }
        if ev.mods.has_ctrl_alt_or_logo() {
            self.history.clear();
            self.punctuation = Punctuation::default();
            if self.buffer.is_empty() {
                return EngineOutput::pass();
            }
            self.clear();
            return EngineOutput::clear_preedit_and_pass();
        }

        if self.buffer.is_empty() {
            if let Some(text) = self
                .punctuation
                .translate(ev, self.profiles.punctuation_mode)
            {
                return self.commit_punctuation(&text);
            }
            if let Some(letter) = input_letter(ev) {
                self.buffer.push(letter);
                self.refresh_candidates();
                return self.preedit();
            }
            self.history.clear();
            return EngineOutput::pass();
        }

        match ev.keysym {
            keysym::ESCAPE => {
                self.history.clear();
                self.punctuation = Punctuation::default();
                self.clear();
                return EngineOutput::clear_preedit();
            }
            keysym::BACKSPACE => {
                self.buffer.pop();
                self.refresh_candidates();
                return if self.buffer.is_empty() {
                    EngineOutput::clear_preedit()
                } else {
                    self.preedit()
                };
            }
            keysym::SPACE => return self.commit_selected(),
            keysym::RETURN => {
                self.history.clear();
                let raw = std::mem::take(&mut self.buffer);
                self.clear();
                return EngineOutput::commit(raw);
            }
            keysym::LEFT | keysym::UP => {
                return self.move_selection(self.selected.saturating_sub(1));
            }
            keysym::RIGHT | keysym::DOWN => {
                return self.move_selection(self.selected.saturating_add(1));
            }
            keysym::PAGE_UP => {
                return self.turn_page(false);
            }
            keysym::PAGE_DOWN => {
                return self.turn_page(true);
            }
            keysym::HOME => return self.move_selection(0),
            keysym::END => return self.move_selection(self.candidates.len().saturating_sub(1)),
            _ => {}
        }

        if ev.text.as_deref() == Some("-") {
            return self.turn_page(false);
        }
        if ev.text.as_deref() == Some("=") {
            return self.turn_page(true);
        }

        if let Some(index) = selection_index(ev) {
            let page = self.current_page();
            if index < page.len()
                && let Some(commit) = self.take_candidate(page.start + index)
            {
                return EngineOutput {
                    consumed: true,
                    commit: Some(commit),
                    candidates_changed: true,
                    ..EngineOutput::clear_preedit()
                };
            }
            return EngineOutput::consume();
        }

        if let Some(letter) = input_letter(ev) {
            self.punctuation.end_number();
            self.buffer.push(letter);
            self.refresh_candidates();
            return self.preedit();
        }
        if is_apostrophe(ev) {
            if !self.buffer.ends_with('\'') {
                self.buffer.push('\'');
                self.refresh_candidates();
            }
            return self.preedit();
        }

        if let Some(text) = self
            .punctuation
            .translate(ev, self.profiles.punctuation_mode)
        {
            return self.commit_punctuation(&text);
        }
        self.commit_selected_and_pass()
    }

    fn reset(&mut self) {
        self.punctuation_switch_key = None;
        self.history.clear();
        self.punctuation = Punctuation::default();
        self.clear();
    }

    fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    fn selected_candidate(&self) -> usize {
        self.selected
    }

    fn set_candidate_pages(&mut self, ends: Vec<usize>) {
        if mirix_engine_protocol::valid_candidate_pages(self.candidates.len(), &ends) {
            self.page_ends = ends;
        }
    }

    fn candidate_page_ends(&self) -> &[usize] {
        &self.page_ends
    }

    fn select(&mut self, id: CandidateId) -> EngineOutput {
        match self.take_candidate_by_id(id) {
            Some(commit) => EngineOutput {
                consumed: true,
                commit: Some(commit),
                candidates_changed: true,
                ..EngineOutput::clear_preedit()
            },
            None => EngineOutput::consume(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reading_switch_preserves_candidates_and_learning_and_is_saved() {
        use mirix_engine_protocol::{ConfigurationEvent, ConfigurationValue};
        let directory = tempfile::tempdir().unwrap();
        let mut engine = test_engine();
        engine.config_directory = Some(directory.path().into());
        type_text(&mut engine, "ni");
        let ids = engine
            .candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();
        assert!(engine.candidates[0].reading.is_some());
        engine
            .configure(
                "en-US",
                engine.configuration_revision(),
                ConfigurationEvent::Change {
                    id: "show-reading".into(),
                    value: ConfigurationValue::Bool(false),
                },
            )
            .unwrap();
        assert!(
            engine
                .candidates
                .iter()
                .all(|candidate| candidate.reading.is_none())
        );
        assert_eq!(
            ids,
            engine
                .candidates
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>()
        );
        engine.process_key(&press_sym(keysym::SPACE));
        assert_eq!(engine.profiles.current().learning.words["ni"][0].text, "你");

        let mut reopened = PinyinEngine::from_config(directory.path()).unwrap();
        type_text(&mut reopened, "ni");
        assert!(!reopened.candidates.is_empty());
        assert!(
            reopened
                .candidates
                .iter()
                .all(|candidate| candidate.reading.is_none())
        );
        let id = reopened.candidates[0].id;
        reopened
            .configure(
                "zh-CN",
                reopened.configuration_revision(),
                ConfigurationEvent::Change {
                    id: "show-reading".into(),
                    value: ConfigurationValue::Bool(true),
                },
            )
            .unwrap();
        assert_eq!(id, reopened.candidates[0].id);
        assert!(reopened.candidates[0].reading.is_some());
    }
    use super::*;
    use mirix_engine_protocol::CANDIDATE_PAGE_SIZE as PAGE_SIZE;
    use mirix_engine_protocol::{KeyState, Modifiers};
    use mirix_pinyin_core::normalize_display_syllable;

    fn test_engine() -> PinyinEngine {
        let mut engine = PinyinEngine::empty();
        let issues = engine.extend_character_dictionary(crate::characters::DEFAULT_CHARACTERS);
        assert!(
            issues.is_empty(),
            "invalid default character dictionary: {issues:?}"
        );
        engine
    }

    fn press_text(text: &str) -> KeyEvent {
        KeyEvent {
            keycode: 0,
            keysym: text.chars().next().unwrap() as u32,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: Some(text.into()),
        }
    }

    fn press_sym(sym: u32) -> KeyEvent {
        KeyEvent {
            keycode: 0,
            keysym: sym,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: None,
        }
    }

    fn type_text(engine: &mut PinyinEngine, text: &str) {
        for character in text.chars() {
            engine.process_key(&press_text(&character.to_string()));
        }
    }

    #[test]
    fn produces_and_commits_candidates() {
        let mut engine = test_engine();
        assert!(
            engine
                .extend_dictionary("nihao\tnǐ hǎo\t你好\t1000000\n")
                .is_empty()
        );
        type_text(&mut engine, "nihao");
        assert_eq!(engine.candidates()[0].text, "你好");
        assert_eq!(
            engine
                .process_key(&press_sym(keysym::SPACE))
                .commit
                .unwrap()
                .text,
            "你好"
        );
        assert!(engine.candidates().is_empty());
    }

    #[test]
    fn composes_continuous_sentence_input() {
        let mut engine = test_engine();
        assert!(
            engine
                .extend_dictionary(
                    "nihao\tnǐ hǎo\t你好\t1000000\nma\tma\t吗\t1000000\nwo\twǒ\t我\t1000000\njiao\tjiào\t叫\t1000000\n"
                )
                .is_empty()
        );
        type_text(&mut engine, "nihaomawojiao");
        assert!(
            engine
                .candidates()
                .iter()
                .any(|candidate| candidate.text == "你好吗我叫"),
            "candidates: {:?}",
            engine.candidates()
        );
    }

    #[test]
    fn default_character_template_is_valid_and_populated() {
        let engine = test_engine();
        assert!(engine.core_lexicon.len() > 2_400);
    }

    #[test]
    fn reopening_engine_uses_edited_character_file_without_merging_template() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let data = root.path().join("data");
        // The production startup path creates the default file on first use.
        drop(PinyinEngine::open(&data, &config).unwrap());
        let path = config.join("characters.tsv");
        assert!(path.is_file());
        std::fs::write(&path, "闺 guī gui 38845\n").unwrap();
        {
            let mut engine = PinyinEngine::open(&data, &config).unwrap();
            type_text(&mut engine, "gui");
            assert_eq!(engine.candidates().len(), 1);
            assert_eq!(engine.candidates()[0].text, "闺");
        }
        std::fs::write(&path, "贵\tguì\tgui\t42126\n").unwrap();
        let mut engine = PinyinEngine::open(&data, &config).unwrap();
        type_text(&mut engine, "gui");
        assert_eq!(engine.candidates().len(), 1);
        assert_eq!(engine.candidates()[0].text, "贵");
    }

    #[test]
    fn incomplete_spelling_uses_generic_prefix_lookup() {
        for input in ["d", "chux", "zhez"] {
            let mut engine = test_engine();
            type_text(&mut engine, input);
            assert!(
                engine.candidates().iter().any(|candidate| candidate
                    .reading
                    .as_deref()
                    .is_some_and(|reading| reading != input)),
                "no completion generated for {input:?}"
            );
        }
    }

    #[test]
    fn complete_fan_reading_precedes_partial_fa_n_readings() {
        let mut engine = test_engine();
        type_text(&mut engine, "fan");

        let first_reading = engine.candidates()[0]
            .reading
            .as_deref()
            .expect("candidate should include its complete reading");
        assert_eq!(
            normalize_display_syllable(first_reading).as_deref(),
            Some("fan")
        );
    }

    #[test]
    fn all_builtin_whole_syllables_list_exact_characters_before_combinations() {
        use std::collections::HashSet;
        let mut engine = test_engine();
        let cases = engine
            .parser
            .table()
            .iter()
            .map(|(id, input)| {
                let characters = engine
                    .core_lexicon
                    .exact(&[id])
                    .iter()
                    .filter(|entry| entry.text.chars().count() == 1)
                    .map(|entry| entry.text.as_str())
                    .collect::<HashSet<_>>();
                (
                    input.to_owned(),
                    characters.len().min(PREFIX_CANDIDATE_LIMIT),
                )
            })
            .collect::<Vec<_>>();
        for (input, exact_count) in cases {
            if exact_count == 0 {
                continue;
            }
            engine.buffer = input.clone();
            engine.refresh_candidates();
            assert!(
                engine.candidates.len() >= exact_count,
                "missing exact characters for {input}"
            );
            for candidate in &engine.candidates[..exact_count] {
                assert_eq!(candidate.text.chars().count(), 1, "{input}: {candidate:?}");
                assert_eq!(
                    candidate
                        .reading
                        .as_deref()
                        .and_then(normalize_display_syllable)
                        .as_deref(),
                    Some(input.as_str()),
                    "{input}: {candidate:?}"
                );
            }
            let unique = engine
                .candidates
                .iter()
                .map(|candidate| &candidate.text)
                .collect::<HashSet<_>>();
            assert_eq!(
                unique.len(),
                engine.candidates.len(),
                "duplicate candidates for {input}"
            );
        }
    }

    #[test]
    fn enter_commits_raw_spelling() {
        let mut engine = test_engine();
        type_text(&mut engine, "ni");
        assert_eq!(
            engine
                .process_key(&press_sym(keysym::RETURN))
                .commit
                .unwrap()
                .text,
            "ni"
        );
    }

    #[test]
    fn number_and_arrow_keys_select_candidates() {
        let mut engine = test_engine();
        type_text(&mut engine, "shi");
        let second = engine.candidates()[1].text.clone();
        engine.process_key(&press_sym(keysym::RIGHT));
        assert_eq!(engine.selected_candidate(), 1);
        assert_eq!(
            engine.process_key(&press_text("2")).commit.unwrap().text,
            second
        );
    }

    #[test]
    fn stale_candidate_id_cannot_select_a_refreshed_list() {
        let mut engine = test_engine();
        type_text(&mut engine, "ni");
        let stale = engine.candidates()[0].id;

        type_text(&mut engine, "h");
        let output = engine.select(stale);
        assert!(output.commit.is_none());
        assert!(!engine.candidates().is_empty());
    }

    #[test]
    fn complete_single_syllables_prefer_single_characters() {
        for spelling in ["xuan", "huai", "huan", "xian", "fan"] {
            let mut engine = test_engine();
            type_text(&mut engine, spelling);
            let first = &engine.candidates()[0];
            assert_eq!(first.text.chars().count(), 1, "{spelling}: {first:?}");
            assert_eq!(
                normalize_display_syllable(first.reading.as_deref().unwrap()).as_deref(),
                Some(spelling),
            );
        }
    }

    #[test]
    fn number_one_selects_the_first_candidate_on_the_second_page() {
        let mut engine = test_engine();
        type_text(&mut engine, "d");
        let expected = engine.candidates()[PAGE_SIZE].text.clone();
        engine.process_key(&press_text("="));
        assert_eq!(
            engine.process_key(&press_text("1")).commit.unwrap().text,
            expected
        );
    }

    #[test]
    fn measured_pages_control_both_navigation_and_number_selection() {
        let mut engine = test_engine();
        type_text(&mut engine, "d");
        let count = engine.candidates().len();
        let expected = engine.candidates()[3].text.clone();
        let mut ends = vec![3];
        let mut end = 3;
        while end < count {
            end = (end + 5).min(count);
            ends.push(end);
        }
        engine.set_candidate_pages(ends);
        assert!(engine.process_key(&press_text("4")).commit.is_none());
        engine.process_key(&press_text("="));
        assert_eq!(engine.selected_candidate(), 3);
        engine.process_key(&press_text("-"));
        assert_eq!(engine.selected_candidate(), 0);
        engine.process_key(&press_sym(keysym::PAGE_DOWN));
        assert_eq!(
            engine.process_key(&press_text("1")).commit.unwrap().text,
            expected
        );
    }

    #[test]
    fn minus_and_equal_switch_candidate_pages() {
        let mut engine = test_engine();
        type_text(&mut engine, "d");
        assert!(engine.candidates().len() > PAGE_SIZE);

        engine.process_key(&press_text("="));
        assert_eq!(engine.selected_candidate(), PAGE_SIZE);
        engine.process_key(&press_text("-"));
        assert_eq!(engine.selected_candidate(), 0);
    }

    #[test]
    fn backspace_escape_and_shortcut_clear_composition() {
        let mut engine = test_engine();
        type_text(&mut engine, "ni");
        engine.process_key(&press_sym(keysym::BACKSPACE));
        assert_eq!(engine.buffer, "n");
        engine.process_key(&press_sym(keysym::ESCAPE));
        assert!(engine.buffer.is_empty());

        type_text(&mut engine, "hao");
        let output = engine.process_key(&KeyEvent {
            mods: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            ..press_text("c")
        });
        assert!(!output.consumed);
        assert_eq!(output.preedit_text(), Some(""));
    }

    #[test]
    fn custom_dictionary_overrides_frequency_and_accepts_apostrophe() {
        let mut engine = test_engine();
        assert!(
            engine
                .extend_dictionary("xi'an\txī ān\t西安\t999999\n")
                .is_empty()
        );
        type_text(&mut engine, "xi");
        engine.process_key(&press_text("'"));
        type_text(&mut engine, "an");
        assert_eq!(engine.candidates()[0].text, "西安");
        assert_eq!(engine.candidates()[0].reading.as_deref(), Some("xī ān"));
    }

    #[test]
    fn custom_dictionary_completes_the_editable_tail_syllable() {
        let mut engine = test_engine();
        assert!(
            engine
                .extend_dictionary("ceshici\tcè shì cí\t测试词\t999999\n")
                .is_empty()
        );
        type_text(&mut engine, "ceshic");
        assert!(engine.candidates().iter().any(|candidate| {
            candidate.text == "测试词" && candidate.reading.as_deref() == Some("cè shì cí")
        }));
    }

    #[test]
    fn character_dictionary_adds_complete_display_pinyin() {
        let mut engine = test_engine();
        let issues = engine.extend_character_dictionary("的\tdē\tde\t1000\n地\tdì\tdi\t1000\n");
        assert!(issues.is_empty());

        type_text(&mut engine, "d");
        assert!(engine.candidates().iter().any(|candidate| {
            candidate.text == "地" && candidate.reading.as_deref() == Some("dì")
        }));
    }

    #[test]
    fn punctuation_commits_selected_word_and_symbol_together() {
        let mut engine = test_engine();
        assert!(engine.extend_dictionary("wo\twǒ\t我\t10000\n").is_empty());
        type_text(&mut engine, "wo");
        let output = engine.process_key(&press_text("."));
        assert!(output.consumed);
        assert_eq!(output.commit.unwrap().text, "我。");
        assert!(output.candidates_changed);
        assert!(engine.candidates().is_empty());
        assert!(engine.buffer.is_empty());
        assert!(engine.history.is_empty());
        assert_eq!(engine.profiles.current().learning.words["wo"][0].text, "我");
    }

    fn profile_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for name in ["日常", "编程"] {
            std::fs::create_dir_all(
                root.path()
                    .join("config/profiles")
                    .join(name)
                    .join("dictionaries"),
            )
            .unwrap();
        }
        root
    }

    fn choose(engine: &mut PinyinEngine, text: &str) {
        let id = engine
            .candidates()
            .iter()
            .find(|candidate| candidate.text == text)
            .unwrap()
            .id;
        assert_eq!(engine.select(id).commit.unwrap().text, text);
    }

    #[test]
    fn learning_is_isolated_persisted_and_recalled_before_truncation() {
        let root = profile_root();
        let mut engine =
            PinyinEngine::open(root.path().join("data"), root.path().join("config")).unwrap();
        engine.activate_profile("编程").unwrap();
        type_text(&mut engine, "ni");
        choose(&mut engine, "泥");
        type_text(&mut engine, "ni");
        assert_eq!(engine.candidates()[0].text, "泥");
        let stale = engine.candidates()[0].id;
        engine.activate_profile("日常").unwrap();
        assert_eq!(engine.buffer, "ni");
        assert_eq!(engine.selected_candidate(), 0);
        assert_eq!(engine.candidates()[0].text, "你");
        assert!(engine.select(stale).commit.is_none());
        engine.activate_profile("编程").unwrap();
        assert_eq!(engine.candidates()[0].text, "泥");
        drop(engine);

        let mut restored =
            PinyinEngine::open(root.path().join("data"), root.path().join("config")).unwrap();
        assert_eq!(restored.profiles().active.as_deref(), Some("编程"));
        // Removing ordinary recall proves the learned candidate is independently recalled.
        restored.core_lexicon = Lexicon::default();
        restored.rebuild_active_lexicon();
        type_text(&mut restored, "ni");
        assert_eq!(restored.candidates()[0].text, "泥");
        assert_eq!(
            restored.profiles.current().learning.words["ni"][0].inputs["ni"].count,
            1
        );
    }

    #[test]
    fn profile_dictionaries_switch_without_discarding_input() {
        let root = profile_root();
        std::fs::write(
            root.path()
                .join("config/profiles/编程/dictionaries/terms.tsv"),
            "ceshici\tcè shì cí\t测试词\t999999\n",
        )
        .unwrap();
        let mut engine =
            PinyinEngine::open(root.path().join("data"), root.path().join("config")).unwrap();
        // Keep this isolation test independent of words added to the shared daily corpus.
        engine.core_lexicon = Lexicon::default();
        engine.rebuild_active_lexicon();
        type_text(&mut engine, "ceshici");
        assert!(
            !engine
                .candidates()
                .iter()
                .any(|candidate| candidate.text == "测试词")
        );
        engine.activate_profile("编程").unwrap();
        assert_eq!(engine.buffer, "ceshici");
        assert_eq!(engine.candidates()[0].text, "测试词");
        let candidates = engine.candidates().to_vec();
        engine.activate_profile("编程").unwrap();
        assert_eq!(engine.candidates(), candidates);
        assert!(engine.activate_profile("不存在").is_err());
        assert_eq!(engine.candidates(), candidates);
        engine.activate_profile("日常").unwrap();
        assert!(
            !engine
                .candidates()
                .iter()
                .any(|candidate| candidate.text == "测试词")
        );
    }

    #[test]
    fn repeated_context_can_outweigh_general_personal_frequency() {
        let mut engine = test_engine();
        for _ in 0..5 {
            engine.reset();
            type_text(&mut engine, "ni");
            choose(&mut engine, "你");
        }
        for _ in 0..2 {
            engine.reset();
            type_text(&mut engine, "wo");
            choose(&mut engine, "我");
            type_text(&mut engine, "ni");
            choose(&mut engine, "泥");
        }
        engine.reset();
        type_text(&mut engine, "ni");
        assert_eq!(engine.candidates()[0].text, "你");
        engine.reset();
        type_text(&mut engine, "wo");
        choose(&mut engine, "我");
        type_text(&mut engine, "ni");
        assert_eq!(engine.candidates()[0].text, "泥");
        engine.reset();
        assert!(engine.history.is_empty());
    }

    #[test]
    fn only_actual_selection_learns_and_keyboard_mouse_share_counts() {
        let mut engine = test_engine();
        type_text(&mut engine, "ni");
        engine.process_key(&press_sym(keysym::ESCAPE));
        type_text(&mut engine, "ni");
        engine.process_key(&press_sym(keysym::RETURN));
        type_text(&mut engine, "ni");
        engine.reset();
        assert!(engine.profiles.current().learning.words.is_empty());
        type_text(&mut engine, "ni");
        let id = engine.candidates()[0].id;
        engine.process_key(&press_sym(keysym::SPACE));
        assert!(engine.select(id).commit.is_none());
        type_text(&mut engine, "ni");
        choose(&mut engine, "你");
        let word = &engine.profiles.current().learning.words["ni"][0];
        assert_eq!(word.inputs["ni"].count, 2);
        engine.process_key(&press_sym(keysym::LEFT));
        assert!(engine.history.is_empty());
    }

    #[test]
    fn closed_chunks_override_learned_splits_and_reopen_after_backspace() {
        let mut engine = test_engine();
        assert!(
            engine
                .extend_dictionary("xuanan\txǔ ān ān\t许安安\t1000000\n")
                .is_empty()
        );
        type_text(&mut engine, "xuanan");
        choose(&mut engine, "许安安");
        type_text(&mut engine, "xuan'an'");
        assert!(!engine.candidates().is_empty());
        for candidate in engine.candidates() {
            let reading = candidate
                .reading
                .as_ref()
                .unwrap()
                .split_whitespace()
                .map(|part| normalize_display_syllable(part).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(reading, ["xuan", "an"]);
            assert_eq!(candidate.text.chars().count(), 2);
        }
        engine.reset();
        type_text(&mut engine, "xuan'");
        assert!(
            engine
                .candidates()
                .iter()
                .all(|candidate| candidate.text.chars().count() == 1)
        );
        engine.process_key(&press_sym(keysym::BACKSPACE));
        assert_eq!(engine.buffer, "xuan");
        let graph = engine.parser.parse(&engine.buffer).unwrap();
        assert!(!graph.requires_single_character(0));
    }

    #[test]
    fn external_shared_words_apply_to_every_profile_without_touching_source_files() {
        let root = profile_root();
        let config = root.path().join("config");
        let dictionaries = config.join("dictionaries");
        std::fs::create_dir_all(&dictionaries).unwrap();
        let source = "shijie\tshì jiè\t世界\t1\nyinyue\tyīn yuè\t音乐\t1\n";
        std::fs::write(dictionaries.join("words.tsv"), source).unwrap();
        let mut engine = PinyinEngine::open(root.path().join("data"), &config).unwrap();
        for profile in ["日常", "编程"] {
            engine.activate_profile(profile).unwrap();
            for (input, word) in [("shijie", "世界"), ("yinyue", "音乐")] {
                engine.reset();
                type_text(&mut engine, input);
                assert_eq!(engine.candidates()[0].text, word);
            }
        }
        choose(&mut engine, "音乐");
        drop(engine);
        assert_eq!(
            std::fs::read_to_string(dictionaries.join("words.tsv")).unwrap(),
            source
        );
        assert!(
            root.path()
                .join("data/profiles/编程/learning.json")
                .is_file()
        );
        assert!(!config.join("profiles/编程/learning.json").exists());
    }

    #[test]
    fn frame_character_can_be_selected_and_learned() {
        let mut engine = test_engine();
        type_text(&mut engine, "zhen");
        let frame = engine
            .candidates()
            .iter()
            .find(|candidate| candidate.text == "帧")
            .unwrap();
        assert_eq!(frame.reading.as_deref(), Some("zhēn"));
        choose(&mut engine, "帧");
        type_text(&mut engine, "zhen");
        assert_ne!(engine.candidates()[0].text, "帧");
        for _ in 0..10 {
            choose(&mut engine, "帧");
            engine.reset();
            type_text(&mut engine, "zhen");
        }
        assert_eq!(engine.candidates()[0].text, "帧");
    }

    #[test]
    fn chinese_punctuation_works_without_composition() {
        let mut engine = test_engine();
        for (input, expected) in [
            (",", "，"),
            (".", "。"),
            ("!", "！"),
            ("?", "？"),
            (":", "："),
            (";", "；"),
            ("(", "（"),
            (")", "）"),
            ("[", "【"),
            ("]", "】"),
            ("<", "《"),
            (">", "》"),
            ("/", "、"),
            ("\\", "、"),
            ("^", "……"),
            ("_", "——"),
            ("~", "～"),
            ("$", "￥"),
        ] {
            let output = engine.process_key(&press_text(input));
            assert!(output.consumed, "{input}");
            assert_eq!(output.commit.unwrap().text, expected);
        }
        assert!(engine.profiles.current().learning.words.is_empty());
        assert_eq!(
            engine
                .process_key(&press_sym(b',' as u32))
                .commit
                .unwrap()
                .text,
            "，"
        );
    }

    #[test]
    fn shift_does_not_commit_before_shifted_punctuation() {
        let mut engine = test_engine();
        type_text(&mut engine, "ni");
        let selected = engine.candidates()[0].text.clone();
        let mut shift = press_sym(0xffe1);
        shift.mods.shift = true;
        let output = engine.process_key(&shift);
        assert!(!output.consumed);
        assert!(output.commit.is_none());
        assert_eq!(engine.buffer, "ni");
        let mut exclamation = press_text("!");
        exclamation.mods.shift = true;
        assert_eq!(
            engine.process_key(&exclamation).commit.unwrap().text,
            format!("{selected}！")
        );
    }

    #[test]
    fn paired_quotes_survive_word_selection_and_apostrophes_still_segment() {
        let mut engine = test_engine();
        assert_eq!(
            engine.process_key(&press_text("\"")).commit.unwrap().text,
            "“"
        );
        assert_eq!(
            engine.process_key(&press_text("'")).commit.unwrap().text,
            "‘"
        );
        type_text(&mut engine, "xi'an");
        assert_eq!(engine.buffer, "xi'an");
        engine.process_key(&press_sym(keysym::SPACE));
        assert_eq!(
            engine.process_key(&press_text("'")).commit.unwrap().text,
            "’"
        );
        assert_eq!(
            engine.process_key(&press_text("\"")).commit.unwrap().text,
            "”"
        );
        engine.process_key(&press_text("\""));
        engine.reset();
        assert_eq!(
            engine.process_key(&press_text("\"")).commit.unwrap().text,
            "“"
        );
    }

    #[test]
    fn numeric_dot_passes_but_cursor_movement_ends_numeric_context() {
        let mut engine = test_engine();
        for text in ["3", ".", "1", "4"] {
            let output = engine.process_key(&press_text(text));
            assert!(!output.consumed);
            assert!(output.commit.is_none());
        }
        engine.process_key(&press_sym(keysym::LEFT));
        assert_eq!(
            engine.process_key(&press_text(".")).commit.unwrap().text,
            "。"
        );
        engine.process_key(&press_text("3"));
        engine.reset();
        assert_eq!(
            engine.process_key(&press_text(".")).commit.unwrap().text,
            "。"
        );
    }

    #[test]
    fn shortcuts_and_releases_do_not_insert_punctuation_or_advance_quotes() {
        let mut engine = test_engine();
        for mods in [
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
            Modifiers {
                alt: true,
                ..Default::default()
            },
            Modifiers {
                logo: true,
                ..Default::default()
            },
        ] {
            let mut event = press_text("\"");
            event.mods = mods;
            assert_eq!(engine.process_key(&event), EngineOutput::pass());
        }
        let mut release = press_text("\"");
        release.state = KeyState::Release;
        assert_eq!(engine.process_key(&release), EngineOutput::pass());
        assert_eq!(
            engine.process_key(&press_text("\"")).commit.unwrap().text,
            "“"
        );
        assert_eq!(engine.process_key(&release), EngineOutput::pass());
        assert_eq!(
            engine.process_key(&press_text("\"")).commit.unwrap().text,
            "”"
        );
    }

    #[test]
    fn learning_uses_internal_reading_instead_of_display_comment() {
        let mut engine = test_engine();
        type_text(&mut engine, "ni");
        engine.candidates[0].reading = Some("display-only annotation".into());
        engine.process_key(&press_sym(keysym::SPACE));
        assert_eq!(engine.profiles.current().learning.words["ni"][0].text, "你");
        assert!(engine.candidate_readings.is_empty());
        assert_eq!(engine.history, "你");
    }

    #[test]
    fn abbreviated_selection_is_learned_by_full_reading_and_actual_input() {
        let mut engine = PinyinEngine::empty();
        assert!(
            engine
                .extend_dictionary("nihao\tnǐ hǎo\t你好\t10\nnihao\tní hào\t泥号\t1\n")
                .is_empty()
        );
        type_text(&mut engine, "nh");
        choose(&mut engine, "泥号");
        let word = &engine.profiles.current().learning.words["nihao"][0];
        assert_eq!(word.reading, "ní hào");
        assert_eq!(word.inputs["nh"].count, 1);
        // Personal recall must not require the phrase to remain in a dictionary.
        engine.core_lexicon = Lexicon::default();
        engine.rebuild_active_lexicon();
        type_text(&mut engine, "nh");
        assert_eq!(engine.candidates[0].text, "泥号");
        assert_eq!(engine.candidates[0].reading.as_deref(), Some("ní hào"));
        engine.reset();
        type_text(&mut engine, "nhao");
        assert_eq!(engine.candidates[0].text, "泥号");
    }

    #[test]
    fn cached_dictionary_results_do_not_freeze_learning_or_candidate_ids() {
        let mut engine = PinyinEngine::empty();
        assert!(
            engine
                .extend_dictionary("ni\tnǐ\t你\t100\nni\tní\t尼\t1\n")
                .is_empty()
        );
        engine.buffer = "ni".into();
        engine.refresh_candidates();
        assert_eq!(engine.candidates[0].text, "你");
        let original_ids = engine
            .candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();
        let selected = engine
            .candidates
            .iter()
            .find(|candidate| candidate.text == "尼")
            .unwrap()
            .id;
        engine.select(selected);
        assert!(engine.decode_cache.get("ni", 72).is_some());
        engine.buffer = "ni".into();
        engine.refresh_candidates();
        // A single rare selection must not override a much stronger prior.
        assert_eq!(engine.candidates[0].text, "你");
        for _ in 0..10 {
            choose(&mut engine, "尼");
            engine.reset();
            engine.buffer = "ni".into();
            engine.refresh_candidates();
        }
        assert_eq!(engine.candidates[0].text, "尼");
        assert!(
            engine
                .candidates
                .iter()
                .all(|candidate| !original_ids.contains(&candidate.id))
        );
        assert!(engine.select(selected).commit.is_none());
        assert!(engine.extend_dictionary("ni\tnǐ\t拟\t10\n").is_empty());
        assert!(engine.decode_cache.get("ni", 72).is_none());
        engine.refresh_candidates();
        assert!(
            engine
                .candidates
                .iter()
                .any(|candidate| candidate.text == "拟")
        );
    }

    #[test]
    fn repeated_word_learning_transfers_between_input_spellings() {
        let mut engine = PinyinEngine::empty();
        assert!(
            engine
                .extend_dictionary("shijie\tshì jiè\t世界\t100\nshijie\tshì jiè\t视界\t1\n")
                .is_empty()
        );
        for _ in 0..20 {
            engine.reset();
            type_text(&mut engine, "shijie");
            choose(&mut engine, "视界");
        }
        for input in ["sj", "shjie", "shi'jie", "shijie"] {
            engine.reset();
            type_text(&mut engine, input);
            assert_eq!(engine.candidates()[0].text, "视界", "{input}");
        }
        // Display accents are not a separate learning identity.
        assert!(
            engine
                .extend_dictionary("shijie\tshi jie\t视界\t2\n")
                .is_empty()
        );
        engine.reset();
        type_text(&mut engine, "sj");
        assert_eq!(engine.candidates()[0].text, "视界");
    }

    #[test]
    fn contextual_evidence_is_shared_without_overriding_the_general_prior() {
        let mut engine = PinyinEngine::empty();
        assert!(
            engine
                .extend_dictionary(
                    "wo\twǒ\t我\t100\nshijie\tshì jiè\t世界\t100\nshijie\tshì jiè\t视界\t1\n"
                )
                .is_empty()
        );
        for input in ["shijie", "sj", "shi'jie"] {
            engine.reset();
            type_text(&mut engine, "wo");
            choose(&mut engine, "我");
            type_text(&mut engine, input);
            choose(&mut engine, "视界");
        }
        engine.reset();
        type_text(&mut engine, "shjie");
        assert_eq!(engine.candidates()[0].text, "世界");
        engine.reset();
        type_text(&mut engine, "wo");
        choose(&mut engine, "我");
        type_text(&mut engine, "shjie");
        assert_eq!(engine.candidates()[0].text, "视界");
    }

    #[test]
    fn learned_transitions_generalize_to_unseen_sentences_and_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let data = root.path().join("data");
        for profile in ["日常", "编程"] {
            std::fs::create_dir_all(config.join("profiles").join(profile)).unwrap();
        }
        std::fs::create_dir_all(config.join("dictionaries")).unwrap();
        std::fs::write(
            config.join("characters.tsv"),
            "我 wǒ wo 100\n在 zài zai 100\n些 xiē xie 100\n写 xiě xie 20\n",
        )
        .unwrap();
        std::fs::write(
            config.join("dictionaries/words.tsv"),
            "chengxu\tchéng xù\t程序\t100\n",
        )
        .unwrap();
        let mut engine = PinyinEngine::open(&data, &config).unwrap();
        type_text(&mut engine, "wozaixiechengxu");
        assert_eq!(engine.candidates()[0].text, "我在些程序");
        assert!(engine.decode_cache.get("wozaixiechengxu", 72).is_some());
        for _ in 0..5 {
            engine.reset();
            type_text(&mut engine, "xie");
            choose(&mut engine, "写");
            type_text(&mut engine, "cx");
            choose(&mut engine, "程序");
        }
        assert!(engine.decode_cache.get("wozaixiechengxu", 72).is_none());
        for input in [
            "wozaixiechengxu",
            "wzxiecx",
            "wozaixiechengx",
            "wo'zai'xiechengxu",
        ] {
            engine.reset();
            type_text(&mut engine, input);
            assert_eq!(engine.candidates()[0].text, "我在写程序", "{input}");
        }
        // Only the component selections were learned, not the generated sentence.
        assert!(
            !engine
                .profiles
                .current()
                .learning
                .words
                .contains_key("wozaixiechengxu")
        );
        engine.activate_profile("编程").unwrap();
        assert_eq!(engine.candidates()[0].text, "我在些程序");
        engine.activate_profile("日常").unwrap();
        assert_eq!(engine.candidates()[0].text, "我在写程序");
        drop(engine);
        let mut restored = PinyinEngine::open(&data, &config).unwrap();
        type_text(&mut restored, "wozaixiechengxu");
        assert_eq!(restored.candidates()[0].text, "我在写程序");
        restored
            .vocabulary(
                None,
                mirix_engine_protocol::VocabularyOperation::Forget {
                    word: mirix_engine_protocol::VocabularyWord {
                        text: "程序".into(),
                        reading: "cheng xu".into(),
                    },
                },
            )
            .unwrap();
        assert_eq!(restored.candidates()[0].text, "我在些程序");
    }
}
