//! Directory-discovered profiles; disk writes run outside the key path.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use mirix_engine_protocol::PunctuationMode;
use mirix_pinyin_core::{Lexicon, SyllableTable, import_tsv};
use serde::Serialize;

use crate::extensions::Extensions;
use crate::learning::{Learning, Selection};
use crate::vocabulary::Vocabulary;
use mirix_engine_protocol::VocabularyOperation;
mod batch;
mod management;
pub(crate) use management::ProfileAction;

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("profile storage: {0}")]
    Io(#[from] io::Error),
    #[error("profile data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid dictionary: {0}")]
    Dictionary(String),
    #[error("invalid extensions: {0}")]
    Extensions(String),
}

#[derive(Clone)]
pub(crate) struct Profile {
    pub vocabulary: Vocabulary,
    pub lexicon: Lexicon,
    pub learning: Learning,
    pub extensions: Extensions,
}

pub(crate) struct Profiles {
    pub punctuation_mode: PunctuationMode,
    pub menu_shortcut: mirix_engine_protocol::Shortcut,
    pub active: String,
    pub entries: BTreeMap<String, Profile>,
    writer: Option<Writer>,
    _lock: Option<fs::File>,
    directories: Option<ProfileDirectories>,
}

struct ProfileDirectories {
    config: PathBuf,
    data: PathBuf,
}

enum WriteEvent {
    Manage {
        action: ProfileAction,
        old: String,
        name: String,
        config: PathBuf,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Checkpoint(mpsc::Sender<Result<(), String>>),
    Retry,
    Edit {
        profile: String,
        operation: VocabularyOperation,
    },
    Learn {
        profile: String,
        selection: Selection,
    },
    Activate(String),
    MenuShortcut(mirix_engine_protocol::Shortcut),
    Punctuation(PunctuationMode),
}

struct Writer {
    sender: Option<mpsc::Sender<WriteEvent>>,
    thread: Option<JoinHandle<()>>,
    status: Arc<Mutex<WriteStatus>>,
}

#[derive(Default)]
struct WriteStatus {
    revision: u64,
    error: Option<String>,
}

fn report(status: &Mutex<WriteStatus>, error: Option<String>) {
    let mut status = status.lock().unwrap();
    if status.error != error {
        if let Some(error) = &error {
            eprintln!("mirix-pinyin: {error}");
        }
        status.error = error;
        status.revision = status.revision.wrapping_add(1);
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Profiles {
    pub fn memory() -> Self {
        Self {
            punctuation_mode: PunctuationMode::default(),
            menu_shortcut: "Ctrl+Shift+P".parse().unwrap(),
            active: "日常".into(),
            entries: BTreeMap::from([(
                "日常".into(),
                Profile {
                    vocabulary: Vocabulary::default(),
                    lexicon: Lexicon::default(),
                    learning: Learning::default(),
                    extensions: Extensions::default(),
                },
            )]),
            writer: None,
            _lock: None,
            directories: None,
        }
    }

    pub fn open(root: &Path, config: &Path, table: &SyllableTable) -> Result<Self, ProfileError> {
        let directory = config.join("profiles");
        fs::create_dir_all(&directory)?;
        fs::create_dir_all(root)?;
        let lock = fs::File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("learning.lock"))?;
        lock.try_lock().map_err(|error| {
            io::Error::other(format!("cannot lock {}: {error}", root.display()))
        })?;
        let mut entries = BTreeMap::new();
        for item in fs::read_dir(&directory)? {
            let item = item?;
            if !item.file_type()?.is_dir() {
                continue;
            }
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let dictionaries = item.path().join("dictionaries");
            fs::create_dir_all(&dictionaries)?;
            let lexicon = load_dictionaries(&dictionaries, table)?;
            let learning: Learning =
                read_json(&root.join("profiles").join(&name).join("learning.json"))?
                    .unwrap_or_default();
            learning.prepare_index();
            let vocabulary = read_json(&root.join("profiles").join(&name).join("vocabulary.json"))?
                .unwrap_or_default();
            let extensions = Extensions::load(&item.path().join("extensions"))?;
            entries.insert(
                name,
                Profile {
                    vocabulary,
                    lexicon,
                    learning,
                    extensions,
                },
            );
        }
        if entries.is_empty() {
            fs::create_dir_all(directory.join("日常/dictionaries"))?;
            entries = Self::memory().entries;
            entries.get_mut("日常").unwrap().extensions =
                Extensions::load(&directory.join("日常/extensions"))?;
        }
        let saved: Option<String> = read_json(&root.join("active-profile.json"))?;
        let punctuation_mode = read_json(&root.join("punctuation-mode.json"))?.unwrap_or_default();
        let menu_shortcut = read_json(&root.join("profile-shortcut.json"))?
            .unwrap_or_else(|| "Ctrl+Shift+P".parse().unwrap());
        let active = saved
            .filter(|name| entries.contains_key(name))
            .unwrap_or_else(|| {
                if entries.contains_key("日常") {
                    "日常".into()
                } else {
                    entries.keys().next().unwrap().clone()
                }
            });
        let learning = entries
            .iter()
            .map(|(name, profile)| (name.clone(), profile.learning.clone()))
            .collect();
        let (sender, receiver) = mpsc::channel();
        let vocabulary = entries
            .iter()
            .map(|(name, profile)| (name.clone(), profile.vocabulary.clone()))
            .collect();
        let writer_root = root.to_owned();
        let status = Arc::new(Mutex::new(WriteStatus::default()));
        let writer_status = status.clone();
        let thread = thread::Builder::new()
            .name("pinyin-learning".into())
            .spawn(move || {
                write_loop(writer_root, learning, vocabulary, receiver, writer_status)
            })?;
        let profiles = Self {
            punctuation_mode,
            menu_shortcut,
            active,
            entries,
            writer: Some(Writer {
                sender: Some(sender),
                thread: Some(thread),
                status,
            }),
            _lock: Some(lock),
            directories: Some(ProfileDirectories {
                config: directory,
                data: root.join("profiles"),
            }),
        };
        profiles.send(WriteEvent::Activate(profiles.active.clone()));
        Ok(profiles)
    }

    pub fn activate(&mut self, name: &str) -> bool {
        if !self.entries.contains_key(name) {
            return false;
        }
        self.active = name.to_owned();
        self.send(WriteEvent::Activate(name.to_owned()));
        true
    }

    pub fn create(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty()
            || matches!(name, "." | "..")
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err(
                "Enter a profile name without path separators or control characters".into(),
            );
        }
        if self.entries.contains_key(name) {
            return Err(format!("Profile already exists: {name}"));
        }
        if let Some(directories) = &self.directories {
            fs::create_dir_all(&directories.data).map_err(|error| error.to_string())?;
            let config = directories.config.join(name);
            let data = directories.data.join(name);
            let paths = [
                config.clone(),
                data,
                config.join("dictionaries"),
                config.join("extensions"),
            ];
            for (index, path) in paths.iter().enumerate() {
                if let Err(error) = fs::create_dir(path) {
                    // Only remove empty directories created by this operation.
                    for created in paths[..index].iter().rev() {
                        if let Err(cleanup) = fs::remove_dir(created) {
                            eprintln!(
                                "mirix-pinyin: remove empty {}: {cleanup}",
                                created.display()
                            );
                        }
                    }
                    return Err(format!(
                        "Cannot create profile {name}: {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        self.entries.insert(
            name.into(),
            Profile {
                vocabulary: Vocabulary::default(),
                lexicon: Lexicon::default(),
                learning: Learning::default(),
                extensions: Extensions::default(),
            },
        );
        Ok(name.into())
    }

    pub fn set_menu_shortcut(&mut self, shortcut: mirix_engine_protocol::Shortcut) {
        self.menu_shortcut = shortcut;
        self.send(WriteEvent::MenuShortcut(shortcut));
    }

    pub fn set_punctuation_mode(&mut self, mode: PunctuationMode) {
        self.punctuation_mode = mode;
        self.send(WriteEvent::Punctuation(mode));
    }

    pub fn storage_revision(&self) -> u64 {
        self.writer
            .as_ref()
            .map(|writer| writer.status.lock().unwrap().revision)
            .unwrap_or(0)
    }

    pub fn storage_error(&self) -> Option<String> {
        self.writer
            .as_ref()
            .and_then(|writer| writer.status.lock().unwrap().error.clone())
    }

    pub fn retry_save(&self) {
        self.send(WriteEvent::Retry);
    }

    fn checkpoint(&self) -> Result<(), String> {
        if let Some(writer) = &self.writer {
            let (sender, receiver) = mpsc::channel();
            writer
                .sender
                .as_ref()
                .unwrap()
                .send(WriteEvent::Checkpoint(sender))
                .map_err(|_| "Learning writer stopped")?;
            receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .map_err(|_| "Timed out waiting for saved profile data")??;
        }
        Ok(())
    }

    pub fn current(&self) -> &Profile {
        &self.entries[&self.active]
    }

    pub fn record(&mut self, selection: Selection) {
        self.entries
            .get_mut(&self.active)
            .unwrap()
            .learning
            .record(&selection);
        self.send(WriteEvent::Learn {
            profile: self.active.clone(),
            selection,
        });
    }

    pub fn edit_vocabulary(&mut self, operation: VocabularyOperation) -> bool {
        let profile = self.entries.get_mut(&self.active).unwrap();
        let mut changed = profile.vocabulary.edit(&operation);
        if let VocabularyOperation::Forget { word } | VocabularyOperation::Remove { word } =
            &operation
        {
            changed |= profile.learning.forget(word);
        }
        if changed {
            self.send(WriteEvent::Edit {
                profile: self.active.clone(),
                operation,
            });
        }
        changed
    }

    fn send(&self, event: WriteEvent) {
        if let Some(writer) = &self.writer
            && writer.sender.as_ref().unwrap().send(event).is_err()
        {
            report(
                &writer.status,
                Some("Learning writer stopped; changes are only in memory".into()),
            );
        }
    }
}

pub(crate) fn load_dictionaries(
    directory: &Path,
    table: &SyllableTable,
) -> Result<Lexicon, ProfileError> {
    let items = match fs::read_dir(directory) {
        Ok(items) => items,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Lexicon::default()),
        Err(error) => return Err(error.into()),
    };
    let mut paths = items
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut entries = Vec::new();
    for path in paths.into_iter().filter(|path| {
        path.is_file() && path.extension().is_some_and(|extension| extension == "tsv")
    }) {
        let result = import_tsv(&fs::read_to_string(&path)?, table);
        if !result.issues.is_empty() {
            return Err(ProfileError::Dictionary(format!(
                "{}: {:?}",
                path.display(),
                result.issues
            )));
        }
        entries.extend(result.lexicon);
    }
    Ok(entries.into_iter().collect())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, ProfileError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save(path: &Path, value: &impl Serialize) -> Result<(), ProfileError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn write_loop(
    root: PathBuf,
    mut learning: BTreeMap<String, Learning>,
    mut vocabulary: BTreeMap<String, Vocabulary>,
    receiver: mpsc::Receiver<WriteEvent>,
    status: Arc<Mutex<WriteStatus>>,
) {
    let mut dirty = BTreeSet::new();
    let mut dirty_vocabulary = BTreeSet::new();
    let mut active = None;
    let mut punctuation_mode = None;
    let mut menu_shortcut = None;
    loop {
        let (events, closed) = batch::receive(&receiver);
        let mut checkpoints = Vec::new();
        for event in events {
            match event {
                WriteEvent::Manage {
                    action,
                    old,
                    name,
                    config,
                    reply,
                } => {
                    let result =
                        management::files(action, &old, &name, &config, &root.join("profiles"));
                    if result.is_ok() {
                        match action {
                            ProfileAction::Rename => {
                                if let Some(value) = learning.remove(&old) {
                                    learning.insert(name.clone(), value);
                                }
                                if let Some(value) = vocabulary.remove(&old) {
                                    vocabulary.insert(name.clone(), value);
                                }
                            }
                            ProfileAction::Duplicate => {
                                learning.insert(
                                    name.clone(),
                                    learning.get(&old).cloned().unwrap_or_default(),
                                );
                                vocabulary.insert(
                                    name.clone(),
                                    vocabulary.get(&old).cloned().unwrap_or_default(),
                                );
                            }
                            ProfileAction::Archive => {
                                learning.remove(&old);
                                vocabulary.remove(&old);
                            }
                        }
                    }
                    let _ = reply.send(result);
                }
                WriteEvent::Checkpoint(reply) => checkpoints.push(reply),
                WriteEvent::Retry => {}
                WriteEvent::Edit { profile, operation } => {
                    vocabulary
                        .entry(profile.clone())
                        .or_default()
                        .edit(&operation);
                    if let VocabularyOperation::Forget { word }
                    | VocabularyOperation::Remove { word } = &operation
                    {
                        learning.entry(profile.clone()).or_default().forget(word);
                        dirty.insert(profile.clone());
                    }
                    dirty_vocabulary.insert(profile);
                }
                WriteEvent::Learn { profile, selection } => {
                    learning
                        .entry(profile.clone())
                        .or_default()
                        .record(&selection);
                    dirty.insert(profile);
                }
                WriteEvent::Activate(name) => active = Some(name),
                WriteEvent::Punctuation(mode) => punctuation_mode = Some(mode),
                WriteEvent::MenuShortcut(shortcut) => menu_shortcut = Some(shortcut),
            }
        }
        let mut errors = Vec::new();
        dirty.retain(|name| {
            match save(
                &root.join("profiles").join(name).join("learning.json"),
                &learning[name],
            ) {
                Ok(()) => false,
                Err(error) => {
                    errors.push(format!("Save learning {name}: {error}"));
                    true
                }
            }
        });
        dirty_vocabulary.retain(|name| {
            match save(
                &root.join("profiles").join(name).join("vocabulary.json"),
                &vocabulary[name],
            ) {
                Ok(()) => false,
                Err(error) => {
                    errors.push(format!("Save vocabulary {name}: {error}"));
                    true
                }
            }
        });
        if let Some(name) = &active {
            match save(&root.join("active-profile.json"), name) {
                Ok(()) => active = None,
                Err(error) => errors.push(format!("Save active profile: {error}")),
            }
        }
        if let Some(shortcut) = menu_shortcut {
            match save(&root.join("profile-shortcut.json"), &shortcut) {
                Ok(()) => menu_shortcut = None,
                Err(error) => errors.push(format!("Save menu shortcut: {error}")),
            }
        }
        if let Some(mode) = punctuation_mode {
            match save(&root.join("punctuation-mode.json"), &mode) {
                Ok(()) => punctuation_mode = None,
                Err(error) => errors.push(format!("Save punctuation mode: {error}")),
            }
        }
        let error = (!errors.is_empty()).then(|| errors.join("\n"));
        report(&status, error.clone());
        for reply in checkpoints {
            let _ = reply.send(error.clone().map_or(Ok(()), Err));
        }
        if closed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirix_engine_protocol::VocabularyWord;

    #[test]
    fn shutdown_flushes_every_selection_in_the_pending_batch() {
        let root = tempfile::tempdir().unwrap();
        let (sender, receiver) = mpsc::channel();
        let path = root.path().to_owned();
        let worker = thread::spawn(move || {
            write_loop(
                path,
                BTreeMap::new(),
                BTreeMap::new(),
                receiver,
                Arc::new(Mutex::new(WriteStatus::default())),
            )
        });
        for _ in 0..600 {
            sender
                .send(WriteEvent::Learn {
                    profile: "work".into(),
                    selection: Selection {
                        input: "ni".into(),
                        reading: "nǐ".into(),
                        text: "你".into(),
                        context: "好".into(),
                        time: 10,
                    },
                })
                .unwrap();
        }
        drop(sender);
        worker.join().unwrap();
        let learning: Learning = read_json(&root.path().join("profiles/work/learning.json"))
            .unwrap()
            .unwrap();
        assert_eq!(learning.words["ni"][0].inputs["ni"].count, 600);
        assert_eq!(learning.words["ni"][0].contexts["ni"]["好"].count, 600);
    }

    #[test]
    fn failed_writes_remain_dirty_and_a_checkpoint_reports_recovery() {
        let root = tempfile::tempdir().unwrap();
        let blocked = root.path().join("profiles/work/vocabulary.json");
        fs::create_dir_all(&blocked).unwrap();
        let (sender, receiver) = mpsc::channel();
        let status = Arc::new(Mutex::new(WriteStatus::default()));
        let worker_status = status.clone();
        let path = root.path().to_owned();
        let thread = thread::spawn(move || {
            write_loop(
                path,
                BTreeMap::new(),
                BTreeMap::new(),
                receiver,
                worker_status,
            )
        });
        sender
            .send(WriteEvent::Edit {
                profile: "work".into(),
                operation: VocabularyOperation::Add {
                    word: VocabularyWord {
                        text: "输入".into(),
                        reading: "shū rù".into(),
                    },
                },
            })
            .unwrap();
        let checkpoint = || {
            let (reply, result) = mpsc::channel();
            sender.send(WriteEvent::Checkpoint(reply)).unwrap();
            result
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        };
        assert!(checkpoint().is_err());
        assert!(status.lock().unwrap().error.is_some());
        let failed_revision = status.lock().unwrap().revision;
        fs::remove_dir(&blocked).unwrap(); // Remove only the empty obstruction created above.
        checkpoint().unwrap();
        assert!(status.lock().unwrap().error.is_none());
        assert!(status.lock().unwrap().revision > failed_revision);
        let vocabulary: Vocabulary = read_json(&blocked).unwrap().unwrap();
        assert_eq!(vocabulary.list(&Learning::default(), "输入").len(), 1);
        drop(sender);
        thread.join().unwrap();
    }
}
