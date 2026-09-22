//! Shared configuration state for the tray and the separate settings window.
mod catalog;
pub mod demo;
mod feedback;
mod form;
mod mock;
mod pages;
mod preferences;
mod shortcuts;
mod sidebar;
mod style;
#[cfg(test)]
mod tests;
mod view;
mod vocabulary;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Section {
    #[default]
    Methods,
    Language,
    Skins,
    Shortcuts,
}
pub(super) use catalog::refresh_catalog;

use super::services::PanelEvent;
use crate::i18n::UiLanguage;
use gpui::{App, Global, WindowHandle};
use mirix_core::{ConfigurationEvent, ConfigurationStatus, EngineConfiguration};
use mirix_ipc::Request;
use std::{collections::BTreeMap, path::PathBuf, sync::mpsc::Sender};

#[derive(Default)]
pub(super) struct Entry {
    pub page: Option<EngineConfiguration>,
    pub pending: Option<(u64, bool)>,
    pub error: Option<String>,
    pending_text_reset: Option<String>,
    pub text_reset: Option<(u64, String)>,
}

pub(super) struct Settings {
    mock: Option<mock::Workspace>,
    pub catalog: Option<mirix_ipc::InputMethodCatalog>,
    pub catalog_pending: Option<u64>,
    pub catalog_error: Option<String>,
    catalog_scanned: bool,
    commands: Option<Sender<Request>>,
    pub socket: PathBuf,
    pub statuses: Vec<ConfigurationStatus>,
    pub entries: BTreeMap<String, Entry>,
    pub selected: Option<String>,
    pub connected: bool,
    next_request: u64,
    window: Option<WindowHandle<view::SettingsView>>,
}
impl Global for Settings {}

#[derive(Debug, PartialEq, Eq)]
enum Completion {
    Ignored,
    Refresh,
    Done,
}

impl Settings {
    fn complete(
        &mut self,
        request: u64,
        input_method: &str,
        locale: &str,
        result: &Result<EngineConfiguration, String>,
    ) -> Completion {
        let Some(entry) = self.entries.get_mut(input_method) else {
            return Completion::Ignored;
        };
        let Some((pending, mutation)) = entry.pending else {
            return Completion::Ignored;
        };
        if pending != request {
            return Completion::Ignored;
        }
        entry.pending = None;
        let text_reset = entry.pending_text_reset.take();
        match result {
            Ok(page)
                if page.locale == locale
                    && page.status.input_method == input_method
                    && self.statuses.iter().any(|status| {
                        status.input_method == input_method
                            && status.session == page.status.session
                            && status.revision <= page.status.revision
                    }) =>
            {
                if let Some(status) = self
                    .statuses
                    .iter_mut()
                    .find(|s| s.input_method == input_method)
                {
                    *status = page.status.clone();
                }
                entry.page = Some(page.clone());
                if mutation {
                    entry.error = None;
                }
                if mutation && let Some(id) = text_reset {
                    entry.text_reset = Some((request, id));
                }
                Completion::Done
            }
            Ok(_) => {
                entry.page = None;
                Completion::Refresh
            }
            Err(error) => {
                entry.error = Some(error.clone());
                entry.page = None;
                if mutation {
                    Completion::Refresh
                } else {
                    Completion::Done
                }
            }
        }
    }
}

pub(super) fn install(commands: Option<Sender<Request>>, socket: PathBuf, cx: &mut App) {
    cx.set_global(Settings {
        mock: None,
        catalog: None,
        catalog_pending: None,
        catalog_error: None,
        catalog_scanned: false,
        commands,
        socket,
        statuses: vec![],
        entries: BTreeMap::new(),
        selected: None,
        connected: false,
        next_request: 0,
        window: None,
    });
}

pub(super) fn handle(event: &PanelEvent, cx: &mut App) -> bool {
    match event {
        PanelEvent::Catalog(catalog) => {
            catalog::receive(catalog.clone(), cx);
            if !cx.global::<Settings>().catalog_scanned {
                cx.global_mut::<Settings>().catalog_scanned = true;
                refresh_catalog(cx);
            }
        }
        PanelEvent::CatalogFinished { request, result } => {
            if cx.global::<Settings>().catalog_pending != Some(*request) {
                return false;
            }
            cx.global_mut::<Settings>().catalog_pending = None;
            match result {
                Ok(catalog) => {
                    catalog::receive(catalog.clone(), cx);
                }
                Err(error) => {
                    cx.global_mut::<Settings>().catalog_error = Some(error.clone());
                    feedback(false, error.clone(), cx);
                }
            }
        }
        PanelEvent::InputMethod {
            configurations,
            name,
            ..
        } => {
            let state = cx.global_mut::<Settings>();
            state.connected = true;
            // Preserve a newer command response when its subscription update is queued behind it.
            state.statuses = configurations
                .iter()
                .map(|status| {
                    state
                        .statuses
                        .iter()
                        .find(|old| {
                            old.input_method == status.input_method
                                && old.session == status.session
                                && old.revision > status.revision
                        })
                        .cloned()
                        .unwrap_or_else(|| status.clone())
                })
                .collect();
            state.entries.retain(|name, _| {
                configurations
                    .iter()
                    .any(|status| &status.input_method == name)
            });
            if !state.catalog.as_ref().is_some_and(|catalog| {
                catalog
                    .entries
                    .iter()
                    .any(|entry| Some(&entry.name) == state.selected.as_ref())
            }) && !state
                .statuses
                .iter()
                .any(|s| Some(&s.input_method) == state.selected.as_ref())
            {
                state.selected = state
                    .statuses
                    .iter()
                    .find(|s| s.input_method == *name)
                    .or(state.statuses.first())
                    .map(|s| s.input_method.clone());
            }
            refresh_missing(cx);
        }
        PanelEvent::ConfigurationFinished {
            request,
            input_method,
            result,
        } => {
            let locale = cx.global::<UiLanguage>().localizer.language().to_string();
            let mutation = cx
                .global::<Settings>()
                .entries
                .get(input_method)
                .and_then(|entry| entry.pending)
                .is_some_and(|(pending, mutation)| pending == *request && mutation);
            // Never replay mutations, including those whose response timed out.
            // One read refreshes authoritative state after a failed mutation.
            let completion =
                cx.global_mut::<Settings>()
                    .complete(*request, input_method, &locale, result);
            if completion != Completion::Ignored {
                match result {
                    Ok(page) if mutation && completion == Completion::Done => {
                        feedback(true, page.message.clone().unwrap_or_default(), cx);
                    }
                    Err(message) => feedback(false, message.clone(), cx),
                    _ => {}
                }
            }
            match completion {
                Completion::Ignored => return false,
                Completion::Refresh => query(input_method, false, cx),
                Completion::Done => {
                    if result.is_ok() {
                        refresh_missing(cx);
                    }
                }
            }
        }
        PanelEvent::Disconnected => {
            let state = cx.global_mut::<Settings>();
            state.catalog = None;
            state.catalog_pending = None;
            state.catalog_scanned = false;
            state.catalog_error = None;
            state.connected = false;
            state.entries.clear();
            state.statuses.clear();
        }
        _ => return false,
    }
    repaint(cx);
    true
}

fn refresh_missing(cx: &mut App) {
    let locale = cx.global::<UiLanguage>().localizer.language().to_string();
    let state = cx.global::<Settings>();
    let missing = state
        .statuses
        .iter()
        .filter(|status| {
            state.entries.get(&status.input_method).is_none_or(|entry| {
                entry.pending.is_none()
                    && (entry.error.is_none() || entry.page.is_some())
                    && entry
                        .page
                        .as_ref()
                        .is_none_or(|page| page.status != **status || page.locale != locale)
            })
        })
        .map(|s| s.input_method.clone())
        .collect::<Vec<_>>();
    for name in missing {
        query(&name, false, cx);
    }
}

pub(super) fn query(name: &str, clear_error: bool, cx: &mut App) {
    if cx.global::<Settings>().mock.is_some() {
        mock::query(name, clear_error, cx);
        return;
    }
    let locale = cx.global::<UiLanguage>().localizer.language().to_string();
    let unavailable = cx
        .global::<UiLanguage>()
        .localizer
        .text("settings-unavailable");
    let state = cx.global_mut::<Settings>();
    if !state.connected || !state.statuses.iter().any(|s| s.input_method == name) {
        return;
    }
    let entry = state.entries.entry(name.into()).or_default();
    if entry.pending.is_some() {
        return;
    }
    state.next_request += 1;
    let request = state.next_request;
    entry.pending = Some((request, false));
    if clear_error {
        entry.error = None;
    }
    if state.commands.as_ref().is_none_or(|sender| {
        sender
            .send(Request::GetEngineConfiguration {
                request,
                input_method: name.into(),
                locale,
            })
            .is_err()
    }) {
        entry.pending = None;
        entry.error = Some(unavailable.clone());
        feedback(false, unavailable, cx);
    }
    repaint(cx);
}

pub(super) fn submit(page: &EngineConfiguration, event: ConfigurationEvent, cx: &mut App) {
    if cx.global::<Settings>().mock.is_some() {
        mock::submit(page, event, cx);
        return;
    }
    let unavailable = cx
        .global::<UiLanguage>()
        .localizer
        .text("settings-unavailable");
    let state = cx.global_mut::<Settings>();
    if !state.connected {
        return;
    }
    let Some(entry) = state.entries.get_mut(&page.status.input_method) else {
        return;
    };
    if entry.pending.is_some() || entry.page.as_ref() != Some(page) {
        return;
    }
    if let Err(error) = page.page.validate_event(&event) {
        entry.error = Some(error.clone());
        feedback(false, error, cx);
        repaint(cx);
        return;
    }
    state.next_request += 1;
    let request = state.next_request;
    entry.pending = Some((request, true));
    entry.pending_text_reset = form::text_reset(&page.page, &event);
    entry.error = None;
    if state.commands.as_ref().is_none_or(|sender| {
        sender
            .send(Request::ConfigureEngine {
                request,
                status: page.status.clone(),
                locale: page.locale.clone(),
                event,
            })
            .is_err()
    }) {
        entry.pending = None;
        entry.pending_text_reset = None;
        entry.error = Some(unavailable.clone());
        feedback(false, unavailable, cx);
    }
    repaint(cx);
}

pub(super) fn language_changed(cx: &mut App) {
    cx.global_mut::<Settings>().entries.clear();
    refresh_missing(cx);
    repaint(cx);
}

fn feedback(success: bool, message: String, cx: &mut App) {
    let i18n = cx.global::<UiLanguage>().localizer.clone();
    feedback::show(success, message, &i18n, cx);
}

fn repaint(cx: &mut App) {
    if let Some(window) = cx.global::<Settings>().window {
        let _ = window.update(cx, |_, window, _| window.refresh());
    }
}

pub(crate) fn run(socket: PathBuf, language: Option<String>) -> anyhow::Result<()> {
    use super::services::Services;
    let config = crate::config::default_path()?;
    let preference = crate::config::resolve_language(&config, language)?;
    let directories = crate::i18n::directories(Some(&config));
    let localizer = std::sync::Arc::new(mirix_i18n::Localizer::load(
        preference.as_deref(),
        &directories,
    )?);
    let (services, events) = Services::start(Some(socket.clone()), false)?;
    gpui_platform::application()
        .with_assets(uic::assets::LucideAssets)
        .run(move |cx| {
            uic::init(cx);
            cx.set_global(UiLanguage {
                localizer,
                preference,
                directories,
            });
            install(services.commands(), socket, cx);
            preferences::install(Some(config), cx);
            view::open(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.spawn(async move |cx| {
                let _services = services;
                while let Ok(event) = events.recv().await {
                    cx.update(|cx| {
                        if let PanelEvent::UiPreferenceChanged(preference) = event {
                            preferences::reload(preference, cx);
                            return;
                        }
                        handle(&event, cx);
                    });
                }
            })
            .detach();
        });
    Ok(())
}
