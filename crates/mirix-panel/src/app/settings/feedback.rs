//! Queued frosted feedback, following the local Apple Music demo's interaction.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use gpui::{
    App, AppContext, Context, Entity, Global, Render, SharedString, Task, Window, div, prelude::*,
    px, relative, rgb, rgba,
};
use gpui_effects::{FrostedGlass, FrostedGlassAppearance};

use super::style::{MUTED, TEXT};

const HOLD: Duration = Duration::from_secs(3);
const FADE: Duration = Duration::from_millis(220);

#[derive(Clone, PartialEq)]
struct Message {
    text: SharedString,
    error: bool,
    dismiss: SharedString,
}

struct Notice {
    id: usize,
    message: Message,
    entered: Instant,
    leaving: Option<Instant>,
}

pub(super) struct Feedback {
    current: Option<Notice>,
    pending: VecDeque<Message>,
    next_id: usize,
    dismiss_task: Option<Task<()>>,
}

impl Feedback {
    fn push(&mut self, message: Message, cx: &mut Context<Self>) {
        if self
            .current
            .as_ref()
            .is_some_and(|notice| notice.message == message)
            || self.pending.contains(&message)
        {
            return;
        }
        self.pending.push_back(message);
        if self.current.is_none() {
            self.advance(cx);
        }
    }

    fn advance(&mut self, cx: &mut Context<Self>) {
        self.dismiss_task = None;
        self.current = self.pending.pop_front().map(|message| {
            self.next_id += 1;
            Notice {
                id: self.next_id,
                message,
                entered: Instant::now(),
                leaving: None,
            }
        });
        if let Some(notice) = &self.current
            && !notice.message.error
        {
            self.schedule_dismiss(notice.id, HOLD, cx);
        }
        cx.notify();
    }

    fn schedule_dismiss(&mut self, id: usize, delay: Duration, cx: &mut Context<Self>) {
        self.dismiss_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if let Some(notice) = &mut this.current
                        && notice.id == id
                    {
                        notice.leaving = Some(Instant::now());
                        cx.notify();
                    }
                });
            }
            cx.background_executor().timer(FADE).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if this.current.as_ref().is_some_and(|notice| notice.id == id) {
                        this.advance(cx);
                    }
                });
            }
        }));
    }

    fn dismiss(&mut self, id: usize, cx: &mut Context<Self>) {
        if self
            .current
            .as_ref()
            .is_some_and(|notice| notice.id == id && notice.leaving.is_none())
        {
            self.schedule_dismiss(id, Duration::ZERO, cx);
        }
    }
}

impl Render for Feedback {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(notice) = &self.current else {
            return div().into_any_element();
        };
        let now = Instant::now();
        let entered = now.saturating_duration_since(notice.entered);
        let leaving = notice
            .leaving
            .map(|started| now.saturating_duration_since(started));
        let opacity = transition_opacity(entered, leaving);
        if entered < FADE || leaving.is_some() {
            window.request_animation_frame();
        }
        let id = notice.id;
        let error = notice.message.error;
        let text = notice.message.text.clone();
        let dismiss = notice.message.dismiss.clone();
        let content = div()
            .px(px(20.))
            .py(px(11.))
            .flex()
            .items_center()
            .gap(px(14.))
            .text_size(px(13.))
            .line_height(relative(1.4))
            .child(
                div()
                    .min_w_0()
                    .text_color(rgb(if error { 0xf0b4bd } else { TEXT }))
                    .child(text),
            )
            .when(error, |content| {
                content.child(
                    div()
                        .id("dismiss-feedback")
                        .tab_index(0)
                        .flex_none()
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .hover(|s| s.text_color(rgb(TEXT)))
                        .child(dismiss)
                        .on_click(cx.listener(move |this, _, _, cx| this.dismiss(id, cx)))
                        .on_key_down(cx.listener(
                            move |this, event: &gpui::KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.dismiss(id, cx);
                                    cx.stop_propagation();
                                }
                            },
                        )),
                )
            });
        let glass = FrostedGlass::with_appearance(FrostedGlassAppearance {
            blur_radius: px(18.),
            saturation: 1.08,
            brightness: 0.90,
            tint: rgba(0x141722b0).into(),
            edge: rgba(0xffffff26).into(),
            edge_width: px(1.),
            sheen: 0.02,
            merge_distance: px(44.),
        })
        .id(("settings-feedback", id))
        .rounded(px(20.))
        .max_w_full()
        .child(content);
        div()
            .w_full()
            .max_w(px(480.))
            .px(px(12.))
            .flex()
            .justify_center()
            .child(
                div()
                    .relative()
                    .top(px(7. * (1. - opacity)))
                    .opacity(opacity)
                    .max_w_full()
                    .child(glass),
            )
            .into_any_element()
    }
}

fn transition_opacity(entered: Duration, leaving: Option<Duration>) -> f32 {
    fn smoothstep(value: f32) -> f32 {
        let value = value.clamp(0., 1.);
        value * value * (3. - 2. * value)
    }
    smoothstep(entered.as_secs_f32() / FADE.as_secs_f32())
        * leaving.map_or(1., |elapsed| {
            1. - smoothstep(elapsed.as_secs_f32() / FADE.as_secs_f32())
        })
}

struct GlobalFeedback(Entity<Feedback>);
impl Global for GlobalFeedback {}

fn entity(cx: &mut App) -> Entity<Feedback> {
    if !cx.has_global::<GlobalFeedback>() {
        let feedback = cx.new(|_| Feedback {
            current: None,
            pending: VecDeque::new(),
            next_id: 0,
            dismiss_task: None,
        });
        cx.set_global(GlobalFeedback(feedback));
    }
    cx.global::<GlobalFeedback>().0.clone()
}

pub(super) fn layer(cx: &mut App) -> gpui::Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(24.))
        .flex()
        .justify_center()
        .child(entity(cx))
}

pub(super) fn show(success: bool, message: String, i18n: &mirix_i18n::Localizer, cx: &mut App) {
    let message = Message {
        text: if message.is_empty() {
            i18n.text(if success {
                "settings-event-success"
            } else {
                "settings-event-failure"
            })
            .into()
        } else {
            message.into()
        },
        error: !success,
        dismiss: i18n.text("settings-dismiss-feedback").into(),
    };
    entity(cx).update(cx, |feedback, cx| feedback.push(message, cx));
}
