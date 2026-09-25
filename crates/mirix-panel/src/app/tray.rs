//! Native GPUI tray, driven by daemon state and explicit skin actions.
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::i18n::UiLanguage;
use gpui::{Action, App, Global, Menu, MenuItem, TrayId, TrayOptions, actions};
use mirix_core::EngineMenu;
use mirix_i18n::FluentArgs;
use mirix_ipc::Request;

mod icon;
mod language;

use super::popup::PanelWindow;
use super::settings::{self, Settings};
use super::skin_catalog::{self, SkinEntry};

actions!(
    mirix_tray,
    [
        ToggleInputMethod,
        RefreshSkins,
        RefreshLanguages,
        RestartDaemon,
        QuitDaemon,
        OpenSettings
    ]
);

#[derive(Clone, PartialEq, Action)]
#[action(namespace = mirix_tray, no_json)]
struct SelectInputMethod {
    name: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = mirix_tray, no_json)]
struct ConfigureEngine {
    page: mirix_core::EngineConfiguration,
    event: mirix_core::ConfigurationEvent,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = mirix_tray, no_json)]
struct SelectSkin {
    path: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = mirix_tray, no_json)]
struct SelectLanguage {
    language: Option<String>,
}

struct TrayState {
    engine_icon: Option<mirix_core::EngineIcon>,
    id: Option<TrayId>,
    window: PanelWindow,
    commands: Option<Sender<Request>>,
    input_method: Option<String>,
    switch_shortcut: String,
    available: Vec<String>,
    menus: Vec<EngineMenu>,
    skins: Vec<SkinEntry>,
    active_skin: Option<PathBuf>,
    config_path: Option<PathBuf>,
    skin_request: u64,
    language_request: u64,
    loading: bool,
    error: Option<&'static str>,
    settings_open: bool,
}

impl Global for TrayState {}

pub(super) fn install(
    window: PanelWindow,
    commands: Option<Sender<Request>>,
    skin: Option<PathBuf>,
    config_path: Option<PathBuf>,
    cx: &mut App,
) {
    let skin = crate::config::canonical_skin(skin);
    cx.set_global(TrayState {
        engine_icon: None,
        id: None,
        window,
        commands,
        input_method: None,
        switch_shortcut: String::new(),
        available: vec![],
        menus: Vec::new(),
        skins: skin_catalog::scan(skin.as_deref()),
        active_skin: skin,
        config_path,
        skin_request: 0,
        language_request: 0,
        loading: false,
        error: None,
        settings_open: false,
    });
    cx.on_action(|_: &ToggleInputMethod, cx| send(Request::ToggleInputMethod, cx));
    cx.on_action(|_: &RestartDaemon, cx| send(Request::Restart, cx));
    cx.on_action(|_: &QuitDaemon, cx| {
        if cx.global::<TrayState>().commands.is_none() {
            cx.quit(); // Standalone skin demo has no daemon to shut down.
        } else {
            send(Request::Shutdown, cx);
        }
    });
    cx.on_action(|action: &SelectInputMethod, cx| {
        send(
            Request::SetInputMethod {
                name: action.name.clone(),
            },
            cx,
        )
    });
    cx.on_action(|action: &SelectSkin, cx| select_skin(action.path.clone(), true, cx));
    cx.on_action(|action: &SelectLanguage, cx| language::select(action.language.clone(), true, cx));
    cx.on_action(|_: &RefreshLanguages, cx| {
        language::select(cx.global::<UiLanguage>().preference.clone(), false, cx);
    });
    cx.on_action(|action: &ConfigureEngine, cx| {
        settings::submit(&action.page, action.event.clone(), cx);
        refresh(cx);
    });
    cx.on_action(|_: &OpenSettings, cx| open_settings(cx));
    cx.on_action(|_: &RefreshSkins, cx| {
        let state = cx.global_mut::<TrayState>();
        state.skins = skin_catalog::scan(state.active_skin.as_deref());
        publish_skins(cx);
        refresh(cx);
    });
    match cx.create_tray(options(
        cx.global::<TrayState>(),
        cx.global::<UiLanguage>(),
        cx.global::<Settings>(),
    )) {
        Ok(id) => cx.global_mut::<TrayState>().id = Some(id),
        Err(error) => log::warn!("mirix-panel: system tray unavailable: {error:#}"),
    }
}

pub(super) fn input_method(
    icon: Option<mirix_core::EngineIcon>,
    name: String,
    available: Vec<String>,
    menus: Vec<EngineMenu>,
    switch_shortcut: String,
    cx: &mut App,
) {
    let state = cx.global_mut::<TrayState>();
    if state.input_method.as_ref() == Some(&name)
        && state.engine_icon == icon
        && state.available == available
        && state.menus == menus
        && state.switch_shortcut == switch_shortcut
    {
        return;
    }
    state.input_method = (!name.is_empty()).then_some(name);
    state.engine_icon = icon;
    state.switch_shortcut = switch_shortcut;
    state.available = available;
    state.menus = menus;
    publish_skins(cx);
    refresh(cx);
}

fn skin_id(path: Option<&std::path::Path>) -> String {
    path.map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn publish_skins(cx: &mut App) {
    let state = cx.global::<TrayState>();
    let entries = state
        .skins
        .iter()
        .map(|skin| mirix_core::MenuEntry {
            id: skin_id(skin.path.as_deref()),
            label: skin.name.clone(),
        })
        .collect();
    let active = Some(skin_id(state.active_skin.as_deref()));
    send(Request::SetPanelSkins { entries, active }, cx);
}

pub(super) fn apply_skin(id: &str, cx: &mut App) {
    let path = cx
        .global::<TrayState>()
        .skins
        .iter()
        .find(|skin| skin_id(skin.path.as_deref()) == id)
        .map(|skin| skin.path.clone());
    if let Some(path) = path {
        select_skin(path, true, cx);
    }
}

pub(super) fn disconnected(cx: &mut App) {
    let state = cx.global_mut::<TrayState>();
    state.engine_icon = None;
    state.input_method = None;
    state.available.clear();
    state.menus = Vec::new();
    refresh(cx);
}

pub(super) fn reload_preference(preference: mirix_ipc::UiPreference, cx: &mut App) {
    let Some(config) = cx.global::<TrayState>().config_path.clone() else {
        return;
    };
    let result =
        match preference {
            mirix_ipc::UiPreference::SkinAnimations => {
                let state = cx.global::<TrayState>();
                let window = state.window.clone();
                crate::config::animation::enabled(Some(&config), state.active_skin.as_deref())
                    .and_then(|enabled| {
                        window.update(cx, |panel, _, cx| panel.set_animations_enabled(enabled, cx))
                    })
            }
            mirix_ipc::UiPreference::Language => crate::config::resolve_language(&config, None)
                .map(|value| {
                    language::select(value, false, cx);
                }),
            mirix_ipc::UiPreference::Skin => {
                crate::config::load_skin(Some(&config), None).map(|(_, value)| {
                    let value = crate::config::canonical_skin(value);
                    let state = cx.global_mut::<TrayState>();
                    state.skins = skin_catalog::scan(value.as_deref());
                    if state.active_skin != value {
                        select_skin(value, false, cx);
                    }
                })
            }
        };
    if let Err(error) = result {
        log::warn!("mirix-panel: reload preference: {error:#}");
    }
}

fn send(request: Request, cx: &mut App) {
    let state = cx.global::<TrayState>();
    if !cx.global::<Settings>().connected {
        return;
    }
    if let Some(commands) = &state.commands
        && commands.send(request).is_err()
    {
        disconnected(cx);
    }
}

fn select_skin(path: Option<PathBuf>, persist: bool, cx: &mut App) {
    let state = cx.global_mut::<TrayState>();
    if !state.skins.iter().any(|skin| skin.path == path) {
        return;
    }
    state.skin_request += 1;
    let request = state.skin_request;
    let window = state.window.clone();
    state.loading = true;
    state.error = None;
    refresh(cx);
    cx.spawn(async move |cx| {
        let (skin, path) = cx
            .background_executor()
            .spawn(async move { crate::config::load_selected_skin(path) })
            .await;
        cx.update(|cx| {
            if cx.global::<TrayState>().skin_request != request {
                return;
            }
            // Read the latest choice for the effective skin, including fallback.
            let result = crate::config::animation::enabled(
                cx.global::<TrayState>().config_path.as_deref(),
                path.as_deref(),
            )
            .and_then(|enabled| {
                window.update(cx, |panel, window, cx| {
                    panel.set_animations_enabled(enabled, cx);
                    panel.set_skin(skin, path.clone(), window, cx)
                })
            });
            let state = cx.global_mut::<TrayState>();
            state.loading = false;
            match result {
                Ok(()) => {
                    state.active_skin = path;
                    if persist
                        && let Some(config_path) = &state.config_path
                        && let Err(error) =
                            crate::config::save_skin(config_path, state.active_skin.as_deref())
                    {
                        log::warn!("mirix-panel: save skin preference: {error:#}");
                        state.error = Some("skin-save-failed");
                    }
                }
                Err(error) => {
                    log::warn!("mirix-panel: skin switch: {error:#}");
                    state.error = Some("skin-load-failed");
                }
            }
            if persist && state.config_path.is_some() && state.error.is_none() {
                send(
                    Request::UiPreferenceChanged {
                        preference: mirix_ipc::UiPreference::Skin,
                    },
                    cx,
                );
            }
            publish_skins(cx);
            refresh(cx);
        });
    })
    .detach();
}

pub(super) fn refresh(cx: &App) {
    let state = cx.global::<TrayState>();
    if let Some(id) = state.id
        && let Err(error) = cx.update_tray(
            id,
            options(state, cx.global::<UiLanguage>(), cx.global::<Settings>()),
        )
    {
        log::warn!("mirix-panel: update tray: {error:#}");
    }
}

fn method_label(name: &str, settings: &Settings) -> String {
    settings
        .entries
        .get(name)
        .and_then(|entry| entry.page.as_ref())
        .map(|page| page.page.title.clone())
        .unwrap_or_else(|| name.to_owned())
}

fn open_settings(cx: &mut App) {
    if cx.global::<TrayState>().settings_open {
        return;
    }
    let socket = cx.global::<Settings>().socket.clone();
    let language = cx
        .global::<UiLanguage>()
        .preference
        .clone()
        .unwrap_or_else(|| "auto".into());
    let executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|root| root.join("mirix-panel")))
        .unwrap_or_else(|| PathBuf::from("mirix-panel"));
    match std::process::Command::new(executable)
        .arg("--settings")
        .arg("--socket")
        .arg(socket)
        .arg("--language")
        .arg(language)
        .spawn()
    {
        Ok(mut child) => {
            cx.global_mut::<TrayState>().settings_open = true;
            cx.spawn(async move |cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { child.wait() })
                    .await;
                if let Err(error) = result {
                    log::warn!("mirix-panel: wait for settings: {error}");
                }
                cx.update(|cx| {
                    cx.global_mut::<TrayState>().settings_open = false;
                    refresh(cx);
                });
            })
            .detach();
        }
        Err(error) => {
            log::warn!("mirix-panel: open settings: {error}");
            cx.global_mut::<TrayState>().error = Some("settings-open-failed");
        }
    }
    refresh(cx);
}

fn configuration_menu(page: &mirix_core::EngineConfiguration, pending: bool) -> Vec<MenuItem> {
    use mirix_core::{ConfigurationEvent, ConfigurationKind, ConfigurationValue};
    page.page
        .groups
        .iter()
        .filter_map(|group| {
            let controls = group
                .controls
                .iter()
                .filter(|control| control.tray)
                .map(|control| {
                    let disabled = pending || !control.enabled;
                    let action = |event| ConfigureEngine {
                        page: page.clone(),
                        event,
                    };
                    match &control.kind {
                        ConfigurationKind::Switch { value }
                        | ConfigurationKind::Checkable { value } => MenuItem::action(
                            control.label.clone(),
                            action(ConfigurationEvent::Change {
                                id: control.id.clone(),
                                value: ConfigurationValue::Bool(!value),
                            }),
                        )
                        .checked(*value)
                        .disabled(disabled),
                        ConfigurationKind::Select { value, choices } => MenuItem::submenu(
                            Menu::new(control.label.clone()).items(choices.iter().map(|choice| {
                                MenuItem::action(
                                    choice.label.clone(),
                                    action(ConfigurationEvent::Change {
                                        id: control.id.clone(),
                                        value: ConfigurationValue::String(choice.value.clone()),
                                    }),
                                )
                                .checked(&choice.value == value)
                                .disabled(disabled)
                            })),
                        ),
                        ConfigurationKind::Checklist { values, choices } => MenuItem::submenu(
                            Menu::new(control.label.clone()).items(choices.iter().map(|choice| {
                                let checked = values.contains(&choice.value);
                                MenuItem::action(
                                    choice.label.clone(),
                                    action(ConfigurationEvent::Change {
                                        id: control.id.clone(),
                                        value: ConfigurationValue::Strings(
                                            control.kind.with_checked(&choice.value, !checked),
                                        ),
                                    }),
                                )
                                .checked(checked)
                                .disabled(disabled)
                            })),
                        ),
                        ConfigurationKind::Action => MenuItem::action(
                            control.label.clone(),
                            action(ConfigurationEvent::Invoke {
                                id: control.id.clone(),
                            }),
                        )
                        .disabled(disabled),
                        ConfigurationKind::Path { .. } | ConfigurationKind::Text { .. } => {
                            MenuItem::action(control.label.clone(), OpenSettings).disabled(disabled)
                        }
                    }
                })
                .collect::<Vec<_>>();
            (!controls.is_empty())
                .then(|| MenuItem::submenu(Menu::new(group.label.clone()).items(controls)))
        })
        .collect()
}

fn options(state: &TrayState, language: &UiLanguage, settings: &Settings) -> TrayOptions {
    let i18n = &language.localizer;
    let active = state.input_method.as_deref();
    let mut args = FluentArgs::new();
    args.set(
        "method",
        active
            .map(|name| method_label(name, settings))
            .unwrap_or_else(|| i18n.text("status-disconnected")),
    );
    let title = i18n.format("tray-title", Some(&args));
    args.set("shortcut", state.switch_shortcut.clone());
    let methods = state.available.iter().map(|name| {
        MenuItem::action(
            method_label(name, settings),
            SelectInputMethod { name: name.clone() },
        )
        .checked(active == Some(name.as_str()))
    });
    let skins = state.skins.iter().map(|skin| {
        MenuItem::action(
            skin.name.clone(),
            SelectSkin {
                path: skin.path.clone(),
            },
        )
        .checked(state.active_skin == skin.path)
    });
    let mut menu = vec![
        MenuItem::separator(),
        MenuItem::action(
            i18n.format("tray-switch-input-method", Some(&args)),
            ToggleInputMethod,
        )
        .disabled(active.is_none() || state.available.len() < 2),
        MenuItem::submenu(Menu::new(i18n.text("menu-input-methods")).items(methods)),
    ];
    if let Some(entry) = active.and_then(|name| settings.entries.get(name)) {
        if let Some(page) = &entry.page {
            menu.extend(configuration_menu(page, entry.pending.is_some()));
        }
        if entry.pending.is_some() {
            menu.push(MenuItem::label(i18n.text("settings-pending")));
        }
        if let Some(error) = &entry.error {
            menu.push(MenuItem::label(error.clone()));
        }
    }
    menu.push(
        MenuItem::action(i18n.text("settings-open"), OpenSettings).disabled(state.settings_open),
    );
    menu.push(MenuItem::submenu(
        Menu::new(i18n.text("menu-skins")).items(skins),
    ));
    menu.push(MenuItem::action(
        i18n.text("tray-refresh-skins"),
        RefreshSkins,
    ));
    let mut languages = vec![
        MenuItem::action(
            i18n.text("language-system"),
            SelectLanguage { language: None },
        )
        .checked(language.preference.is_none()),
    ];
    languages.extend(i18n.available_languages().iter().map(|entry| {
        MenuItem::action(
            entry.name.clone(),
            SelectLanguage {
                language: Some(entry.id.clone()),
            },
        )
        .checked(language.preference.is_some() && entry.id == i18n.language().to_string())
    }));
    languages.push(MenuItem::separator());
    languages.push(MenuItem::action(
        i18n.text("language-refresh"),
        RefreshLanguages,
    ));
    menu.push(MenuItem::submenu(
        Menu::new(i18n.text("menu-language")).items(languages),
    ));
    if state.loading {
        menu.push(MenuItem::label(i18n.text("skin-loading")));
    }
    if let Some(error) = &state.error {
        menu.push(MenuItem::label(i18n.text(error)));
    }
    menu.push(MenuItem::separator());
    menu.push(
        MenuItem::action(i18n.text("tray-restart"), RestartDaemon).disabled(!settings.connected),
    );
    menu.push(
        MenuItem::action(i18n.text("tray-quit"), QuitDaemon)
            .disabled(state.commands.is_some() && !settings.connected),
    );
    TrayOptions::new(icon::render(state.engine_icon.as_ref()))
        .tooltip(title)
        .menu(menu)
        .on_activate(ToggleInputMethod)
}
