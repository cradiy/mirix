//! Framework-owned method selection. Engines do not implement host routing.

use crate::{Candidate, CandidateId, Engine, EngineOutput, KeyEvent, KeyState, Shortcut};

pub struct EngineManager {
    engines: Vec<Box<dyn Engine>>,
    enabled: Vec<String>,
    active: Option<usize>,
    passthrough: PassThrough,
    shortcut: Shortcut,
    held: Option<(u32, u32)>,
    configuration_session: String,
}

impl EngineManager {
    pub fn new(engines: Vec<Box<dyn Engine>>) -> Result<Self, String> {
        let enabled = engines
            .iter()
            .map(|engine| engine.name().to_owned())
            .collect::<Vec<_>>();
        let mut names = std::collections::HashSet::new();
        if enabled
            .iter()
            .any(|name| name.is_empty() || !names.insert(name))
        {
            return Err("input method names must be nonempty and unique".into());
        }
        Ok(Self {
            active: (!engines.is_empty()).then_some(0),
            engines,
            enabled,
            passthrough: PassThrough,
            shortcut: "Ctrl+Space".parse().unwrap(),
            held: None,
            configuration_session: new_session(),
        })
    }

    pub fn input_methods(&self) -> Vec<String> {
        self.enabled.clone()
    }

    pub fn add_engine(&mut self, engine: Box<dyn Engine>) -> Result<(), String> {
        if engine.name().is_empty()
            || self
                .engines
                .iter()
                .any(|loaded| loaded.name() == engine.name())
        {
            return Err("input method is already loaded or unnamed".into());
        }
        self.engines.push(engine);
        Ok(())
    }

    pub fn replace_engine(&mut self, engine: Box<dyn Engine>) -> Result<(), String> {
        let slot = self
            .engines
            .iter_mut()
            .find(|loaded| loaded.name() == engine.name())
            .ok_or_else(|| "input method is not loaded".to_owned())?;
        *slot = engine;
        // Old settings acknowledgements must not be accepted by a new process.
        self.configuration_session = new_session();
        Ok(())
    }

    pub fn failures(&self) -> std::collections::BTreeMap<String, String> {
        self.engines
            .iter()
            .filter_map(|engine| {
                engine
                    .failure()
                    .map(|error| (engine.name().to_owned(), error.to_owned()))
            })
            .collect()
    }

    pub fn take_worker(&mut self, name: &str) -> Option<Box<dyn Engine + Send>> {
        self.engine_mut(name).ok()?.take_worker()
    }

    pub fn restore_worker(&mut self, mut engine: Box<dyn Engine + Send>) {
        if let Some(slot) = self
            .engines
            .iter_mut()
            .find(|slot| slot.name() == engine.name())
        {
            if slot.reset_pending() {
                engine.reset();
            }
            *slot = engine;
        }
    }

    pub fn configuration_status(&self) -> Vec<crate::ConfigurationStatus> {
        self.engines
            .iter()
            .map(|engine| crate::ConfigurationStatus {
                input_method: engine.name().into(),
                session: self.configuration_session.clone(),
                revision: engine.configuration_revision(),
            })
            .collect()
    }

    pub fn configuration(
        &mut self,
        name: &str,
        locale: &str,
    ) -> Result<crate::EngineConfiguration, String> {
        let session = self.configuration_session.clone();
        let engine = self.engine_mut(name)?;
        let page = engine.configuration(locale)?;
        page.validate()?;
        Ok(crate::EngineConfiguration {
            status: crate::ConfigurationStatus {
                input_method: name.into(),
                session,
                revision: engine.configuration_revision(),
            },
            locale: locale.into(),
            page,
            message: None,
        })
    }

    pub fn configure(
        &mut self,
        status: &crate::ConfigurationStatus,
        locale: &str,
        event: crate::ConfigurationEvent,
    ) -> Result<crate::ActionResult<crate::EngineConfiguration>, String> {
        if status.session != self.configuration_session {
            return Err("input method session changed; refresh settings".into());
        }
        let engine = self.engine_mut(&status.input_method)?;
        if status.revision != engine.configuration_revision() {
            return Err("configuration changed; refresh settings".into());
        }
        let result = engine.configure(locale, status.revision, event)?;
        validate_action(engine, &result.output)?;
        let mut configuration = self.configuration(&status.input_method, locale)?;
        configuration.message = result.value;
        Ok(crate::ActionResult::new(configuration, result.output))
    }

    pub fn set_enabled(&mut self, names: Vec<String>) -> Result<(), String> {
        let mut unique = std::collections::HashSet::new();
        if names.iter().any(|name| {
            !unique.insert(name) || !self.engines.iter().any(|engine| engine.name() == name)
        }) {
            return Err("enabled methods must be loaded, unique names".into());
        }
        let retain_active = names.iter().any(|name| name == self.name());
        self.enabled = names;
        if !retain_active {
            self.reset();
            self.active = self
                .enabled
                .first()
                .and_then(|name| self.engines.iter().position(|engine| engine.name() == name));
            self.reset();
        }
        Ok(())
    }

    pub fn set_switch_shortcut(&mut self, shortcut: Shortcut) {
        self.shortcut = shortcut;
    }

    pub fn switch_shortcut(&self) -> Shortcut {
        self.shortcut
    }

    pub fn activate_input_method(&mut self, name: &str) -> bool {
        if !self.enabled.iter().any(|enabled| enabled == name) {
            return false;
        }
        let target = self.engines.iter().position(|engine| engine.name() == name);
        if target != self.active {
            self.active_mut().reset();
            self.active = target;
            self.active_mut().reset();
        }
        true
    }

    pub fn toggle(&mut self) -> Option<String> {
        if self.enabled.len() < 2 {
            return None;
        }
        let current = self
            .enabled
            .iter()
            .position(|name| name == self.name())
            .unwrap_or(0);
        let next = self.enabled[(current + 1) % self.enabled.len()].clone();
        self.activate_input_method(&next);
        Some(next)
    }

    pub fn vocabulary_for(
        &mut self,
        name: &str,
        profile: Option<&str>,
        operation: crate::VocabularyOperation,
    ) -> Result<crate::ActionResult<crate::VocabularyResult>, String> {
        let engine = self.engine_mut(name)?;
        let result = engine.vocabulary(profile, operation)?;
        validate_action(engine, &result.output)?;
        Ok(result)
    }

    pub fn engine_mut(&mut self, name: &str) -> Result<&mut (dyn Engine + '_), String> {
        for engine in &mut self.engines {
            if engine.name() == name {
                return Ok(engine.as_mut());
            }
        }
        Err(format!("input method is not loaded: {name}"))
    }

    pub fn process_key(&mut self, event: &KeyEvent) -> EngineOutput {
        if let Some(output) = self.intercept_key(event) {
            return output;
        }
        let engine = self.active_mut();
        let output = engine.process_key(event);
        if validate_action(engine, &output).is_err() {
            return EngineOutput::clear_preedit_and_pass();
        }
        output
    }

    pub fn intercept_key(&mut self, event: &KeyEvent) -> Option<EngineOutput> {
        if let Some((code, sym)) = self.held {
            let same_key = if code != 0 {
                code == event.keycode
            } else {
                sym == normalize_symbol(event.keysym)
            };
            if same_key {
                if event.state == KeyState::Release {
                    self.held = None;
                }
                return Some(EngineOutput::consume());
            }
        }
        if self.enabled.len() > 1 && event.is_press() && self.shortcut.matches(event) {
            self.held = Some((event.keycode, normalize_symbol(event.keysym)));
            self.toggle();
            return Some(EngineOutput::clear_preedit());
        }
        None
    }

    pub fn reset(&mut self) {
        self.held = None;
        self.active_mut().reset();
    }

    fn active(&self) -> &dyn Engine {
        match self.active {
            Some(index) => self.engines[index].as_ref(),
            None => &self.passthrough,
        }
    }

    fn active_mut(&mut self) -> &mut dyn Engine {
        match self.active {
            Some(index) => self.engines[index].as_mut(),
            None => &mut self.passthrough,
        }
    }

    pub fn name(&self) -> &str {
        self.active().name()
    }
    pub fn menus(&self) -> Vec<crate::EngineMenu> {
        self.active().menus()
    }
    pub fn tray_icon(&self) -> Option<std::sync::Arc<crate::EngineIcon>> {
        self.active().tray_icon()
    }
    pub fn select_menu(&mut self, menu: &str, entry: &str) -> Result<EngineOutput, String> {
        let engine = self.active_mut();
        let result = crate::select_menu(engine, menu, entry)?;
        validate_action(engine, &result)?;
        Ok(result)
    }
    pub fn candidates(&self) -> &[Candidate] {
        self.active().candidates()
    }
    pub fn selected_candidate(&self) -> usize {
        self.active().selected_candidate()
    }
    pub fn punctuation_mode(&self) -> Option<crate::PunctuationMode> {
        self.active().punctuation_mode()
    }
    pub fn candidate_page_ends(&self) -> &[usize] {
        self.active().candidate_page_ends()
    }
    pub fn set_candidate_pages(&mut self, ends: Vec<usize>) {
        self.active_mut().set_candidate_pages(ends);
    }
    pub fn select(&mut self, id: CandidateId) -> EngineOutput {
        let engine = self.active_mut();
        let output = engine.select(id);
        if validate_action(engine, &output).is_err() {
            return EngineOutput::clear_preedit();
        }
        output
    }
}

fn validate_action(engine: &mut dyn Engine, output: &EngineOutput) -> Result<(), String> {
    let result = output
        .validate(engine.candidates(), engine.selected_candidate())
        .and_then(|()| mirix_engine_protocol::validate_menus(&engine.menus()));
    if result.is_err() {
        engine.reset();
    }
    result
}

struct PassThrough;
fn new_session() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn normalize_symbol(sym: u32) -> u32 {
    char::from_u32(sym)
        .map(|ch| ch.to_ascii_lowercase() as u32)
        .unwrap_or(sym)
}
impl Engine for PassThrough {
    fn name(&self) -> &str {
        ""
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Modifiers, keysym};

    struct Method(&'static str, usize);
    impl Engine for Method {
        fn name(&self) -> &str {
            self.0
        }
        fn process_key(&mut self, _: &KeyEvent) -> EngineOutput {
            EngineOutput::commit(self.0)
        }
        fn reset(&mut self) {
            self.1 += 1;
        }
        fn candidates(&self) -> &[Candidate] {
            &[]
        }
        fn select(&mut self, _: CandidateId) -> EngineOutput {
            EngineOutput::pass()
        }
        fn vocabulary(
            &mut self,
            _: Option<&str>,
            _: crate::VocabularyOperation,
        ) -> Result<crate::ActionResult<crate::VocabularyResult>, String> {
            Ok(crate::ActionResult::new(
                crate::VocabularyResult::Words {
                    profile: self.0.into(),
                    entries: vec![],
                    total: self.1,
                    offset: 0,
                },
                EngineOutput::default(),
            ))
        }
    }

    fn manager() -> EngineManager {
        EngineManager::new(
            ["one", "two", "three"]
                .into_iter()
                .map(|name| Box::new(Method(name, 0)) as Box<dyn Engine>)
                .collect(),
        )
        .unwrap()
    }

    fn space(state: KeyState) -> KeyEvent {
        KeyEvent {
            keycode: crate::evdev::KEY_SPACE,
            keysym: keysym::SPACE,
            state,
            mods: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            text: Some(" ".into()),
        }
    }

    #[test]
    fn cycles_in_enabled_order_and_suppresses_repeat_until_release() {
        let mut manager = manager();
        manager
            .set_enabled(vec!["one".into(), "three".into(), "two".into()])
            .unwrap();
        for expected in ["three", "two", "one"] {
            assert!(manager.process_key(&space(KeyState::Press)).consumed);
            assert_eq!(manager.name(), expected);
            assert!(manager.process_key(&space(KeyState::Press)).consumed);
            assert_eq!(manager.name(), expected);
            let mut release = space(KeyState::Release);
            release.mods = Modifiers::default();
            assert!(manager.process_key(&release).consumed);
        }
    }

    #[test]
    fn selection_is_idempotent_and_inactive_management_does_not_switch() {
        let mut manager = manager();
        assert!(manager.activate_input_method("three"));
        assert!(manager.activate_input_method("three"));
        assert!(!manager.activate_input_method("missing"));
        let query = crate::VocabularyOperation::List {
            query: String::new(),
            offset: 0,
            limit: 10,
        };
        assert!(matches!(
            manager
                .vocabulary_for("three", None, query.clone())
                .unwrap()
                .value,
            crate::VocabularyResult::Words { total: 1, .. }
        ));
        assert!(matches!(
            manager.vocabulary_for("two", None, query).unwrap().value,
            crate::VocabularyResult::Words { total: 0, .. }
        ));
        assert_eq!(manager.name(), "three");
    }

    #[test]
    fn disabling_active_cancels_and_empty_enabled_set_passes_input() {
        let mut manager = manager();
        manager.set_enabled(vec!["two".into()]).unwrap();
        assert_eq!(manager.name(), "two");
        assert!(!manager.activate_input_method("one"));
        assert!(manager.toggle().is_none());
        assert!(manager.set_enabled(vec!["missing".into()]).is_err());
        assert_eq!(manager.input_methods(), ["two"]);
        manager.set_enabled(vec![]).unwrap();
        assert_eq!(manager.name(), "");
        assert_eq!(
            manager.process_key(&space(KeyState::Press)),
            EngineOutput::pass()
        );
        assert!(manager.toggle().is_none());
    }

    #[test]
    fn configured_shortcut_overrides_default_and_handles_modifier_release() {
        let mut manager = manager();
        manager.set_switch_shortcut("Alt+J".parse().unwrap());
        assert_eq!(
            manager
                .process_key(&space(KeyState::Press))
                .commit
                .unwrap()
                .text,
            "one"
        );
        let mut event = space(KeyState::Press);
        event.keycode = 36;
        event.keysym = 'j' as u32;
        event.mods = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        manager.process_key(&event);
        assert_eq!(manager.name(), "two");
        event.state = KeyState::Release;
        event.mods = Modifiers::default();
        assert!(manager.process_key(&event).consumed);
        assert_eq!(manager.switch_shortcut().to_string(), "Alt+J");
    }
}
