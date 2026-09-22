//! Settings-owned preferences. Explicit notifications synchronize separate UI processes.
use super::super::{
    client,
    skin_catalog::{self, SkinEntry},
};
use super::Settings;
use crate::i18n::UiLanguage;
use gpui::{App, Global};
use mirix_i18n::Localizer;
use mirix_ipc::{Request, UiPreference};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct Preferences {
    pub config: Option<PathBuf>,
    pub skins: Vec<SkinEntry>,
    pub active_skin: Option<PathBuf>,
    pub animations: BTreeMap<String, bool>,
    pub busy: bool,
    pub error: Option<String>,
    generation: u64,
}
impl Global for Preferences {}

pub(super) fn install(config: Option<PathBuf>, cx: &mut App) {
    let result = config
        .as_ref()
        .map(|path| crate::config::load_skin(Some(path), None).map(|(_, path)| path))
        .transpose();
    let (active_skin, mut error) = match result {
        Ok(path) => (canonical(path.flatten()), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let animations = match config
        .as_deref()
        .map(crate::config::animation::read)
        .transpose()
    {
        Ok(values) => values.unwrap_or_default(),
        Err(problem) => {
            error = Some(format!("{problem:#}"));
            BTreeMap::new()
        }
    };
    let skins = scan(config.as_deref(), active_skin.as_deref());
    cx.set_global(Preferences {
        config,
        skins,
        active_skin,
        animations,
        busy: false,
        error,
        generation: 0,
    });
}

fn canonical(path: Option<PathBuf>) -> Option<PathBuf> {
    crate::config::canonical_skin(path)
}

fn scan(config: Option<&Path>, active: Option<&Path>) -> Vec<SkinEntry> {
    if config.is_some() {
        skin_catalog::scan(active)
    } else {
        // Offline previews use the built-in skin without scanning the filesystem.
        skin_catalog::discover(&[], None)
    }
}

pub(super) fn refresh_skins(cx: &mut App) {
    let state = cx.global_mut::<Preferences>();
    state.skins = scan(state.config.as_deref(), state.active_skin.as_deref());
    super::repaint(cx);
}

pub(super) fn select_language(preference: Option<String>, persist: bool, cx: &mut App) {
    let directories = cx.global::<UiLanguage>().directories.clone();
    let state = cx.global_mut::<Preferences>();
    state.generation += 1;
    let generation = state.generation;
    state.busy = true;
    state.error = None;
    let config = state.config.clone().filter(|_| persist);
    super::repaint(cx);
    cx.spawn(async move |cx| {
        let requested = preference.clone();
        let roots = directories.clone();
        let result = cx
            .background_executor()
            .spawn(async move { Localizer::load(requested.as_deref(), &roots) })
            .await;
        cx.update(|cx| {
            let state = cx.global_mut::<Preferences>();
            if generation != state.generation {
                return;
            }
            state.busy = false;
            match result {
                Ok(localizer) => {
                    if let Some(path) = &config
                        && let Err(error) =
                            crate::config::save_language(path, preference.as_deref())
                    {
                        state.error = Some(format!("{error:#}"));
                        super::repaint(cx);
                        return;
                    }
                    cx.set_global(UiLanguage {
                        localizer: Arc::new(localizer),
                        preference,
                        directories,
                    });
                    if cx.global::<Settings>().mock.is_some() {
                        super::mock::sync(cx);
                    } else {
                        super::language_changed(cx);
                    }
                    if config.is_some() {
                        notify(UiPreference::Language, cx);
                    }
                }
                Err(error) => cx.global_mut::<Preferences>().error = Some(format!("{error:#}")),
            }
            super::repaint(cx);
        });
    })
    .detach();
}

pub(super) fn apply_skin(path: Option<PathBuf>, cx: &mut App) {
    let state = cx.global_mut::<Preferences>();
    if state.busy {
        return;
    }
    let (_, path) = crate::config::load_selected_skin(path);
    state.error = None;
    if let Some(config) = &state.config
        && let Err(error) = crate::config::save_skin(config, path.as_deref())
    {
        state.error = Some(format!("{error:#}"));
        super::repaint(cx);
        return;
    }
    state.active_skin = path;
    if state.config.is_some() {
        notify(UiPreference::Skin, cx);
    }
    super::repaint(cx);
}

pub(super) fn set_skin_animations(key: String, enabled: bool, cx: &mut App) {
    let state = cx.global_mut::<Preferences>();
    state.error = None;
    if let Some(config) = &state.config
        && let Err(error) = crate::config::animation::save(config, &key, enabled)
    {
        state.error = Some(format!("{error:#}"));
        super::repaint(cx);
        return;
    }
    state.animations.insert(key, enabled);
    if state.config.is_some() {
        notify(UiPreference::SkinAnimations, cx);
    }
    super::repaint(cx);
}

fn notify(preference: UiPreference, cx: &mut App) {
    let socket = cx.global::<Settings>().socket.clone();
    cx.spawn(async move |cx| {
        let result = cx
            .background_executor()
            .spawn(async move {
                client::exchange(&socket, &Request::UiPreferenceChanged { preference })
            })
            .await;
        if let Err(error) = result {
            cx.update(|cx| {
                let i18n = &cx.global::<UiLanguage>().localizer;
                cx.global_mut::<Preferences>().error =
                    Some(format!("{}: {error:#}", i18n.text("settings-sync-failed")));
                super::repaint(cx);
            });
        }
    })
    .detach();
}

pub(super) fn reload(preference: UiPreference, cx: &mut App) {
    let Some(config) = cx.global::<Preferences>().config.clone() else {
        return;
    };
    let result = match preference {
        UiPreference::SkinAnimations => crate::config::animation::read(&config).map(|values| {
            cx.global_mut::<Preferences>().animations = values;
        }),
        UiPreference::Language => crate::config::resolve_language(&config, None).map(|value| {
            select_language(value, false, cx);
        }),
        UiPreference::Skin => crate::config::load_skin(Some(&config), None).map(|(_, value)| {
            cx.global_mut::<Preferences>().active_skin = canonical(value);
            refresh_skins(cx);
        }),
    };
    if let Err(error) = result {
        cx.global_mut::<Preferences>().error = Some(format!("{error:#}"));
    }
    super::repaint(cx);
}
