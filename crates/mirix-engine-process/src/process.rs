//! Child-process lifetime, bounded request exchange, and engine adaptation.
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use mirix_engine_protocol::encode_frame;
use mirix_engine_protocol::{
    ActionResult, Candidate, CandidateId, Engine, EngineMenu, EngineOutput, KeyEvent,
    PunctuationMode,
};
use mirix_engine_protocol::{EngineRequest, EngineResponse};

use crate::InputMethod;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(250);

#[cfg(test)]
mod tests;

pub struct ProcessEngine {
    icon: Option<std::sync::Arc<mirix_engine_protocol::EngineIcon>>,
    configuration_revision: u64,
    name: String,
    child: Option<Child>,
    reaper: Option<std::thread::JoinHandle<()>>,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    read_buffer: Vec<u8>,
    // Share immutable lists with the main-loop snapshot while a worker owns I/O.
    // Arc<Vec<_>> also retains the response's allocation without moving its elements.
    candidates: Arc<Vec<Candidate>>,
    selected: usize,
    page_ends: Arc<Vec<usize>>,
    failed: bool,
    reset_pending: bool,
    failure: Option<String>,
    menus: Arc<Vec<EngineMenu>>,
    punctuation_mode: Option<PunctuationMode>,
    configuration_cache: Option<(String, mirix_engine_protocol::Configuration)>,
}

impl ProcessEngine {
    pub fn spawn(method: InputMethod) -> Result<Self> {
        let config = dirs::config_dir()
            .context("input method config directory")?
            .join("mirix/input-methods")
            .join(&method.name);
        let data = dirs::data_dir()
            .context("input method data directory")?
            .join("mirix/input-method-data")
            .join(&method.name);
        let cache = dirs::cache_dir()
            .context("input method cache directory")?
            .join("mirix/input-methods")
            .join(&method.name);
        let mut child = Command::new(&method.executable)
            .current_dir(&method.directory)
            .env("MIRIX_ENGINE_NAME", &method.name)
            .env("MIRIX_CONFIG_DIR", config)
            .env("MIRIX_DATA_DIR", data)
            .env("MIRIX_CACHE_DIR", cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("start input method {}", method.executable.display()))?;
        let stdin = child.stdin.take().context("input method stdin")?;
        let stdout = child.stdout.take().context("input method stdout")?;
        let mut engine = Self {
            icon: None,
            configuration_revision: 0,
            name: method.name,
            child: Some(child),
            reaper: None,
            stdin: Some(stdin),
            stdout: Some(stdout),
            read_buffer: Vec::new(),
            candidates: Arc::default(),
            selected: 0,
            page_ends: Arc::default(),
            failed: false,
            reset_pending: false,
            failure: None,
            menus: Arc::default(),
            punctuation_mode: None,
            configuration_cache: None,
        };

        crate::transport::nonblocking(engine.stdin.as_ref().unwrap())?;
        crate::transport::nonblocking(engine.stdout.as_ref().unwrap())?;

        match engine.exchange(&EngineRequest::Hello, HANDSHAKE_TIMEOUT)? {
            EngineResponse::Ready {
                configuration_revision,
                menus,
                punctuation_mode,
            } => {
                engine.configuration_revision = configuration_revision;
                engine.menus = Arc::new(menus);
                engine.punctuation_mode = punctuation_mode;
                Ok(engine)
            }
            response => bail!("unexpected input method handshake: {response:?}"),
        }
    }

    fn exchange(&mut self, request: &EngineRequest, timeout: Duration) -> Result<EngineResponse> {
        self.configuration_cache = None;
        if self.reset_pending {
            self.reset_pending = false;
            match self.exchange(&EngineRequest::Reset, REQUEST_TIMEOUT)? {
                EngineResponse::ResetDone {
                    configuration_revision,
                    menus,
                    punctuation_mode,
                } => {
                    self.configuration_revision = configuration_revision;
                    self.menus = Arc::new(menus);
                    self.punctuation_mode = punctuation_mode;
                }
                _ => bail!("unexpected reset response"),
            }
        }
        let started = Instant::now();
        let deadline = started + timeout;
        let frame = encode_frame(request)?;
        let result = (|| {
            crate::transport::write_frame(
                self.stdin.as_mut().context("input method stdin closed")?,
                &frame,
                deadline,
            )?;
            loop {
                if Instant::now() >= deadline {
                    bail!("input method response timed out");
                }
                let response = crate::transport::read_response(
                    self.stdout.as_mut().context("input method stdout closed")?,
                    &mut self.read_buffer,
                    deadline,
                )?;
                if let EngineResponse::IconChanged { icon } = response {
                    if let Some(icon) = &icon {
                        icon.validate().map_err(anyhow::Error::msg)?;
                    }
                    self.icon = icon.map(std::sync::Arc::new);
                } else {
                    break Ok(response);
                }
            }
        })();
        let elapsed = started.elapsed();
        // Never record request payloads: key text and vocabulary are user data.
        log::debug!(
            "engine exchange engine={} elapsed_us={} success={}",
            self.name,
            elapsed.as_micros() as u64,
            result.is_ok()
        );
        if elapsed > Duration::from_millis(16) {
            log::warn!(
                "slow engine exchange engine={} elapsed_ms={}",
                self.name,
                elapsed.as_millis() as u64
            );
        }
        if let Ok(response) = &result {
            response.validate().map_err(anyhow::Error::msg)?;
        }
        result
    }

    fn update_state(&mut self, candidates: Vec<Candidate>, selected: u32) {
        if self.candidates.as_ref() != &candidates {
            self.page_ends = Arc::default();
            self.candidates = Arc::new(candidates);
        }
        self.selected = selected as usize;
    }

    fn configuration_request(
        &mut self,
        request: EngineRequest,
    ) -> Result<
        (
            mirix_engine_protocol::Configuration,
            ActionResult<Option<String>>,
        ),
        String,
    > {
        if self.failed {
            return Err("input method process is unavailable".into());
        }
        match self.exchange(&request, Duration::from_secs(10)) {
            Ok(EngineResponse::Configuration {
                configuration,
                message,
                output,
                revision,
                menus,
                punctuation_mode,
                candidates,
                selected,
            }) => {
                configuration.validate()?;
                self.configuration_revision = revision;
                self.menus = Arc::new(menus);
                self.punctuation_mode = punctuation_mode;
                self.update_state(candidates, selected);
                Ok((configuration, ActionResult::new(message, output)))
            }
            Ok(EngineResponse::Error { message }) => Err(message),
            result => {
                let error = match result {
                    Ok(response) => {
                        anyhow::anyhow!("unexpected configuration response: {response:?}")
                    }
                    Err(error) => error,
                };
                let message = error.to_string();
                self.fail(error);
                Err(message)
            }
        }
    }

    fn fail(&mut self, error: anyhow::Error) {
        if !self.failed {
            log::error!(
                "input method process disabled engine={} error={:?}",
                self.name,
                error
            );
        }
        self.failed = true;
        self.failure = Some(error.to_string());
        self.icon = None;
        self.menus = Arc::default();
        self.configuration_cache = None;
        self.configuration_revision = self.configuration_revision.wrapping_add(1);
        self.punctuation_mode = None;
        self.candidates = Arc::default();
        self.selected = 0;
        self.stdin.take();
        self.stdout.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            self.reaper = Some(std::thread::spawn(move || {
                let _ = child.wait();
            }));
        }
    }
}

impl Drop for ProcessEngine {
    fn drop(&mut self) {
        // Read-only snapshots own no process. Dropping one only releases its
        // shared lists; it need not allocate empty replacement state first.
        if (self.child.is_some() || self.reaper.is_some())
            && let Some(cleanup) = self.retire()
        {
            cleanup();
        }
    }
}

impl Engine for ProcessEngine {
    fn tray_icon(&self) -> Option<std::sync::Arc<mirix_engine_protocol::EngineIcon>> {
        self.icon.clone()
    }
    fn take_worker(&mut self) -> Option<Box<dyn Engine + Send>> {
        if self.failed {
            return None;
        }
        let snapshot = Self {
            icon: self.icon.clone(),
            name: self.name.clone(),
            configuration_revision: self.configuration_revision,
            child: None,
            reaper: None,
            stdin: None,
            stdout: None,
            read_buffer: vec![],
            candidates: self.candidates.clone(),
            selected: self.selected,
            page_ends: self.page_ends.clone(),
            failed: true,
            failure: self.failure.clone(),
            reset_pending: false,
            menus: self.menus.clone(),
            punctuation_mode: self.punctuation_mode,
            configuration_cache: None,
        };
        Some(Box::new(std::mem::replace(self, snapshot)))
    }

    fn reset_pending(&self) -> bool {
        self.reset_pending
    }

    fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    fn retire(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        self.icon = None;
        self.failed = true;
        self.configuration_cache = None;
        self.failure = Some("input method stopped".into());
        self.menus = Arc::default();
        self.configuration_revision = self.configuration_revision.wrapping_add(1);
        self.candidates = Arc::default();
        self.page_ends = Arc::default();
        self.selected = 0;
        self.punctuation_mode = None;
        self.stdin.take();
        self.stdout.take();
        if let Some(reaper) = self.reaper.take() {
            return Some(Box::new(move || {
                let _ = reaper.join();
            }));
        }
        self.child.take().map(|mut child| {
            Box::new(move || {
                // EOF lets learning writes finish before the replacement opens its files.
                let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
                loop {
                    match child.try_wait() {
                        Ok(Some(_)) => return,
                        Err(_) => break,
                        Ok(None) if Instant::now() >= deadline => break,
                        Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
            }) as Box<dyn FnOnce() + Send>
        })
    }

    fn configuration_revision(&self) -> u64 {
        self.configuration_revision
    }

    fn configuration(
        &mut self,
        locale: &str,
    ) -> Result<mirix_engine_protocol::Configuration, String> {
        if let Some((cached_locale, configuration)) = self.configuration_cache.take()
            && cached_locale == locale
        {
            return Ok(configuration);
        }
        self.configuration_request(EngineRequest::GetConfiguration {
            locale: locale.into(),
        })
        .map(|(configuration, _)| configuration)
    }

    fn configure(
        &mut self,
        locale: &str,
        revision: u64,
        event: mirix_engine_protocol::ConfigurationEvent,
    ) -> Result<ActionResult<Option<String>>, String> {
        self.configuration_request(EngineRequest::Configure {
            locale: locale.into(),
            revision,
            event,
        })
        .map(|(configuration, message)| {
            self.configuration_cache = Some((locale.into(), configuration));
            message
        })
    }
    fn vocabulary(
        &mut self,
        profile: Option<&str>,
        operation: mirix_engine_protocol::VocabularyOperation,
    ) -> Result<ActionResult<mirix_engine_protocol::VocabularyResult>, String> {
        if self.failed {
            return Err("input method process is unavailable".into());
        }
        match self.exchange(
            &EngineRequest::Vocabulary {
                profile: profile.map(str::to_owned),
                operation,
            },
            REQUEST_TIMEOUT,
        ) {
            Ok(EngineResponse::VocabularyState {
                configuration_revision,
                menus,
                result,
                output,
                candidates,
                selected,
            }) => {
                self.configuration_revision = configuration_revision;
                self.menus = Arc::new(menus);
                self.update_state(candidates, selected);
                Ok(ActionResult::new(result, output))
            }
            Ok(EngineResponse::Error { message }) => Err(message),
            Ok(response) => {
                let message = format!("unexpected vocabulary response: {response:?}");
                self.fail(anyhow::anyhow!(message.clone()));
                Err(message)
            }
            Err(error) => {
                let message = error.to_string();
                self.fail(error);
                Err(message)
            }
        }
    }

    fn punctuation_mode(&self) -> Option<PunctuationMode> {
        self.punctuation_mode
    }
    fn menus(&self) -> Vec<EngineMenu> {
        self.menus.as_ref().clone()
    }

    fn select_menu(&mut self, menu: &str, entry: &str) -> Result<EngineOutput, String> {
        if self.failed {
            return Err("input method process is unavailable".into());
        }
        match self.exchange(
            &EngineRequest::SelectMenu {
                menu: menu.to_owned(),
                entry: entry.to_owned(),
            },
            REQUEST_TIMEOUT,
        ) {
            Ok(EngineResponse::MenuState {
                configuration_revision,
                menus,
                punctuation_mode,
                output,
                candidates,
                selected,
            }) => {
                self.configuration_revision = configuration_revision;
                self.menus = Arc::new(menus);
                self.punctuation_mode = punctuation_mode;
                self.update_state(candidates, selected);
                Ok(output)
            }
            Ok(EngineResponse::Error { message }) => Err(message),
            Ok(response) => {
                let message = format!("unexpected menu response: {response:?}");
                self.fail(anyhow::anyhow!(message.clone()));
                Err(message)
            }
            Err(error) => {
                let message = error.to_string();
                self.fail(error);
                Err(message)
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn process_key(&mut self, event: &KeyEvent) -> EngineOutput {
        if self.failed {
            return EngineOutput::pass();
        }
        match self.exchange(
            &EngineRequest::ProcessKey {
                event: event.clone(),
            },
            REQUEST_TIMEOUT,
        ) {
            Ok(EngineResponse::State {
                configuration_revision,
                menus,
                punctuation_mode,
                output,
                candidates,
                selected,
            }) => {
                self.configuration_revision = configuration_revision;
                self.menus = Arc::new(menus);
                self.punctuation_mode = punctuation_mode;
                self.update_state(candidates, selected);
                output
            }
            Ok(EngineResponse::Error { message }) => {
                log::warn!(
                    "input method rejected key engine={} message={}",
                    self.name,
                    message
                );
                EngineOutput::pass()
            }
            Ok(response) => {
                self.fail(anyhow::anyhow!("unexpected key response: {response:?}"));
                EngineOutput::clear_preedit_and_pass()
            }
            Err(error) => {
                self.fail(error);
                EngineOutput::clear_preedit_and_pass()
            }
        }
    }

    fn reset(&mut self) {
        self.configuration_cache = None;
        self.page_ends = Arc::default();
        self.candidates = Arc::default();
        self.selected = 0;
        // Reset is ordered before the next exchange, without waiting on a focus callback.
        self.reset_pending = true;
    }

    fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    fn selected_candidate(&self) -> usize {
        self.selected
    }

    fn set_candidate_pages(&mut self, ends: Vec<usize>) {
        if self.failed {
            return;
        }
        match self.exchange(
            &EngineRequest::SetCandidatePages { ends: ends.clone() },
            REQUEST_TIMEOUT,
        ) {
            Ok(EngineResponse::PagesSet) => self.page_ends = Arc::new(ends),
            Ok(response) => self.fail(anyhow::anyhow!(
                "unexpected pagination response: {response:?}"
            )),
            Err(error) => self.fail(error),
        }
    }

    fn candidate_page_ends(&self) -> &[usize] {
        &self.page_ends
    }

    fn select(&mut self, id: CandidateId) -> EngineOutput {
        if self.failed {
            return EngineOutput::clear_preedit();
        }
        match self.exchange(&EngineRequest::Select { id }, REQUEST_TIMEOUT) {
            Ok(EngineResponse::State {
                configuration_revision,
                menus,
                punctuation_mode,
                output,
                candidates,
                selected,
            }) => {
                self.configuration_revision = configuration_revision;
                self.menus = Arc::new(menus);
                self.punctuation_mode = punctuation_mode;
                self.update_state(candidates, selected);
                output
            }
            Ok(EngineResponse::Error { message }) => {
                log::warn!(
                    "input method rejected selection engine={} message={}",
                    self.name,
                    message
                );
                EngineOutput::consume()
            }
            Ok(response) => {
                self.fail(anyhow::anyhow!(
                    "unexpected selection response: {response:?}"
                ));
                EngineOutput::clear_preedit()
            }
            Err(error) => {
                self.fail(error);
                EngineOutput::clear_preedit()
            }
        }
    }
}
