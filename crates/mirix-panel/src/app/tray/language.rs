//! Explicit language switching and discovery; no locale filesystem watcher.

use gpui::App;
use mirix_i18n::Localizer;
use std::sync::Arc;

use super::{TrayState, UiLanguage, publish_skins, refresh};

pub(super) fn select(preference: Option<String>, persist: bool, cx: &mut App) {
    let directories = cx.global::<UiLanguage>().directories.clone();
    let state = cx.global_mut::<TrayState>();
    state.language_request += 1;
    let request = state.language_request;
    let window = state.window.clone();
    cx.spawn(async move |cx| {
        let requested = preference.clone();
        let roots = directories.clone();
        let result = cx
            .background_executor()
            .spawn(async move { Localizer::load(requested.as_deref(), &roots) })
            .await;
        cx.update(|cx| {
            if cx.global::<TrayState>().language_request != request {
                return;
            }
            match result {
                Ok(localizer) => {
                    cx.set_global(UiLanguage {
                        localizer: Arc::new(localizer),
                        preference: preference.clone(),
                        directories,
                    });
                    let state = cx.global_mut::<TrayState>();
                    state.error = None;
                    if persist
                        && let Some(path) = &state.config_path
                        && let Err(error) =
                            crate::config::save_language(path, preference.as_deref())
                    {
                        log::warn!("mirix-panel: save language preference: {error:#}");
                        state.error = Some("language-save-failed");
                    }
                    let _ =
                        window.update(cx, |panel, window, cx| panel.language_changed(window, cx));
                    publish_skins(cx);
                    super::settings::language_changed(cx);
                    if persist
                        && cx.global::<TrayState>().config_path.is_some()
                        && cx.global::<TrayState>().error.is_none()
                    {
                        super::send(
                            mirix_ipc::Request::UiPreferenceChanged {
                                preference: mirix_ipc::UiPreference::Language,
                            },
                            cx,
                        );
                    }
                }
                Err(error) => {
                    log::warn!("mirix-panel: load language: {error:#}");
                    cx.global_mut::<TrayState>().error = Some("language-load-failed");
                }
            }
            refresh(cx);
        });
    })
    .detach();
}
