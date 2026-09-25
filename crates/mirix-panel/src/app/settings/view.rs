use super::{Section, Settings, form::Form, pages, query, sidebar, style::*, submit};
use crate::i18n::UiLanguage;
use gpui::{
    App, AppContext, Context, Entity, Subscription, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use std::rc::Rc;
use uic::components::input::TextInput;

pub(super) struct SettingsView {
    focus: gpui::FocusHandle,
    form: Form,
    owner: Option<(String, String)>,
    scroll: PageScroll,
    search: Entity<TextInput>,
    _search_subscription: Subscription,
    section: Section,
    language_scroll: PageScroll,
    language_form: Form,
    shortcuts_form: Form,
    skins: Option<Entity<pages::SkinsPage>>,
    vocabulary: Option<Entity<super::vocabulary::VocabularyPage>>,
}

pub(super) fn open(cx: &mut App) {
    let title = cx.global::<UiLanguage>().localizer.text("settings-title");
    match cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::centered(
                None,
                size(px(1280.), px(800.)),
                cx,
            ))),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some(title.into()),
                ..Default::default()
            }),
            app_id: Some(
                if cx.global::<Settings>().mock.is_some() {
                    "mirix-settings-demo"
                } else {
                    "mirix"
                }
                .into(),
            ),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| {
                let search = cx.new(TextInput::new);
                let subscription = cx.subscribe(&search, |_, _, _, cx| cx.notify());
                SettingsView {
                    focus: cx.focus_handle(),
                    form: Form::default(),
                    owner: None,
                    scroll: PageScroll::default(),
                    search,
                    _search_subscription: subscription,
                    section: Section::Methods,
                    language_scroll: PageScroll::default(),
                    language_form: Form::default(),
                    shortcuts_form: Form::default(),
                    skins: None,
                    vocabulary: None,
                }
            });
            window.focus(&view.read(cx).focus.clone(), cx);
            view
        },
    ) {
        Ok(window) => {
            cx.global_mut::<Settings>().window = Some(window);
            cx.activate(true);
        }
        Err(error) => {
            log::warn!("mirix settings: {error:#}");
            cx.quit();
        }
    }
}

pub(super) fn select_method(name: &str, cx: &mut App) {
    cx.global_mut::<Settings>().selected = Some(name.to_owned());
    query(name, false, cx);
    super::repaint(cx);
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let i18n = cx.global::<UiLanguage>().localizer.clone();
        let compact = f32::from(window.viewport_size().width) < 1200.;
        let view = cx.weak_entity();
        let navigation = sidebar::navigation(
            compact,
            &i18n,
            self.section,
            Rc::new(move |section, cx| {
                let _ = view.update(cx, |this, cx| {
                    if this.section != section {
                        this.section = section;
                        this.form = Form::default();
                        if section != Section::Skins {
                            this.skins = None;
                        }
                        cx.notify();
                    }
                });
            }),
        );
        if self.section != Section::Methods {
            let page = match self.section {
                Section::Shortcuts => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(pages::page_header("settings-nav-shortcuts", cx))
                    .child(self.language_scroll.render(
                        "settings-shortcuts-content",
                        super::shortcuts::render(&mut self.shortcuts_form, window, cx),
                    ))
                    .into_any_element(),
                Section::Language => div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(pages::page_header("settings-nav-language", cx))
                    .child(self.language_scroll.render(
                        "settings-language-content",
                        pages::language(&mut self.language_form, window, cx),
                    ))
                    .into_any_element(),
                Section::Skins => self
                    .skins
                    .get_or_insert_with(|| cx.new(pages::SkinsPage::new))
                    .clone()
                    .into_any_element(),
                Section::Methods => unreachable!(),
            };
            return div()
                .relative()
                .size_full()
                .track_focus(&self.focus)
                .flex()
                .bg(rgb(BG))
                .text_color(rgb(TEXT))
                .text_size(px(15.))
                .on_key_down(keyboard_navigation)
                .child(navigation)
                .child(div().flex_1().min_w_0().h_full().child(page))
                .child(super::feedback::layer(cx))
                .into_any_element();
        }
        let placeholder = cx.global::<UiLanguage>().localizer.text("settings-search");
        self.search
            .update(cx, |search, _| search.set_placeholder(placeholder));
        let state = cx.global::<Settings>();
        let i18n = cx.global::<UiLanguage>().localizer.clone();
        let selected = state.selected.clone();
        if self.owner.as_ref().map(|(name, _)| name) != selected.as_ref() {
            self.form = Form::default();
            self.owner = None;
            self.vocabulary = None;
        }
        let entry = selected.as_ref().and_then(|name| state.entries.get(name));
        let page = entry.and_then(|entry| entry.page.clone());
        let busy = entry.is_some_and(|entry| entry.pending.is_some());
        let loaded = state.catalog.as_ref().is_some_and(|catalog| {
            catalog
                .entries
                .iter()
                .any(|method| Some(&method.name) == selected.as_ref() && method.loaded)
        });
        let text_reset = entry.and_then(|entry| entry.text_reset.clone());
        let title = page
            .as_ref()
            .map(|page| page.page.title.clone())
            .or_else(|| selected.clone())
            .unwrap_or_else(|| i18n.text("settings-title"));
        let vocabulary_view = cx.weak_entity();
        let toolbar = header(title)
            .when_some(selected.clone(), |bar, name| {
                bar.child(action_button(
                    "open-vocabulary",
                    i18n.text(if self.vocabulary.is_some() {
                        "settings-back-configuration"
                    } else {
                        "settings-vocabulary"
                    }),
                    busy || !loaded || !state.connected,
                    move |_, cx| {
                        let _ = vocabulary_view.update(cx, |this, cx| {
                            this.vocabulary = if this.vocabulary.is_some() {
                                None
                            } else {
                                Some(cx.new(|cx| {
                                    super::vocabulary::VocabularyPage::new(name.clone(), cx)
                                }))
                            };
                            cx.notify();
                        });
                    },
                ))
            })
            .when_some(
                selected.clone().filter(|_| state.mock.is_none()),
                |bar, name| {
                    bar.child(action_button(
                        "restart-input-method",
                        i18n.text("settings-restart-method"),
                        state.catalog_pending.is_some() || !state.connected || !loaded,
                        move |_, cx| super::catalog::restart(name.clone(), cx),
                    ))
                },
            )
            .when(state.mock.is_some(), |bar| {
                bar.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(i18n.text("settings-demo")),
                )
            })
            .when_some(selected.clone(), |header, name| {
                header.child(action_button(
                    "refresh-configuration",
                    i18n.text("settings-refresh"),
                    busy,
                    move |_, cx| query(&name, true, cx),
                ))
            });

        let methods = sidebar::methods(state, &self.search, compact, &i18n, cx);

        let mut content = div().w_full().flex().flex_col().gap_4().text_size(px(13.));
        if let Some(error) = &state.catalog_error {
            content = content.child(error_message(error.clone()));
        }
        if state.catalog_pending.is_some() {
            content = content.child(
                div()
                    .text_color(rgb(ACCENT))
                    .child(i18n.text("settings-inventory-pending")),
            );
        }
        if state
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.selection_locked)
        {
            content = content.child(
                div()
                    .text_color(rgb(MUTED))
                    .child(i18n.text("settings-cli-selection")),
            );
        }
        if !state.connected {
            content = content.child(i18n.text("status-disconnected"));
        } else if let Some(name) = &selected {
            if let Some(method) = state
                .catalog
                .as_ref()
                .and_then(|catalog| catalog.entries.iter().find(|entry| &entry.name == name))
            {
                if let Some(error) = &method.error {
                    content = content.child(error_message(error.clone()));
                }
                if method.directory.is_none() {
                    content = content.child(i18n.text("settings-missing"));
                }
                if !method.loaded {
                    content = content.child(i18n.text("settings-enable-to-configure"));
                }
            }
            if busy {
                content = content.child(
                    div()
                        .text_color(rgb(ACCENT))
                        .child(i18n.text("settings-pending")),
                );
            }
            if let Some(page) = page {
                if !page.page.description.is_empty() {
                    content = content.child(
                        div()
                            .text_color(rgb(MUTED))
                            .child(page.page.description.clone()),
                    );
                }
                if page.page.groups.is_empty() && page.page.description.is_empty() {
                    content = content.child(i18n.text("settings-empty"));
                }
                let owner = (
                    page.status.input_method.clone(),
                    page.status.session.clone(),
                );
                if self.owner.as_ref() != Some(&owner) {
                    self.form = Form::default();
                    self.scroll = PageScroll::default();
                    self.owner = Some(owner);
                }
                let document = page.page.clone();
                self.form.acknowledge(text_reset);
                content = content.child(self.form.render(
                    &document,
                    busy,
                    &i18n,
                    Rc::new(move |event, _, cx| submit(&page, event, cx)),
                    window,
                    cx,
                ));
            }
        } else {
            content = content.child(i18n.text("settings-no-methods"));
        }
        div()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(15.))
            .on_key_down(keyboard_navigation)
            .child(navigation)
            .child(methods)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(toolbar)
                    .child(self.scroll.render(
                        "configuration-content",
                        match &self.vocabulary {
                            Some(page) => page.clone().into_any_element(),
                            None => content.into_any_element(),
                        },
                    )),
            )
            .child(super::feedback::layer(cx))
            .into_any_element()
    }
}

fn error_message(error: String) -> impl IntoElement {
    div().p_3().rounded(px(10.)).bg(rgb(0x482d35)).child(error)
}
