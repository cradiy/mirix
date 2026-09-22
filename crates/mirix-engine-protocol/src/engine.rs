use crate::KeyEvent;

/// Validate a nonempty UTF-8 byte selection before forwarding it to a renderer.
pub fn preedit_selection(
    text: &str,
    selection: Option<&std::ops::Range<usize>>,
) -> Option<std::ops::Range<usize>> {
    selection
        .filter(|range| {
            range.start < range.end
                && range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        })
        .cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CandidateId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    pub id: CandidateId,
    pub text: String,
    pub comment: Option<String>,
    /// Engine-controlled pronunciation, shown independently of skin comment preferences.
    pub reading: Option<String>,
    /// Selection updates composition instead of dismissing the candidate panel.
    pub keep_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Commit {
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PunctuationMode {
    #[default]
    Full,
    Half,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Composition {
    /// Nonempty composition text.
    pub text: String,
    /// UTF-8 byte offset in this composition's text.
    pub cursor: i32,
    pub selection: Option<std::ops::Range<usize>>,
}

impl Composition {
    pub fn validate(&self) -> Result<(), String> {
        if self.text.is_empty()
            || self.cursor < 0
            || !self.text.is_char_boundary(self.cursor as usize)
        {
            return Err("composition needs nonempty text and a UTF-8 cursor boundary".into());
        }
        if self.selection.is_some()
            && preedit_selection(&self.text, self.selection.as_ref()).is_none()
        {
            return Err("composition selection must be a nonempty UTF-8 byte range".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PreeditUpdate {
    #[default]
    Keep,
    Clear,
    Set(Composition),
}

/// The panel header is independent of the application's composition.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PanelPreedit {
    #[default]
    FollowApplication,
    Hidden,
    Composition(Composition),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PanelUpdate {
    #[default]
    Keep,
    Set(PanelPreedit),
}

/// Configuration and vocabulary return their UI result alongside the same input effect as keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionResult<T> {
    pub value: T,
    pub output: EngineOutput,
}

impl<T> ActionResult<T> {
    pub fn new(value: T, output: EngineOutput) -> Self {
        Self { value, output }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct EngineOutput {
    pub consumed: bool,
    pub commit: Option<Commit>,
    pub preedit: PreeditUpdate,
    pub panel_composition: PanelUpdate,
    pub candidates_changed: bool,
}

impl EngineOutput {
    pub fn validate(&self, candidates: &[Candidate], selected: usize) -> Result<(), String> {
        if self.commit.is_some() && matches!(self.preedit, PreeditUpdate::Keep) {
            return Err("a commit must explicitly clear or replace application preedit".into());
        }
        if let PreeditUpdate::Set(composition) = &self.preedit {
            composition.validate()?;
        }
        if let PanelUpdate::Set(PanelPreedit::Composition(composition)) = &self.panel_composition {
            composition.validate()?;
        }
        validate_candidates(candidates, selected)
    }

    pub fn preedit_text(&self) -> Option<&str> {
        match &self.preedit {
            PreeditUpdate::Keep => None,
            PreeditUpdate::Clear => Some(""),
            PreeditUpdate::Set(composition) => Some(&composition.text),
        }
    }

    pub fn preedit_cursor(&self) -> i32 {
        match &self.preedit {
            PreeditUpdate::Set(composition) => composition.cursor,
            _ => 0,
        }
    }

    pub fn preedit_selection(&self) -> Option<std::ops::Range<usize>> {
        match &self.preedit {
            PreeditUpdate::Set(composition) => composition.selection.clone(),
            _ => None,
        }
    }

    pub fn pass() -> Self {
        Self::default()
    }

    pub fn consume() -> Self {
        Self {
            consumed: true,
            ..Self::default()
        }
    }

    pub fn commit(text: impl Into<String>) -> Self {
        Self {
            consumed: true,
            commit: Some(Commit { text: text.into() }),
            ..Self::clear_preedit()
        }
    }

    pub fn preedit(text: impl Into<String>) -> Self {
        let text = text.into();
        if text.is_empty() {
            return Self::clear_preedit();
        }
        let cursor = text.len() as i32;
        Self {
            consumed: true,
            preedit: PreeditUpdate::Set(Composition {
                text,
                cursor,
                selection: None,
            }),
            panel_composition: PanelUpdate::Set(PanelPreedit::FollowApplication),
            ..Self::default()
        }
    }

    pub fn clear_preedit() -> Self {
        Self {
            consumed: true,
            preedit: PreeditUpdate::Clear,
            panel_composition: PanelUpdate::Set(PanelPreedit::FollowApplication),
            ..Self::default()
        }
    }

    /// Clear composition state while allowing the triggering key to continue
    /// to the application (for example, a shortcut interrupting Compose).
    pub fn clear_preedit_and_pass() -> Self {
        Self {
            consumed: false,
            ..Self::clear_preedit()
        }
    }
}

pub fn validate_candidates(candidates: &[Candidate], selected: usize) -> Result<(), String> {
    if (candidates.is_empty() && selected != 0)
        || (!candidates.is_empty() && selected >= candidates.len())
    {
        return Err("candidate selection is out of range".into());
    }
    let mut ids = std::collections::HashSet::with_capacity(candidates.len());
    if candidates.iter().any(|candidate| !ids.insert(candidate.id)) {
        return Err("candidates need unique IDs".into());
    }
    Ok(())
}

/// Single input method contract. Host routing and method switching are not engine responsibilities.
pub trait Engine {
    fn name(&self) -> &str;
    /// Return shared immutable image data. None selects the host's generic icon.
    fn tray_icon(&self) -> Option<std::sync::Arc<crate::EngineIcon>> {
        None
    }
    /// Lend an out-of-process session to the host's I/O worker, retaining a UI snapshot.
    fn take_worker(&mut self) -> Option<Box<dyn Engine + Send>> {
        None
    }
    /// A focus reset requested while the session was on the worker invalidates its composition.
    fn reset_pending(&self) -> bool {
        false
    }
    /// Host-visible runtime failure; configuration and installation are separate states.
    fn failure(&self) -> Option<&str> {
        None
    }
    /// Detach process cleanup so the host can wait outside its event loop before restarting.
    fn retire(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        None
    }
    /// Increment whenever configuration values or available controls change.
    fn configuration_revision(&self) -> u64 {
        0
    }
    fn configuration(&mut self, _locale: &str) -> Result<crate::Configuration, String> {
        Ok(crate::Configuration {
            title: self.name().into(),
            description: String::new(),
            groups: vec![],
        })
    }
    /// Apply a fast configuration operation; enqueue expensive work outside the key path.
    /// Return a success message and input effect, or a failure message for the settings UI.
    fn configure(
        &mut self,
        _locale: &str,
        _revision: u64,
        _event: crate::ConfigurationEvent,
    ) -> Result<ActionResult<Option<String>>, String> {
        Err("this input method has no configurable controls".into())
    }
    fn vocabulary(
        &mut self,
        _profile: Option<&str>,
        _operation: crate::VocabularyOperation,
    ) -> Result<ActionResult<crate::VocabularyResult>, String> {
        Err("this input method does not support personal vocabulary management".into())
    }
    fn punctuation_mode(&self) -> Option<PunctuationMode> {
        None
    }
    /// Menus are declarative snapshots; selection is routed back by opaque IDs.
    fn menus(&self) -> Vec<crate::EngineMenu> {
        vec![]
    }
    fn select_menu(&mut self, _menu: &str, _entry: &str) -> Result<EngineOutput, String> {
        Err("this input method has no shortcut menus".into())
    }
    fn process_key(&mut self, ev: &KeyEvent) -> EngineOutput;
    fn reset(&mut self);
    fn candidates(&self) -> &[Candidate];
    /// Index highlighted by the candidate panel. Engines without navigation
    /// support keep the first candidate selected.
    fn selected_candidate(&self) -> usize {
        0
    }
    fn select(&mut self, id: CandidateId) -> EngineOutput;

    fn set_candidate_pages(&mut self, _ends: Vec<usize>) {}

    fn candidate_page_ends(&self) -> &[usize] {
        &[]
    }
}
