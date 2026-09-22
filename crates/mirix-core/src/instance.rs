use std::collections::{HashMap, HashSet};

use crate::context::{ContextId, InputContext, Preedit};
use crate::quick_switch::{QuickSwitch, QuickSwitchAction};
use crate::{CandidateId, Engine, EngineManager, EngineOutput, KeyEvent};
use crate::{MenuEntry, QuickSwitchKind, QuickSwitchShortcuts, QuickSwitchSnapshot};

pub struct Instance {
    next_id: u64,
    contexts: HashMap<ContextId, InputContext>,
    focused: Option<ContextId>,
    engine: EngineManager,
    ime_inhibitors: HashSet<u32>,
    quick_switch: QuickSwitch,
    pending_skin: Option<String>,
    pending_menu: Option<(String, String, String, u64)>,
    input_epoch: u64,
}

impl Instance {
    pub fn add_engine(&mut self, engine: Box<dyn Engine>) -> Result<(), String> {
        self.engine.add_engine(engine)
    }

    pub fn set_enabled_methods(&mut self, names: Vec<String>) -> Result<(), String> {
        let active = self.engine.name().to_owned();
        self.engine.set_enabled(names)?;
        if self.engine.name() != active {
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.quick_switch.close();
            if let Some(context) = self.focused.and_then(|id| self.contexts.get_mut(&id)) {
                context.preedit = Preedit::empty();
            }
        }
        Ok(())
    }
    pub fn configuration_status(&self) -> Vec<crate::ConfigurationStatus> {
        self.engine.configuration_status()
    }

    pub fn configuration(
        &mut self,
        name: &str,
        locale: &str,
    ) -> Result<crate::EngineConfiguration, String> {
        self.engine.configuration(name, locale)
    }

    pub fn configure(
        &mut self,
        status: &crate::ConfigurationStatus,
        locale: &str,
        event: crate::ConfigurationEvent,
    ) -> Result<crate::ActionResult<crate::EngineConfiguration>, String> {
        let menus = self.engine.menus();
        let result = self.engine.configure(status, locale, event);
        if self.engine.menus() != menus {
            self.quick_switch.close();
        }
        if status.input_method == self.engine_name()
            && self.ime_enabled()
            && let Ok(result) = &result
            && let Some(id) = self.focused
        {
            self.apply_output(id, &result.output);
        }
        result
    }
    pub fn new(engine: Box<dyn Engine>) -> Self {
        Self::with_manager(EngineManager::new(vec![engine]).expect("named engine"))
    }

    pub fn with_manager(engine: EngineManager) -> Self {
        Self {
            next_id: 1,
            contexts: HashMap::new(),
            focused: None,
            engine,
            ime_inhibitors: HashSet::new(),
            quick_switch: QuickSwitch::default(),
            pending_skin: None,
            pending_menu: None,
            input_epoch: 0,
        }
    }

    pub fn engine_name(&self) -> &str {
        self.engine.name()
    }

    pub fn input_methods(&self) -> Vec<String> {
        self.engine.input_methods()
    }

    pub fn switch_shortcut(&self) -> crate::Shortcut {
        self.engine.switch_shortcut()
    }

    pub fn set_switch_shortcut(&mut self, shortcut: crate::Shortcut) {
        self.engine.set_switch_shortcut(shortcut);
    }

    pub fn menus(&self) -> Vec<crate::EngineMenu> {
        self.engine.menus()
    }

    pub fn tray_icon(&self) -> Option<std::sync::Arc<crate::EngineIcon>> {
        self.engine.tray_icon()
    }

    pub fn vocabulary(
        &mut self,
        name: &str,
        profile: Option<&str>,
        operation: crate::VocabularyOperation,
    ) -> Result<crate::ActionResult<crate::VocabularyResult>, String> {
        let result = self.engine.vocabulary_for(name, profile, operation)?;
        if name == self.engine_name()
            && self.ime_enabled()
            && let Some(id) = self.focused
        {
            self.apply_output(id, &result.output);
        }
        Ok(result)
    }

    pub fn select_menu(&mut self, menu: &str, entry: &str) -> Result<EngineOutput, String> {
        let effect = self.engine.select_menu(menu, entry)?;
        self.quick_switch.close();
        Ok(effect)
    }

    pub fn set_quick_switch_shortcuts(&mut self, shortcuts: QuickSwitchShortcuts) {
        self.quick_switch.shortcuts = shortcuts;
    }

    pub fn set_quick_switch_available(&mut self, available: bool) {
        self.quick_switch.available = available;
        if !available {
            self.quick_switch.close();
        }
    }

    pub fn set_panel_skins(&mut self, entries: Vec<MenuEntry>, active: Option<String>) {
        self.quick_switch.skins = entries;
        self.quick_switch.active_skin = active;
        // A catalog replacement invalidates the displayed indices.
        if self
            .quick_switch
            .snapshot
            .as_ref()
            .is_some_and(|state| state.kind == QuickSwitchKind::Skin)
        {
            self.quick_switch.close();
        }
    }

    pub fn quick_switch_snapshot(&self) -> Option<&QuickSwitchSnapshot> {
        self.quick_switch.snapshot.as_ref()
    }

    pub fn take_skin_selection(&mut self) -> Option<String> {
        self.pending_skin.take()
    }

    pub fn choose_quick_switch(&mut self, session: u64, index: usize) -> bool {
        let selected = self.prepare_quick_switch(session, index);
        self.flush_menu_selection();
        selected
    }

    pub fn prepare_quick_switch(&mut self, session: u64, index: usize) -> bool {
        let Some((kind, id)) = self.quick_switch.select(session, index) else {
            return false;
        };
        self.apply_quick_switch(kind, id);
        true
    }

    fn apply_quick_switch(&mut self, kind: QuickSwitchKind, id: String) {
        match kind {
            QuickSwitchKind::Engine { id: menu } => {
                if let Some(state) = &self.quick_switch.snapshot {
                    self.pending_menu = Some((self.engine_name().into(), menu, id, state.session));
                }
            }
            QuickSwitchKind::Skin => {
                self.pending_skin = Some(id);
                self.quick_switch.close();
            }
        }
    }

    pub fn take_menu_selection(&mut self) -> Option<(String, String, String, u64)> {
        self.pending_menu.take()
    }

    pub fn complete_menu_selection(&mut self, session: u64, result: Result<(), String>) {
        if self
            .quick_switch
            .snapshot
            .as_ref()
            .is_none_or(|state| state.session != session)
        {
            return;
        }
        match result {
            Ok(()) => self.quick_switch.close(),
            Err(message) => self.quick_switch.snapshot.as_mut().unwrap().error = Some(message),
        }
    }

    fn flush_menu_selection(&mut self) -> Option<EngineOutput> {
        if let Some((_, menu, entry, session)) = self.take_menu_selection() {
            let result = self.engine.select_menu(&menu, &entry);
            let output = result.as_ref().ok().cloned();
            if let Some(output) = &output
                && let Some(id) = self.focused
            {
                self.apply_output(id, output);
            }
            self.complete_menu_selection(session, result.map(|_| ()));
            return output;
        }
        None
    }

    pub fn activate_input_method(&mut self, name: &str) -> bool {
        let changed = self.engine.name() != name;
        if !self.engine.activate_input_method(name) {
            return false;
        }
        if changed {
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.quick_switch.close();
        }
        if changed
            && let Some(id) = self.focused
            && let Some(context) = self.contexts.get_mut(&id)
        {
            context.preedit = Preedit::empty();
        }
        true
    }

    pub fn toggle_input_method(&mut self) -> Option<String> {
        let name = self.engine.toggle()?;
        self.input_epoch = self.input_epoch.wrapping_add(1);
        self.quick_switch.close();
        if let Some(id) = self.focused
            && let Some(context) = self.contexts.get_mut(&id)
        {
            context.preedit = Preedit::empty();
        }
        Some(name)
    }

    pub fn create_context(&mut self, frontend: impl Into<String>) -> ContextId {
        let id = ContextId(self.next_id);
        self.next_id += 1;
        self.contexts.insert(id, InputContext::new(id, frontend));
        id
    }

    pub fn destroy_context(&mut self, id: ContextId) {
        if self.focused == Some(id) {
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.quick_switch.reset_focus();
            self.ime_inhibitors.clear();
            self.focused = None;
            self.engine.reset();
        }
        self.contexts.remove(&id);
    }

    pub fn context(&self, id: ContextId) -> Option<&InputContext> {
        self.contexts.get(&id)
    }

    pub fn context_mut(&mut self, id: ContextId) -> Option<&mut InputContext> {
        self.contexts.get_mut(&id)
    }

    pub fn focus_in(&mut self, id: ContextId) {
        if !self.contexts.contains_key(&id) {
            return;
        }
        if self.focused != Some(id) {
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.quick_switch.reset_focus();
            self.ime_inhibitors.clear();
        }
        if let Some(prev) = self.focused
            && prev != id
        {
            if let Some(ctx) = self.contexts.get_mut(&prev) {
                ctx.focused = false;
            }
            self.engine.reset();
        }
        if let Some(ctx) = self.contexts.get_mut(&id) {
            ctx.focused = true;
            self.focused = Some(id);
        }
    }

    pub fn focus_out(&mut self, id: ContextId) {
        if self.focused == Some(id) {
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.quick_switch.reset_focus();
            self.ime_inhibitors.clear();
            self.focused = None;
            self.engine.reset();
        }
        if let Some(ctx) = self.contexts.get_mut(&id) {
            ctx.focused = false;
            ctx.preedit = Preedit::empty();
        }
    }

    pub fn process_key(&mut self, id: ContextId, ev: &KeyEvent) -> EngineOutput {
        if let Some(output) = self.prepare_key(id, ev) {
            return self.flush_menu_selection().unwrap_or(output);
        }
        let out = self.engine.process_key(ev);
        self.apply_output(id, &out);
        out
    }

    pub fn prepare_key(&mut self, id: ContextId, ev: &KeyEvent) -> Option<EngineOutput> {
        if !self.contexts.contains_key(&id) || !self.ime_inhibitors.is_empty() {
            return Some(EngineOutput::pass());
        }
        // Framework switching takes precedence over any engine-declared binding.
        if let Some(output) = self.engine.intercept_key(ev) {
            self.quick_switch.close();
            self.apply_output(id, &output);
            return Some(output);
        }
        if self.focused == Some(id) {
            let visible = !self.candidates().is_empty();
            match self.quick_switch.handle(ev, visible, || {
                self.engine
                    .menus()
                    .into_iter()
                    .filter(|menu| menu.shortcut != self.engine.switch_shortcut())
                    .collect()
            }) {
                QuickSwitchAction::Pass => {}
                action => {
                    if let QuickSwitchAction::Select(kind, entry) = action {
                        self.apply_quick_switch(kind, entry);
                    }
                    return Some(EngineOutput::consume());
                }
            }
        }
        None
    }

    pub fn input_epoch(&self) -> u64 {
        self.input_epoch
    }

    pub fn take_engine_worker(&mut self, name: &str) -> Option<Box<dyn Engine + Send>> {
        self.engine.take_worker(name)
    }

    pub fn restore_engine_worker(&mut self, engine: Box<dyn Engine + Send>) {
        let menus = self.engine.menus();
        self.engine.restore_worker(engine);
        if self.engine.menus() != menus {
            self.quick_switch.close();
        }
    }

    pub fn complete_key(&mut self, id: ContextId, epoch: u64, output: &EngineOutput) -> bool {
        if epoch != self.input_epoch
            || self.focused != Some(id)
            || output
                .validate(self.engine.candidates(), self.engine.selected_candidate())
                .is_err()
        {
            return false;
        }
        self.apply_output(id, output);
        true
    }

    pub fn engine_failures(&self) -> std::collections::BTreeMap<String, String> {
        self.engine.failures()
    }

    pub fn retire_engine(
        &mut self,
        name: &str,
    ) -> Result<Option<Box<dyn FnOnce() + Send>>, String> {
        if self.engine.name() == name {
            self.input_epoch = self.input_epoch.wrapping_add(1);
            self.quick_switch.close();
            if let Some(id) = self.focused {
                self.contexts.get_mut(&id).unwrap().preedit = Preedit::empty();
            }
        }
        Ok(self.engine.engine_mut(name)?.retire())
    }

    pub fn replace_engine(&mut self, engine: Box<dyn crate::Engine>) -> Result<(), String> {
        self.engine.replace_engine(engine)
    }

    /// Whether the focused context allows the engine to process keys.
    pub fn ime_enabled(&self) -> bool {
        self.focused.is_some() && self.ime_inhibitors.is_empty()
    }

    /// Each controller owns one pause request for the current focus interval.
    /// Enabling only releases that controller's request, not other controllers'.
    pub fn set_ime_enabled(&mut self, id: ContextId, controller: u32, enabled: bool) -> bool {
        if self.focused != Some(id) {
            return false;
        }
        let was_enabled = self.ime_enabled();
        if enabled {
            self.ime_inhibitors.remove(&controller);
        } else {
            self.ime_inhibitors.insert(controller);
        }
        if was_enabled && !self.ime_enabled() {
            self.reset(id);
        }
        true
    }

    pub fn release_ime_controller(&mut self, controller: u32) {
        self.ime_inhibitors.remove(&controller);
    }

    fn apply_output(&mut self, id: ContextId, out: &EngineOutput) {
        if let Some(ctx) = self.contexts.get_mut(&id) {
            match &out.preedit {
                crate::PreeditUpdate::Keep => {}
                crate::PreeditUpdate::Clear => {
                    ctx.preedit.text.clear();
                    ctx.preedit.cursor = 0;
                    ctx.preedit.selection = None;
                }
                crate::PreeditUpdate::Set(composition) => {
                    ctx.preedit.text = composition.text.clone();
                    ctx.preedit.cursor = composition.cursor;
                    ctx.preedit.selection = composition.selection.clone();
                }
            }
            if let crate::PanelUpdate::Set(panel) = &out.panel_composition {
                ctx.preedit.panel_composition = panel.clone();
            }
        }
    }

    pub fn reset(&mut self, id: ContextId) {
        self.input_epoch = self.input_epoch.wrapping_add(1);
        self.quick_switch.close();
        self.engine.reset();
        if let Some(ctx) = self.contexts.get_mut(&id) {
            ctx.preedit = Preedit::empty();
        }
    }

    pub fn select(&mut self, id: ContextId, candidate_id: CandidateId) -> Option<EngineOutput> {
        if self.focused != Some(id) || !self.ime_enabled() || self.quick_switch.snapshot.is_some() {
            return None;
        }
        let out = self.engine.select(candidate_id);
        self.apply_output(id, &out);
        Some(out)
    }

    pub fn candidates(&self) -> &[crate::Candidate] {
        if self.ime_inhibitors.is_empty() {
            self.engine.candidates()
        } else {
            &[]
        }
    }

    pub fn set_candidate_pages(
        &mut self,
        id: ContextId,
        ids: &[CandidateId],
        ends: Vec<usize>,
    ) -> bool {
        if self.focused != Some(id)
            || !self.ime_enabled()
            || !self
                .engine
                .candidates()
                .iter()
                .map(|candidate| candidate.id)
                .eq(ids.iter().copied())
            || !crate::valid_candidate_pages(ids.len(), &ends)
        {
            return false;
        }
        self.engine.set_candidate_pages(ends);
        true
    }

    pub fn focused(&self) -> Option<ContextId> {
        self.focused
    }

    pub fn panel_snapshot(&self) -> PanelSnapshot {
        let preedit = self
            .focused
            .and_then(|id| self.contexts.get(&id))
            .map(|c| c.preedit.text.clone())
            .unwrap_or_default();
        let candidates = self
            .candidates()
            .iter()
            .enumerate()
            .map(|(index, candidate)| PanelCandidate {
                id: candidate.id,
                label: candidate_label(
                    index,
                    self.engine.candidates().len(),
                    self.engine.candidate_page_ends(),
                ),
                text: candidate.text.clone(),
                comment: candidate.comment.clone(),
                reading: candidate.reading.clone(),
                keep_open: candidate.keep_open,
            })
            .collect::<Vec<_>>();
        let selected = if candidates.is_empty() {
            0
        } else {
            self.engine.selected_candidate().min(candidates.len() - 1) as u32
        };
        let panel_composition = self
            .focused
            .and_then(|id| self.contexts.get(&id))
            .map(|c| c.preedit.panel_composition.clone())
            .unwrap_or_default();
        let visible = !preedit.is_empty()
            || !candidates.is_empty()
            || matches!(panel_composition, crate::PanelPreedit::Composition(_));
        PanelSnapshot {
            context_id: self.focused,
            punctuation_mode: self.engine.punctuation_mode(),
            preedit,
            panel_composition,
            preedit_cursor: self
                .focused
                .and_then(|id| self.contexts.get(&id))
                .map_or(0, |c| c.preedit.cursor),
            preedit_selection: self
                .focused
                .and_then(|id| self.contexts.get(&id))
                .and_then(|c| c.preedit.selection.clone()),
            candidates,
            selected,
            visible,
        }
    }
}

fn candidate_label(index: usize, count: usize, ends: &[usize]) -> String {
    (index - crate::candidate_page(index, count, ends).start + 1).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PanelCandidate {
    pub id: CandidateId,
    pub label: String,
    pub text: String,
    pub comment: Option<String>,
    pub reading: Option<String>,
    pub keep_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSnapshot {
    pub punctuation_mode: Option<crate::PunctuationMode>,
    pub context_id: Option<ContextId>,
    pub preedit: String,
    pub panel_composition: crate::PanelPreedit,
    pub preedit_cursor: i32,
    pub preedit_selection: Option<std::ops::Range<usize>>,
    pub candidates: Vec<PanelCandidate>,
    pub selected: u32,
    pub visible: bool,
}

impl PanelSnapshot {
    pub fn panel_text(&self) -> &str {
        match &self.panel_composition {
            crate::PanelPreedit::FollowApplication => &self.preedit,
            crate::PanelPreedit::Hidden => "",
            crate::PanelPreedit::Composition(panel) => &panel.text,
        }
    }

    pub fn panel_selection(&self) -> Option<&std::ops::Range<usize>> {
        match &self.panel_composition {
            crate::PanelPreedit::FollowApplication => self.preedit_selection.as_ref(),
            crate::PanelPreedit::Hidden => None,
            crate::PanelPreedit::Composition(panel) => panel.selection.as_ref(),
        }
    }

    /// Engine-provided composition is required panel content. The caller's
    /// display preference applies only to the duplicated application preedit.
    pub fn show_panel_preedit(&self, show_application_preedit: bool) -> bool {
        match self.panel_composition {
            crate::PanelPreedit::FollowApplication => show_application_preedit,
            crate::PanelPreedit::Hidden => false,
            crate::PanelPreedit::Composition(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Candidate, EngineManager};
    use crate::{KeyState, Modifiers};

    #[test]
    fn independent_panel_updates_do_not_replace_application_preedit() {
        let mut instance = Instance::with_manager(EngineManager::new(vec![]).unwrap());
        let id = instance.create_context("test");
        instance.focus_in(id);
        let epoch = instance.input_epoch();
        instance.complete_key(id, epoch, &EngineOutput::preedit("kimi"));
        let panel = crate::Composition {
            text: "きみ".into(),
            cursor: 6,
            selection: Some(0..6),
        };
        let output = EngineOutput {
            panel_composition: crate::PanelUpdate::Set(crate::PanelPreedit::Composition(panel)),
            ..EngineOutput::default()
        };
        assert!(instance.complete_key(id, epoch, &output));
        let snapshot = instance.panel_snapshot();
        assert_eq!(snapshot.preedit, "kimi");
        assert_eq!(snapshot.preedit_cursor, 4);
        assert_eq!(snapshot.panel_text(), "きみ");
        assert_eq!(snapshot.panel_selection(), Some(&(0..6)));
        instance.complete_key(
            id,
            epoch,
            &EngineOutput {
                preedit: crate::PreeditUpdate::Clear,
                ..EngineOutput::default()
            },
        );
        assert_eq!(instance.panel_snapshot().panel_text(), "きみ");
        assert!(instance.panel_snapshot().preedit.is_empty());
        instance.complete_key(
            id,
            epoch,
            &EngineOutput {
                panel_composition: crate::PanelUpdate::Set(crate::PanelPreedit::Hidden),
                ..EngineOutput::default()
            },
        );
        assert!(!instance.panel_snapshot().show_panel_preedit(true));
        instance.complete_key(id, epoch, &EngineOutput::commit("君"));
        assert!(!instance.panel_snapshot().visible);
        assert_eq!(
            instance.panel_snapshot().panel_composition,
            crate::PanelPreedit::FollowApplication
        );
    }

    #[test]
    fn candidate_labels_restart_on_each_number_key_page() {
        let labels: Vec<_> = (0..27)
            .map(|index| candidate_label(index, 27, &[]))
            .collect();
        for page in labels.chunks(crate::CANDIDATE_PAGE_SIZE) {
            assert_eq!(page, &["1", "2", "3", "4", "5", "6", "7", "8", "9"]);
        }
    }

    struct FakeEngine {
        candidates: Vec<Candidate>,
        selected: usize,
    }

    impl Engine for FakeEngine {
        fn name(&self) -> &str {
            "fake"
        }

        fn process_key(&mut self, ev: &KeyEvent) -> EngineOutput {
            if ev.is_press() && ev.text.as_deref() == Some(" ") {
                return EngineOutput::commit("好");
            }
            if ev.is_press()
                && let Some(t) = &ev.text
            {
                return EngineOutput {
                    consumed: true,
                    candidates_changed: true,
                    ..EngineOutput::preedit(t.clone())
                };
            }
            EngineOutput::pass()
        }

        fn reset(&mut self) {}

        fn candidates(&self) -> &[Candidate] {
            &self.candidates
        }

        fn selected_candidate(&self) -> usize {
            self.selected
        }

        fn select(&mut self, id: CandidateId) -> EngineOutput {
            self.candidates
                .iter()
                .find(|candidate| candidate.id == id)
                .map(|candidate| EngineOutput::commit(candidate.text.clone()))
                .unwrap_or_else(EngineOutput::consume)
        }
    }

    fn press(text: &str) -> KeyEvent {
        KeyEvent {
            keycode: 0,
            keysym: 0,
            state: KeyState::Press,
            mods: Modifiers::default(),
            text: Some(text.into()),
        }
    }

    #[test]
    fn page_configuration_rejects_stale_candidates_and_unfocused_contexts() {
        let mut instance = Instance::new(Box::new(FakeEngine {
            candidates: vec![Candidate {
                keep_open: false,
                id: CandidateId(42),
                text: "好".into(),
                comment: None,
                reading: None,
            }],
            selected: 0,
        }));
        let id = instance.create_context("test");
        assert!(!instance.set_candidate_pages(id, &[CandidateId(42)], vec![1]));
        instance.focus_in(id);
        assert!(!instance.set_candidate_pages(id, &[CandidateId(41)], vec![1]));
        assert!(!instance.set_candidate_pages(id, &[CandidateId(42)], vec![2]));
        assert!(instance.set_candidate_pages(id, &[CandidateId(42)], vec![1]));
    }

    #[test]
    fn key_pipeline_preedit_then_commit() {
        let engine = FakeEngine {
            candidates: vec![Candidate {
                keep_open: false,
                id: CandidateId(1),
                text: "好".into(),
                comment: None,
                reading: None,
            }],
            selected: 0,
        };
        let mut inst = Instance::new(Box::new(engine));
        let id = inst.create_context("test");
        inst.focus_in(id);

        let out = inst.process_key(id, &press("n"));
        assert!(out.consumed);
        assert_eq!(out.preedit_text(), Some("n"));
        assert_eq!(inst.context(id).unwrap().preedit.text, "n");

        let out = inst.process_key(id, &press(" "));
        assert_eq!(out.commit.as_ref().map(|c| c.text.as_str()), Some("好"));
        assert!(inst.context(id).unwrap().preedit.text.is_empty());
    }

    #[test]
    fn missing_context_passes() {
        let mut inst = Instance::new(Box::new(FakeEngine {
            candidates: vec![],
            selected: 0,
        }));
        let out = inst.process_key(ContextId(99), &press("a"));
        assert!(!out.consumed);
    }

    #[test]
    fn select_candidate() {
        let mut inst = Instance::new(Box::new(FakeEngine {
            candidates: vec![Candidate {
                keep_open: false,
                id: CandidateId(7),
                text: "是".into(),
                comment: None,
                reading: None,
            }],
            selected: 0,
        }));
        let id = inst.create_context("test");
        inst.focus_in(id);
        let out = inst.select(id, CandidateId(7)).unwrap();
        assert_eq!(out.commit.unwrap().text, "是");
    }

    #[test]
    fn selection_can_replace_preedit_without_committing() {
        struct LockingEngine {
            candidates: Vec<Candidate>,
        }

        impl Engine for LockingEngine {
            fn name(&self) -> &str {
                "locking"
            }

            fn process_key(&mut self, _: &KeyEvent) -> EngineOutput {
                EngineOutput::pass()
            }

            fn reset(&mut self) {}

            fn candidates(&self) -> &[Candidate] {
                &self.candidates
            }

            fn select(&mut self, id: CandidateId) -> EngineOutput {
                if id != CandidateId(42) {
                    return EngineOutput::consume();
                }
                EngineOutput {
                    consumed: true,
                    preedit: crate::PreeditUpdate::Set(crate::Composition {
                        text: "中文shuru".into(),
                        cursor: "中文shuru".len() as i32,
                        selection: None,
                    }),
                    candidates_changed: true,
                    ..EngineOutput::default()
                }
            }
        }

        let mut inst = Instance::new(Box::new(LockingEngine {
            candidates: vec![Candidate {
                keep_open: false,
                id: CandidateId(42),
                text: "中文".into(),
                comment: None,
                reading: None,
            }],
        }));
        let context = inst.create_context("test");
        inst.focus_in(context);

        let output = inst.select(context, CandidateId(42)).unwrap();
        assert!(output.commit.is_none());
        assert_eq!(output.preedit_text(), Some("中文shuru"));
        assert_eq!(inst.context(context).unwrap().preedit.text, "中文shuru");
    }

    #[test]
    fn panel_snapshot_tracks_preedit() {
        let mut inst = Instance::new(Box::new(FakeEngine {
            candidates: vec![],
            selected: 0,
        }));
        let id = inst.create_context("test");
        inst.focus_in(id);
        assert!(!inst.panel_snapshot().visible);

        inst.process_key(id, &press("n"));
        let snap = inst.panel_snapshot();
        assert!(snap.visible);
        assert_eq!(snap.preedit, "n");

        inst.focus_out(id);
        assert!(!inst.panel_snapshot().visible);
    }

    #[test]
    fn panel_snapshot_preserves_candidate_metadata() {
        let inst = Instance::new(Box::new(FakeEngine {
            candidates: vec![Candidate {
                keep_open: false,
                id: CandidateId(11),
                text: "候选".into(),
                comment: Some("说明".into()),
                reading: Some("hòu xuǎn".into()),
            }],
            selected: 0,
        }));

        assert_eq!(
            inst.panel_snapshot().candidates,
            vec![PanelCandidate {
                keep_open: false,
                id: CandidateId(11),
                label: "1".into(),
                text: "候选".into(),
                comment: Some("说明".into()),
                reading: Some("hòu xuǎn".into()),
            }]
        );
    }

    #[test]
    fn panel_snapshot_uses_and_clamps_engine_selection() {
        let candidates = ["一", "二", "三"]
            .into_iter()
            .enumerate()
            .map(|(index, text)| Candidate {
                keep_open: false,
                id: CandidateId(index as u64 + 1),
                text: text.into(),
                comment: None,
                reading: None,
            })
            .collect();
        let inst = Instance::new(Box::new(FakeEngine {
            candidates,
            selected: 99,
        }));

        assert_eq!(inst.panel_snapshot().selected, 2);
    }

    #[test]
    fn toggling_input_method_clears_focused_preedit() {
        struct Other;
        impl Engine for Other {
            fn name(&self) -> &str {
                "other"
            }
            fn process_key(&mut self, _: &KeyEvent) -> EngineOutput {
                EngineOutput::pass()
            }
            fn reset(&mut self) {}
            fn candidates(&self) -> &[Candidate] {
                &[]
            }
            fn select(&mut self, _: CandidateId) -> EngineOutput {
                EngineOutput::pass()
            }
        }
        let engine = EngineManager::new(vec![
            Box::new(FakeEngine {
                candidates: vec![],
                selected: 0,
            }),
            Box::new(Other),
        ])
        .unwrap();
        let mut inst = Instance::with_manager(engine);
        let id = inst.create_context("test");
        inst.focus_in(id);
        inst.process_key(id, &press("n"));
        assert_eq!(inst.context(id).unwrap().preedit.text, "n");

        assert_eq!(inst.toggle_input_method().as_deref(), Some("other"));
        assert!(inst.context(id).unwrap().preedit.text.is_empty());
        assert!(!inst.panel_snapshot().visible);
    }

    #[test]
    fn selecting_current_or_unknown_method_does_not_discard_composition() {
        let mut inst = Instance::new(Box::new(FakeEngine {
            candidates: vec![],
            selected: 0,
        }));
        let id = inst.create_context("test");
        inst.focus_in(id);
        inst.process_key(id, &press("n"));
        assert!(inst.activate_input_method("fake"));
        assert_eq!(inst.context(id).unwrap().preedit.text, "n");
        assert!(!inst.activate_input_method("missing"));
        assert_eq!(inst.context(id).unwrap().preedit.text, "n");
    }

    #[test]
    fn ime_pause_hides_composition_and_blocks_keys_and_stale_selection() {
        let mut instance = Instance::new(Box::new(FakeEngine {
            candidates: vec![Candidate {
                keep_open: false,
                id: CandidateId(1),
                text: "你".into(),
                comment: None,
                reading: None,
            }],
            selected: 0,
        }));
        let id = instance.create_context("test");
        instance.focus_in(id);
        instance.process_key(id, &press("n"));
        assert!(instance.panel_snapshot().visible);
        assert!(instance.set_ime_enabled(id, 1, false));
        assert!(!instance.panel_snapshot().visible);
        assert!(instance.candidates().is_empty());
        assert!(instance.context(id).unwrap().preedit.text.is_empty());
        assert_eq!(instance.process_key(id, &press("a")), EngineOutput::pass());
        assert!(instance.select(id, CandidateId(1)).is_none());
        assert!(!instance.set_candidate_pages(id, &[CandidateId(1)], vec![1]));
        assert_eq!(instance.engine_name(), "fake");
        assert!(instance.set_ime_enabled(id, 1, true));
        assert!(instance.process_key(id, &press("a")).consumed);
    }

    #[test]
    fn ime_pause_is_owned_idempotent_and_ends_with_the_focus_interval() {
        let mut instance = Instance::new(Box::new(FakeEngine {
            candidates: vec![],
            selected: 0,
        }));
        let first = instance.create_context("test");
        let second = instance.create_context("test");
        instance.focus_in(first);
        assert!(instance.set_ime_enabled(first, 1, false));
        assert!(instance.set_ime_enabled(first, 1, false));
        assert!(instance.set_ime_enabled(first, 2, false));
        assert!(instance.set_ime_enabled(first, 1, true));
        assert!(!instance.ime_enabled());
        instance.release_ime_controller(2);
        assert!(instance.ime_enabled());

        instance.set_ime_enabled(first, 1, false);
        instance.focus_in(second);
        assert!(instance.ime_enabled());
        assert!(!instance.set_ime_enabled(first, 1, false));
        instance.set_ime_enabled(second, 1, false);
        instance.focus_out(second);
        instance.focus_in(second);
        assert!(instance.ime_enabled());
        instance.set_ime_enabled(second, 1, false);
        instance.destroy_context(second);
        instance.focus_in(first);
        assert!(instance.ime_enabled());
    }
}
