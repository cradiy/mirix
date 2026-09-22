//! Personal vocabulary is managed through the engine protocol, never by editing its files.
use super::{Settings, style::*};
use crate::{app::client, i18n::UiLanguage};
use gpui::{AppContext, Context, Entity, Render, Window, div, prelude::*, px, rgb};
use mirix_core::{VocabularyEntry, VocabularyOperation as Op, VocabularyResult, VocabularyWord};
use mirix_ipc::{Request, Response};
use uic::components::input::{Input, TextInput};

pub(super) struct VocabularyPage {
    method: String,
    profile: Option<String>,
    query: Entity<TextInput>,
    word: Entity<TextInput>,
    reading: Entity<TextInput>,
    entries: Vec<VocabularyEntry>,
    offset: usize,
    total: usize,
    busy: bool,
    error: Option<String>,
}

impl VocabularyPage {
    pub(super) fn new(method: String, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            method,
            profile: None,
            query: cx.new(TextInput::new),
            word: cx.new(TextInput::new),
            reading: cx.new(TextInput::new),
            entries: vec![],
            offset: 0,
            total: 0,
            busy: false,
            error: None,
        };
        page.list(0, cx);
        page
    }

    fn list(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.request(
            Op::List {
                query: self.query.read(cx).value().to_string(),
                offset,
                limit: 10,
            },
            cx,
        );
    }

    fn request(&mut self, operation: Op, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if cx.global::<Settings>().mock.is_some() {
            super::feedback(
                true,
                "Offline preview: no vocabulary is changed.".into(),
                cx,
            );
            return;
        }
        self.busy = true;
        self.error = None;
        let socket = cx.global::<Settings>().socket.clone();
        let method = self.method.clone();
        let profile = self.profile.clone();
        let mutation = !matches!(operation, Op::List { .. });
        let add = matches!(operation, Op::Add { .. });
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    client::exchange(
                        &socket,
                        &Request::Vocabulary {
                            input_method: method,
                            profile,
                            operation,
                        },
                    )
                    .and_then(|response| match response {
                        Response::Vocabulary { result } => Ok(result),
                        _ => anyhow::bail!("expected vocabulary response"),
                    })
                    .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(VocabularyResult::Words {
                        profile,
                        entries,
                        total,
                        offset,
                    }) => {
                        this.profile = Some(profile);
                        this.entries = entries;
                        this.total = total;
                        this.offset = offset;
                        if this.entries.is_empty() && total > 0 && offset >= total {
                            this.list((total - 1) / 10 * 10, cx);
                        }
                    }
                    Ok(VocabularyResult::Updated { profile, .. }) => {
                        this.profile = Some(profile);
                        if add {
                            this.word.update(cx, |input, cx| input.clear(cx));
                            this.reading.update(cx, |input, cx| input.clear(cx));
                        }
                        let message = cx
                            .global::<UiLanguage>()
                            .localizer
                            .text("settings-vocabulary-updated");
                        super::feedback(true, message, cx);
                        this.list(this.offset, cx);
                    }
                    Err(message) => {
                        this.error = Some(message.clone());
                        if mutation {
                            super::feedback(false, message, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for VocabularyPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = cx.global::<UiLanguage>().localizer.clone();
        for (field, key) in [
            (&self.query, "settings-vocabulary-search"),
            (&self.word, "settings-vocabulary-word"),
            (&self.reading, "settings-vocabulary-reading"),
        ] {
            field.update(cx, |input, _| input.set_placeholder(text.text(key)));
        }
        let weak = cx.weak_entity();
        let refresh = weak.clone();
        let add = weak.clone();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div().text_color(rgb(MUTED)).child(
                    self.profile
                        .clone()
                        .unwrap_or_else(|| text.text("settings-vocabulary")),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Input::new(&self.query).w(px(280.)))
                    .child(action_button(
                        "vocab-search",
                        text.text("settings-vocabulary-search"),
                        self.busy,
                        move |_, cx| {
                            let _ = weak.update(cx, |this, cx| this.list(0, cx));
                        },
                    ))
                    .child(action_button(
                        "vocab-refresh",
                        text.text("settings-refresh"),
                        self.busy,
                        move |_, cx| {
                            let _ = refresh.update(cx, |this, cx| {
                                this.profile = None;
                                this.list(0, cx);
                            });
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(Input::new(&self.word).w(px(220.)))
                    .child(Input::new(&self.reading).w(px(280.)))
                    .child(action_button(
                        "vocab-add",
                        text.text("settings-vocabulary-add"),
                        self.busy || self.profile.is_none(),
                        move |_, cx| {
                            let _ = add.update(cx, |this, cx| {
                                let word = VocabularyWord {
                                    text: this.word.read(cx).value().to_string(),
                                    reading: this.reading.read(cx).value().to_string(),
                                };
                                this.request(Op::Add { word }, cx);
                            });
                        },
                    )),
            );
        for (index, entry) in self.entries.iter().enumerate() {
            let mut actions = div().flex().flex_wrap().gap_2();
            let word = &entry.word;
            for (key, operation) in [
                (
                    if entry.pinned { "unpin" } else { "pin" },
                    Op::Pin {
                        word: word.clone(),
                        enabled: !entry.pinned,
                    },
                ),
                (
                    if entry.hidden { "show" } else { "hide" },
                    Op::Hide {
                        word: word.clone(),
                        enabled: !entry.hidden,
                    },
                ),
                ("forget", Op::Forget { word: word.clone() }),
                ("remove", Op::Remove { word: word.clone() }),
            ] {
                let weak = cx.weak_entity();
                actions = actions.child(action_button(
                    format!("vocab-{index}-{key}"),
                    text.text(&format!("settings-vocabulary-{key}")),
                    self.busy,
                    move |_, cx| {
                        let _ = weak.update(cx, |this, cx| this.request(operation.clone(), cx));
                    },
                ));
            }
            body = body.child(
                div()
                    .p_3()
                    .rounded(px(12.))
                    .bg(rgb(FIELD))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().child(format!("{}   {}", word.text, word.reading)))
                    .child(div().text_color(rgb(MUTED)).child(format!(
                        "{}: {}",
                        text.text("settings-vocabulary-count"),
                        entry.count
                    )))
                    .child(actions),
            );
        }
        let previous = cx.weak_entity();
        let next = previous.clone();
        body.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(action_button(
                    "vocab-prev",
                    text.text("settings-vocabulary-previous"),
                    self.busy || self.offset == 0,
                    move |_, cx| {
                        let _ = previous
                            .update(cx, |this, cx| this.list(this.offset.saturating_sub(10), cx));
                    },
                ))
                .child(format!(
                    "{}–{} / {}",
                    if self.total == 0 { 0 } else { self.offset + 1 },
                    (self.offset + self.entries.len()).min(self.total),
                    self.total
                ))
                .child(action_button(
                    "vocab-next",
                    text.text("settings-vocabulary-next"),
                    self.busy || self.offset + self.entries.len() >= self.total,
                    move |_, cx| {
                        let _ = next.update(cx, |this, cx| this.list(this.offset + 10, cx));
                    },
                )),
        )
        .when_some(self.error.clone(), |body, error| {
            body.child(div().text_color(rgb(0xeea5ad)).child(error))
        })
    }
}
