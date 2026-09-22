//! In-memory data source for the real management view. Never starts services.
#[cfg(test)]
mod tests;

use super::{Settings, demo::Document, install, repaint, view};
use crate::i18n::UiLanguage;
use anyhow::{Context as _, ensure};
use gpui::App;
use mirix_core::{ConfigurationEvent, ConfigurationStatus, EngineConfiguration};
use mirix_ipc::{InputMethodCatalog, InstalledInputMethod};
use serde::Deserialize;
use std::{collections::HashSet, path::PathBuf, sync::Arc};

#[derive(Deserialize)]
struct Manifest {
    selected: String,
    input_methods: Vec<MethodFile>,
}

#[derive(Deserialize)]
struct MethodFile {
    id: String,
    enabled: bool,
    page: PathBuf,
}

struct Method {
    id: String,
    enabled: bool,
    document: Document,
}

pub(super) struct Workspace {
    directory: PathBuf,
    selected: String,
    revision: u64,
    methods: Vec<Method>,
}

impl Workspace {
    fn load(directory: PathBuf) -> anyhow::Result<Self> {
        let path = directory.join("workspace.json");
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("read mock workspace {}", path.display()))?;
        let manifest: Manifest = serde_json::from_str(&source)
            .with_context(|| format!("parse mock workspace {}", path.display()))?;
        let mut ids = HashSet::new();
        let mut methods = Vec::new();
        for entry in manifest.input_methods {
            ensure!(
                !entry.id.is_empty() && ids.insert(entry.id.clone()),
                "mock input-method IDs must be unique and nonempty: {}",
                entry.id
            );
            methods.push(Method {
                id: entry.id,
                enabled: entry.enabled,
                document: Document::load(directory.join(entry.page))?,
            });
        }
        ensure!(
            ids.contains(&manifest.selected),
            "selected mock input method does not exist: {}",
            manifest.selected
        );
        Ok(Self {
            directory,
            selected: manifest.selected,
            revision: 0,
            methods,
        })
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        let mut next = Self::load(self.directory.clone())?;
        next.revision = self.revision + 1;
        *self = next;
        Ok(())
    }

    fn page(&self, name: &str, locale: &str) -> Option<EngineConfiguration> {
        self.methods
            .iter()
            .find(|method| method.id == name)
            .map(|method| EngineConfiguration {
                message: None,
                status: ConfigurationStatus {
                    input_method: name.into(),
                    session: "offline-preview".into(),
                    revision: self.revision,
                },
                locale: locale.into(),
                page: method.document.page.clone(),
            })
    }

    fn apply(
        &mut self,
        page: &EngineConfiguration,
        event: &ConfigurationEvent,
    ) -> Result<(), String> {
        if self.page(&page.status.input_method, &page.locale).as_ref() != Some(page) {
            return Err("preview changed; use the current page".into());
        }
        self.methods
            .iter_mut()
            .find(|method| method.id == page.status.input_method)
            .expect("validated mock method")
            .document
            .apply(event)?;
        self.revision += 1;
        Ok(())
    }

    fn reload_page(&mut self, name: &str) -> anyhow::Result<()> {
        let method = self
            .methods
            .iter_mut()
            .find(|method| method.id == name)
            .with_context(|| format!("unknown mock method: {name}"))?;
        method.document.reload()?;
        self.revision += 1;
        Ok(())
    }

    fn set_enabled(&mut self, name: &str, enabled: bool) -> anyhow::Result<()> {
        let method = self
            .methods
            .iter_mut()
            .find(|method| method.id == name)
            .with_context(|| format!("unknown mock method: {name}"))?;
        method.enabled = enabled;
        self.revision += 1;
        Ok(())
    }

    fn catalog(&self) -> InputMethodCatalog {
        InputMethodCatalog {
            session: "offline-preview".into(),
            revision: self.revision,
            selection_locked: false,
            enabled_order: self
                .methods
                .iter()
                .filter(|method| method.enabled)
                .map(|method| method.id.clone())
                .collect(),
            shortcuts: [("switch", "Ctrl+Space"), ("skin", "Ctrl+Shift+S")]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
            locked_shortcuts: vec![],
            entries: self
                .methods
                .iter()
                .map(|method| InstalledInputMethod {
                    name: method.id.clone(),
                    directory: Some(self.directory.clone()),
                    enabled: method.enabled,
                    loaded: true,
                    error: None,
                })
                .collect(),
        }
    }
}

pub(super) fn sync(cx: &mut App) {
    let locale = cx.global::<UiLanguage>().localizer.language().to_string();
    let state = cx.global_mut::<Settings>();
    let mock = state.mock.as_ref().expect("mock mode");
    state.connected = true;
    state.catalog = Some(mock.catalog());
    state.statuses.clear();
    state
        .entries
        .retain(|id, _| mock.methods.iter().any(|method| &method.id == id));
    for method in &mock.methods {
        let page = mock.page(&method.id, &locale).expect("mock page");
        state.statuses.push(page.status.clone());
        let entry = state.entries.entry(method.id.clone()).or_default();
        entry.page = Some(page);
        entry.pending = None;
    }
    if !mock
        .methods
        .iter()
        .any(|method| Some(&method.id) == state.selected.as_ref())
    {
        state.selected = Some(mock.selected.clone());
    }
}

pub(super) fn query(name: &str, reload_page: bool, cx: &mut App) {
    if reload_page {
        let result = cx
            .global_mut::<Settings>()
            .mock
            .as_mut()
            .expect("mock mode")
            .reload_page(name);
        if let Some(entry) = cx.global_mut::<Settings>().entries.get_mut(name) {
            entry.error = result.as_ref().err().map(|error| format!("{error:#}"));
        }
        if let Err(error) = result {
            super::feedback(false, format!("{error:#}"), cx);
        }
    }
    sync(cx);
    repaint(cx);
}

pub(super) fn submit(page: &EngineConfiguration, event: ConfigurationEvent, cx: &mut App) {
    let message = cx
        .global::<UiLanguage>()
        .localizer
        .text("settings-preview-applied");
    let state = cx.global_mut::<Settings>();
    state.next_request += 1;
    let request = state.next_request;
    println!(
        "{}",
        serde_json::json!({"input_method": page.status.input_method, "event": event})
    );
    let result = cx
        .global_mut::<Settings>()
        .mock
        .as_mut()
        .expect("mock mode")
        .apply(page, &event);
    if let Some(entry) = cx
        .global_mut::<Settings>()
        .entries
        .get_mut(&page.status.input_method)
    {
        if result.is_ok()
            && let Some(id) = super::form::text_reset(&page.page, &event)
        {
            entry.text_reset = Some((request, id));
        }
        entry.error = result.as_ref().err().cloned();
    }
    match result {
        Ok(()) => super::feedback(true, message, cx),
        Err(error) => super::feedback(false, error, cx),
    }
    sync(cx);
    repaint(cx);
}

pub(super) fn reload(cx: &mut App) {
    let state = cx.global_mut::<Settings>();
    match state.mock.as_mut().expect("mock mode").reload() {
        Ok(()) => {
            state.entries.clear();
            state.catalog_error = None;
        }
        Err(error) => state.catalog_error = Some(format!("{error:#}")),
    }
    sync(cx);
    repaint(cx);
}

pub(super) fn set_enabled(name: &str, enabled: bool, cx: &mut App) {
    println!(
        "{}",
        serde_json::json!({"type": "mock_enabled", "input_method": name, "enabled": enabled})
    );
    let state = cx.global_mut::<Settings>();
    state.catalog_error = state
        .mock
        .as_mut()
        .expect("mock mode")
        .set_enabled(name, enabled)
        .err()
        .map(|error| format!("{error:#}"));
    sync(cx);
    repaint(cx);
}

pub(super) fn run(directory: PathBuf, language: Option<String>) -> anyhow::Result<()> {
    let workspace = Workspace::load(directory)?;
    let preference = language.filter(|value| value != "auto");
    let localizer = Arc::new(mirix_i18n::Localizer::load(preference.as_deref(), &[])?);
    gpui_platform::application()
        .with_assets(uic::assets::LucideAssets)
        .run(move |cx| {
            uic::init(cx);
            cx.set_global(UiLanguage {
                localizer,
                preference,
                directories: vec![],
            });
            install(None, PathBuf::new(), cx);
            super::preferences::install(None, cx);
            cx.global_mut::<Settings>().mock = Some(workspace);
            sync(cx);
            view::open(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
    Ok(())
}
