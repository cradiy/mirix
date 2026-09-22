//! Framework-owned language and skin pages; engine configuration stays separate.
use super::{
    Settings,
    form::Form,
    preferences::{self, Preferences},
    style::*,
};
use crate::{
    app::{skin_catalog::SkinEntry, view::CandidatePanel},
    i18n::UiLanguage,
};
use gpui::{App, Context, Div, Entity, Render, Window, div, prelude::*, px, rgb};
use mirix_core::{
    Configuration, ConfigurationChoice, ConfigurationControl, ConfigurationEvent,
    ConfigurationGroup, ConfigurationKind, ConfigurationValue,
};
use mirix_skin::LayoutStyle;
use std::rc::Rc;

pub(super) fn language(form: &mut Form, window: &mut Window, cx: &mut App) -> Div {
    let language = cx.global::<UiLanguage>();
    let i18n = language.localizer.clone();
    let value = language
        .preference
        .as_ref()
        .map(|_| i18n.language().to_string())
        .unwrap_or_else(|| "auto".into());
    let busy = cx.global::<Preferences>().busy;
    let error = cx.global::<Preferences>().error.clone();
    let choices = std::iter::once(ConfigurationChoice {
        value: "auto".into(),
        label: i18n.text("language-system"),
    })
    .chain(
        i18n.available_languages()
            .iter()
            .map(|item| ConfigurationChoice {
                value: item.id.clone(),
                label: item.name.clone(),
            }),
    )
    .collect();
    let page = selection_document(
        "language",
        i18n.text("settings-language-label"),
        value,
        choices,
    );
    let options = form.render(
        &page,
        busy,
        &i18n,
        Rc::new(|event, _, cx| {
            if let ConfigurationEvent::Change {
                value: ConfigurationValue::String(value),
                ..
            } = event
            {
                preferences::select_language((value != "auto").then_some(value), true, cx);
            }
        }),
        window,
        cx,
    );
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_4()
        .child(options)
        .child(div().flex().child(action_button(
            "refresh-languages",
            i18n.text("language-refresh"),
            busy,
            |_, cx| {
                preferences::select_language(
                    cx.global::<UiLanguage>().preference.clone(),
                    false,
                    cx,
                );
            },
        )))
        .when_some(error, |body, error| body.child(error_message(error)))
}

fn selection_document(
    id: &str,
    label: String,
    value: String,
    choices: Vec<ConfigurationChoice>,
) -> Configuration {
    Configuration {
        title: label.clone(),
        description: String::new(),
        groups: vec![ConfigurationGroup {
            id: id.into(),
            label: String::new(),
            controls: vec![ConfigurationControl {
                id: id.into(),
                label,
                description: String::new(),
                enabled: true,
                tray: false,
                kind: ConfigurationKind::Select { value, choices },
            }],
        }],
    }
}

fn skin_id(entry: &SkinEntry) -> String {
    entry
        .path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub(super) fn page_header(key: &str, cx: &App) -> Div {
    let i18n = &cx.global::<UiLanguage>().localizer;
    header(i18n.text(key)).when(cx.global::<Settings>().mock.is_some(), |bar| {
        bar.child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child(i18n.text("settings-demo")),
        )
    })
}

pub(super) struct SkinsPage {
    selector: Form,
    selected: SkinEntry,
    previews: Option<[Entity<CandidatePanel>; 2]>,
    animation_key: String,
    has_animations: bool,
    loading: bool,
    generation: u64,
    error: Option<String>,
    scroll: PageScroll,
}

impl SkinsPage {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let state = cx.global::<Preferences>();
        let selected = state
            .skins
            .iter()
            .find(|entry| entry.path == state.active_skin)
            .cloned()
            .unwrap_or_else(|| SkinEntry {
                name: mirix_skin::BuiltinSkin::Light.name().into(),
                path: None,
            });
        let mut page = Self {
            selector: Form::default(),
            selected: selected.clone(),
            previews: None,
            animation_key: String::new(),
            has_animations: false,
            loading: false,
            generation: 0,
            error: None,
            scroll: PageScroll::default(),
        };
        page.select(selected, cx);
        page
    }

    fn select(&mut self, entry: SkinEntry, cx: &mut Context<Self>) {
        self.selected = entry;
        self.generation += 1;
        let generation = self.generation;
        self.previews = None;
        self.has_animations = false;
        self.error = None;
        self.loading = true;
        let path = self.selected.path.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let root = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let key = crate::config::animation::key(root.as_deref())?;
                    let skin = match root {
                        Some(root) => mirix_skin::load(&root),
                        None => Ok(mirix_skin::BuiltinSkin::Light.skin()),
                    }?;
                    Ok::<_, anyhow::Error>((skin, key))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((skin, key)) => {
                        this.has_animations = crate::app::motion::available(&skin);
                        this.animation_key = key;
                        let enabled = cx
                            .global::<Preferences>()
                            .animations
                            .get(&this.animation_key)
                            .copied()
                            .unwrap_or(true);
                        // Both layouts use one decode/upload per source, owned by this preview.
                        let images = gpui::RetainAllImageCache::new(cx);
                        this.previews = Some([LayoutStyle::Horizontal, LayoutStyle::Vertical].map(
                            |style| {
                                let mut preview_skin = skin.clone();
                                preview_skin.layout.style = style;
                                cx.new(|_| {
                                    let mut panel = CandidatePanel::preview(
                                        preview_skin,
                                        path.clone(),
                                        images.clone(),
                                    );
                                    panel.animations_enabled = enabled;
                                    panel
                                })
                            },
                        ));
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for SkinsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let i18n = cx.global::<UiLanguage>().localizer.clone();
        let state = cx.global::<Preferences>();
        let entries = state.skins.clone();
        let active = state.active_skin.clone();
        let busy = state.busy;
        let error = state.error.clone();
        if !entries.iter().any(|entry| entry.path == self.selected.path)
            && let Some(entry) = entries.first()
        {
            self.select(entry.clone(), cx);
        }
        let choices = entries
            .iter()
            .map(|entry| {
                let mut label = entry.name.clone();
                if entry.path == active {
                    label = format!("{label} · {}", i18n.text("settings-skin-active"));
                }
                ConfigurationChoice {
                    value: skin_id(entry),
                    label,
                }
            })
            .collect();
        let mut page = selection_document(
            "skin",
            i18n.text("settings-skin-installed"),
            skin_id(&self.selected),
            choices,
        );
        let animations_enabled = cx
            .global::<Preferences>()
            .animations
            .get(&self.animation_key)
            .copied()
            .unwrap_or(true);
        if self.has_animations {
            page.groups[0].controls.push(ConfigurationControl {
                id: "skin-animations".into(),
                label: i18n.text("settings-skin-animations"),
                description: i18n.text("settings-skin-animations-hint"),
                enabled: !busy && !self.loading,
                tray: false,
                kind: ConfigurationKind::Switch {
                    value: animations_enabled,
                },
            });
        }
        if let Some(panels) = &self.previews {
            for panel in panels {
                if panel.read(cx).animations_enabled != animations_enabled {
                    panel.update(cx, |panel, cx| {
                        panel.set_animations_enabled(animations_enabled, cx)
                    });
                }
            }
        }
        let choose = cx.weak_entity();
        let animation_key = self.animation_key.clone();
        let list = self.selector.render(
            &page,
            false,
            &i18n,
            Rc::new(move |event, _, cx| {
                if let ConfigurationEvent::Change {
                    ref id,
                    value: ConfigurationValue::Bool(enabled),
                } = event
                    && id == "skin-animations"
                {
                    preferences::set_skin_animations(animation_key.clone(), enabled, cx);
                    let _ = choose.update(cx, |_, cx| cx.notify());
                    return;
                }
                if let ConfigurationEvent::Change {
                    value: ConfigurationValue::String(value),
                    ..
                } = event
                    && let Some(entry) = entries.iter().find(|entry| skin_id(entry) == value)
                {
                    let _ = choose.update(cx, |this, cx| this.select(entry.clone(), cx));
                }
            }),
            window,
            cx,
        );
        let path = self.selected.path.clone();
        let name = self.selected.name.clone();
        let selected = path == active;
        let mut preview = card()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().text_size(px(17.)).child(name))
                    .child(action_button(
                        "apply-skin",
                        i18n.text(if selected {
                            "settings-skin-active"
                        } else {
                            "settings-skin-apply"
                        }),
                        selected || busy || self.loading || self.previews.is_none(),
                        move |_, cx| preferences::apply_skin(path.clone(), cx),
                    )),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(MUTED))
                    .child(i18n.text("settings-skin-preview-hint")),
            );
        if self.loading {
            preview = preview.child(i18n.text("skin-loading"));
        }
        if let Some(panels) = &self.previews {
            for (panel, label) in panels
                .iter()
                .zip(["settings-skin-horizontal", "settings-skin-vertical"])
            {
                preview = preview.child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(eyebrow(i18n.text(label)))
                        .child(
                            div()
                                .id(label)
                                .w_full()
                                .p_3()
                                .overflow_x_scroll()
                                .rounded(px(12.))
                                .bg(rgb(0x10131d))
                                .child(panel.clone()),
                        ),
                );
            }
        }
        if let Some(error) = &self.error {
            preview = preview.child(error_message(error.clone()));
        }
        let content = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(list)
            .child(preview)
            .when_some(error, |body, error| body.child(error_message(error)));
        let current = self.selected.clone();
        let reload = cx.weak_entity();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(page_header("settings-nav-skins", cx).child(action_button(
                "refresh-skins",
                i18n.text("tray-refresh-skins"),
                false,
                move |_, cx| {
                    preferences::refresh_skins(cx);
                    let _ = reload.update(cx, |this, cx| this.select(current.clone(), cx));
                },
            )))
            .child(self.scroll.render("settings-skins-content", content))
    }
}

fn error_message(message: String) -> Div {
    div()
        .p_3()
        .rounded(px(10.))
        .bg(rgb(0x482d35))
        .text_size(px(13.))
        .child(message)
}
