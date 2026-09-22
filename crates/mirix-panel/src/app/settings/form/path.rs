//! Controlled path selection. The owning engine, not the form, loads the file.
#[cfg(test)]
mod tests;

use super::super::style::{FIELD, LINE, MUTED, action_button, icon};
use super::OnEvent;
use gpui::{Context, PathPromptOptions, Render, Window, div, prelude::*, px, rgb};
use mirix_core::{
    ConfigurationControl, ConfigurationEvent, ConfigurationKind, ConfigurationPathTarget,
    ConfigurationValue,
};
use std::path::PathBuf;
use uic::assets::LucideIcons as I;

#[derive(Default, PartialEq, Eq)]
struct Labels {
    browse: String,
    empty: String,
    clear: String,
    pending: String,
    error: String,
}

#[derive(Default)]
pub(super) struct PathField {
    control: Option<ConfigurationControl>,
    disabled: bool,
    on_event: Option<OnEvent>,
    labels: Labels,
    generation: u64,
    pending: bool,
    error: Option<String>,
}

impl PathField {
    pub(super) fn sync(
        &mut self,
        control: ConfigurationControl,
        disabled: bool,
        on_event: OnEvent,
        localizer: &mirix_i18n::Localizer,
        cx: &mut Context<Self>,
    ) {
        let directory = matches!(
            control.kind,
            ConfigurationKind::Path {
                target: ConfigurationPathTarget::Directory,
                ..
            }
        );
        let labels = Labels {
            browse: localizer.text(if directory {
                "settings-path-directory"
            } else {
                "settings-path-file"
            }),
            empty: localizer.text("settings-path-empty"),
            clear: localizer.text("settings-path-clear"),
            pending: localizer.text("settings-path-pending"),
            error: localizer.text("settings-path-error"),
        };
        if self.control.as_ref() != Some(&control) || self.disabled != disabled {
            self.generation += 1;
            self.pending = false;
            self.error = None;
            cx.notify();
        }
        if self.labels != labels {
            self.labels = labels;
            cx.notify();
        }
        self.control = Some(control);
        self.disabled = disabled;
        self.on_event = Some(on_event);
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.pending {
            return;
        }
        let Some(control) = &self.control else { return };
        let ConfigurationKind::Path { target, .. } = control.kind else {
            return;
        };
        let Some(on_event) = self.on_event.clone() else {
            return;
        };
        let id = control.id.clone();
        let generation = self.generation;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: target == ConfigurationPathTarget::File,
            directories: target == ConfigurationPathTarget::Directory,
            multiple: false,
            prompt: Some(self.labels.browse.clone().into()),
        });
        self.pending = true;
        self.error = None;
        cx.notify();
        // Keep the callback's original page/revision. Never submit a dialog result
        // against a newer configuration snapshot or a different selected engine.
        cx.spawn_in(window, async move |this, cx| {
            let result = match receiver.await {
                Ok(result) => result.and_then(selected_path),
                Err(error) => Err(error.into()),
            };
            let _ = cx.update(|window, cx| {
                let value = this
                    .update(cx, |this, cx| {
                        if this.generation != generation || this.disabled {
                            return None;
                        }
                        this.pending = false;
                        cx.notify();
                        match result {
                            Ok(value) => value,
                            Err(error) => {
                                this.error = Some(format!("{}: {error:#}", this.labels.error));
                                None
                            }
                        }
                    })
                    .ok()
                    .flatten();
                if let Some(value) = value {
                    on_event(
                        ConfigurationEvent::Change {
                            id,
                            value: ConfigurationValue::String(value),
                        },
                        window,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.pending {
            return;
        }
        if let (Some(control), Some(on_event)) = (&self.control, &self.on_event) {
            self.error = None;
            on_event(
                ConfigurationEvent::Change {
                    id: control.id.clone(),
                    value: ConfigurationValue::String(String::new()),
                },
                window,
                cx,
            );
            cx.notify();
        }
    }
}

impl Render for PathField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ConfigurationControl {
            kind: ConfigurationKind::Path { value, target },
            ..
        }) = &self.control
        else {
            return div().into_any_element();
        };
        let choose = cx.weak_entity();
        let clear = cx.weak_entity();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(40.))
                            .px_3()
                            .rounded(px(10.))
                            .bg(rgb(FIELD))
                            .border_1()
                            .border_color(rgb(LINE))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(13.))
                            .when(self.disabled, |field| field.opacity(0.45))
                            .child(icon(
                                if *target == ConfigurationPathTarget::Directory {
                                    I::Folder
                                } else {
                                    I::File
                                },
                                16.,
                            ))
                            .child(
                                div()
                                    .id("path-value")
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_x_scroll()
                                    .whitespace_nowrap()
                                    .when(value.is_empty(), |text| text.text_color(rgb(MUTED)))
                                    .child(if value.is_empty() {
                                        self.labels.empty.clone()
                                    } else {
                                        value.clone()
                                    }),
                            ),
                    )
                    .child(action_button(
                        "browse",
                        if self.pending {
                            self.labels.pending.clone()
                        } else {
                            self.labels.browse.clone()
                        },
                        self.disabled || self.pending,
                        move |window, cx| {
                            let _ = choose.update(cx, |this, cx| this.choose(window, cx));
                        },
                    ))
                    .when(!value.is_empty(), |row| {
                        row.child(action_button(
                            "clear",
                            self.labels.clear.clone(),
                            self.disabled || self.pending,
                            move |window, cx| {
                                let _ = clear.update(cx, |this, cx| this.clear(window, cx));
                            },
                        ))
                    }),
            )
            .when_some(self.error.clone(), |field, error| {
                field.child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(0xe6a4b0))
                        .child(error),
                )
            })
            .into_any_element()
    }
}

fn selected_path(paths: Option<Vec<PathBuf>>) -> anyhow::Result<Option<String>> {
    let Some(paths) = paths else { return Ok(None) };
    let mut paths = paths.into_iter();
    let Some(path) = paths.next() else {
        return Ok(None);
    };
    anyhow::ensure!(paths.next().is_none(), "Select a single path");
    // Do not silently corrupt a non-UTF-8 filename when sending the JSON value.
    path.into_os_string()
        .into_string()
        .map(Some)
        .map_err(|_| anyhow::anyhow!("The selected path is not valid UTF-8"))
}
