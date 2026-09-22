//! Offline settings preview. No services, global settings state, or user preferences.
#[cfg(test)]
mod tests;

use super::{feedback::show as show_feedback, form::Form, style::*};
use anyhow::{Context as _, anyhow};
use gpui::{
    Bounds, Context, Render, TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*,
    px, rgb, size,
};
use mirix_core::{Configuration, ConfigurationEvent, ConfigurationKind, ConfigurationValue};
use std::{path::PathBuf, rc::Rc};

pub(super) struct Document {
    pub(super) path: PathBuf,
    pub(super) page: Configuration,
}

impl Document {
    pub(super) fn load(path: PathBuf) -> anyhow::Result<Self> {
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("read settings preview {}", path.display()))?;
        let page: Configuration = serde_json::from_str(&source)
            .with_context(|| format!("parse settings preview {}", path.display()))?;
        page.validate()
            .map_err(|error| anyhow!("{}: {error}", path.display()))?;
        Ok(Self { path, page })
    }

    pub(super) fn reload(&mut self) -> anyhow::Result<()> {
        *self = Self::load(self.path.clone())?;
        Ok(())
    }

    /// Returns whether the demo action replaced the entire document.
    pub(super) fn apply(&mut self, event: &ConfigurationEvent) -> Result<bool, String> {
        self.page.validate_event(event)?;
        // This is a demo-owned action handler, not a reserved protocol ID.
        if matches!(event, ConfigurationEvent::Invoke { id } if id == "load-config") {
            return self
                .reload()
                .map(|()| true)
                .map_err(|error| format!("{error:#}"));
        }
        let ConfigurationEvent::Change { id, value } = event else {
            return Ok(false);
        };
        let control = self
            .page
            .groups
            .iter_mut()
            .flat_map(|group| &mut group.controls)
            .find(|control| &control.id == id)
            .expect("validated control");
        match (&mut control.kind, value) {
            (
                ConfigurationKind::Switch { value } | ConfigurationKind::Checkable { value },
                ConfigurationValue::Bool(next),
            ) => *value = *next,
            (
                ConfigurationKind::Select { value, .. } | ConfigurationKind::Path { value, .. },
                ConfigurationValue::String(next),
            ) => *value = next.clone(),
            (
                ConfigurationKind::Text {
                    value,
                    clear_on_submit,
                    ..
                },
                ConfigurationValue::String(next),
            ) => {
                *value = if *clear_on_submit {
                    String::new()
                } else {
                    next.clone()
                };
            }
            (ConfigurationKind::Checklist { values, .. }, ConfigurationValue::Strings(next)) => {
                *values = next.clone()
            }
            _ => unreachable!("validated configuration event"),
        }
        Ok(false)
    }
}

struct DemoView {
    focus: gpui::FocusHandle,
    document: Document,
    form: Form,
    scroll: PageScroll,
    localizer: mirix_i18n::Localizer,
}

impl Render for DemoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.weak_entity();
        let body = self.form.render(
            &self.document.page,
            false,
            &self.localizer,
            Rc::new(move |event, _, cx| {
                let _ = entity.update(cx, |this, cx| {
                    println!(
                        "{}",
                        serde_json::to_string(&event).expect("serializable event")
                    );
                    match this.document.apply(&event) {
                        Ok(reloaded) => {
                            if let Some(id) = super::form::text_reset(&this.document.page, &event) {
                                this.form.clear_text(id);
                            }
                            if reloaded {
                                this.form = Form::default();
                            }
                            show_feedback(
                                true,
                                this.localizer.text("settings-preview-applied"),
                                &this.localizer,
                                cx,
                            );
                        }
                        Err(error) => {
                            show_feedback(false, error, &this.localizer, cx);
                        }
                    }
                    cx.notify();
                });
            }),
            window,
            cx,
        );
        let reload = cx.weak_entity();
        let toolbar = header(self.document.page.title.clone())
            .when(f32::from(window.viewport_size().width) >= 640., |bar| {
                bar.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(self.localizer.text("settings-demo")),
                )
            })
            .child(action_button(
                "reload-demo",
                self.localizer.text("settings-demo-reload"),
                false,
                move |_, cx| {
                    let _ = reload.update(cx, |this, cx| {
                        match this.document.reload() {
                            Ok(()) => {
                                this.form = Form::default();
                                show_feedback(
                                    true,
                                    this.localizer.text("settings-preview-applied"),
                                    &this.localizer,
                                    cx,
                                );
                            }
                            Err(error) => {
                                show_feedback(false, format!("{error:#}"), &this.localizer, cx);
                            }
                        }
                        cx.notify();
                    });
                },
            ));
        let mut content = div().w_full().flex().flex_col().gap_4();
        if !self.document.page.description.is_empty() {
            content = content.child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child(self.document.page.description.clone()),
            );
        }
        content = content.child(body);
        div()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(15.))
            .on_key_down(keyboard_navigation)
            .child(toolbar)
            .child(self.scroll.render("settings-demo-content", content))
            .child(super::feedback::layer(cx))
    }
}

pub fn run(path: PathBuf, language: Option<String>) -> anyhow::Result<()> {
    if path.is_dir() {
        return super::mock::run(path, language);
    }
    let document = Document::load(path)?;
    let localizer =
        mirix_i18n::Localizer::load(language.as_deref().filter(|value| *value != "auto"), &[])?;
    gpui_platform::application()
        .with_assets(uic::assets::LucideAssets)
        .run(move |cx| {
            uic::init(cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(920.), px(740.)),
                        cx,
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Mirix · Settings demo".into()),
                        ..Default::default()
                    }),
                    app_id: Some("mirix-settings-demo".into()),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| DemoView {
                        focus: cx.focus_handle(),
                        document,
                        form: Form::default(),
                        scroll: PageScroll::default(),
                        localizer,
                    });
                    window.focus(&view.read(cx).focus.clone(), cx);
                    view
                },
            )
            .expect("open settings preview window");
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}
