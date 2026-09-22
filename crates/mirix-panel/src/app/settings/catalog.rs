//! Inventory state is separate from engine-owned settings pages.
use super::{Settings, repaint};
use crate::i18n::UiLanguage;
use gpui::App;
use mirix_ipc::{InputMethodCatalog, Request};

pub(super) fn receive(catalog: InputMethodCatalog, cx: &mut App) {
    let state = cx.global_mut::<Settings>();
    if state.catalog.as_ref().is_some_and(|current| {
        current.session == catalog.session && current.revision > catalog.revision
    }) {
        return;
    }
    if !catalog
        .entries
        .iter()
        .any(|entry| Some(&entry.name) == state.selected.as_ref())
    {
        state.selected = catalog.entries.first().map(|entry| entry.name.clone());
    }
    state.catalog = Some(catalog);
}

pub(in crate::app) fn refresh_catalog(cx: &mut App) {
    if cx.global::<Settings>().mock.is_some() {
        super::mock::reload(cx);
        return;
    }
    send(cx, |request, _| {
        Some(Request::RefreshInputMethods { request })
    });
}

pub(in crate::app) fn set_enabled(name: String, enabled: bool, cx: &mut App) {
    if cx.global::<Settings>().mock.is_some() {
        super::mock::set_enabled(&name, enabled, cx);
        return;
    }
    send(cx, |request, state| {
        let catalog = state.catalog.as_ref()?;
        if catalog.selection_locked {
            return None;
        }
        Some(Request::SetInputMethodEnabled {
            request,
            session: catalog.session.clone(),
            revision: catalog.revision,
            name,
            enabled,
        })
    });
}

pub(super) fn send(cx: &mut App, request: impl FnOnce(u64, &Settings) -> Option<Request>) {
    let unavailable = cx
        .global::<UiLanguage>()
        .localizer
        .text("settings-unavailable");
    let state = cx.global_mut::<Settings>();
    if !state.connected || state.catalog_pending.is_some() {
        return;
    }
    state.next_request += 1;
    let id = state.next_request;
    let Some(request) = request(id, state) else {
        return;
    };
    state.catalog_pending = Some(id);
    state.catalog_error = None;
    if state
        .commands
        .as_ref()
        .is_none_or(|commands| commands.send(request).is_err())
    {
        state.catalog_pending = None;
        state.catalog_error = Some(unavailable);
    }
    repaint(cx);
}

pub(super) fn restart(name: String, cx: &mut App) {
    send(cx, |request, _| {
        Some(Request::RestartInputMethod { request, name })
    });
}
