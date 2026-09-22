//! Explicit-submit text input; drafts stay local until Enter or the button.
use super::super::style::{ACCENT, FIELD, LINE, MUTED, TEXT, action_button};
use super::OnEvent;
use gpui::{Context, Entity, Focusable, Render, Subscription, Window, div, prelude::*, px, rgb};
use mirix_core::{ConfigurationControl, ConfigurationEvent, ConfigurationKind, ConfigurationValue};
use uic::components::input::{Input, InputEvent, TextInput};

pub(super) struct TextField {
    input: Entity<TextInput>,
    control: Option<ConfigurationControl>,
    disabled: bool,
    on_event: Option<OnEvent>,
    _subscription: Subscription,
}

impl TextField {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(TextInput::new);
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if let InputEvent::Submit(value) = event {
                this.submit(value.to_string(), window, cx);
            }
        });
        Self {
            input,
            control: None,
            disabled: false,
            on_event: None,
            _subscription: subscription,
        }
    }

    pub(super) fn sync(
        &mut self,
        control: ConfigurationControl,
        disabled: bool,
        clear: bool,
        on_event: OnEvent,
        cx: &mut Context<Self>,
    ) {
        let ConfigurationKind::Text {
            value, placeholder, ..
        } = &control.kind
        else {
            return;
        };
        let previous = self.control.as_ref().and_then(|control| {
            if let ConfigurationKind::Text { value, .. } = &control.kind {
                Some(value)
            } else {
                None
            }
        });
        let first_sync = self.control.is_none();
        let disabled_changed = self.disabled != disabled;
        let control_changed = self.control.as_ref() != Some(&control);
        if clear || control_changed || disabled_changed {
            self.input.update(cx, |input, cx| {
                if first_sync || disabled_changed {
                    input.focus_handle(cx).tab_stop(!disabled);
                }
                // This setter cancels preedit and mouse selection even when the
                // supplied value is unchanged. Never call it on ordinary renders.
                if input.is_disabled() != disabled {
                    input.set_disabled(disabled, cx);
                }
                if clear {
                    input.clear(cx);
                } else if previous != Some(value) {
                    input.set_value(value.clone(), cx);
                }
                input.set_placeholder(placeholder.clone());
            });
            cx.notify();
        }
        self.control = Some(control);
        self.disabled = disabled;
        self.on_event = Some(on_event);
    }

    fn submit(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let (Some(control), Some(on_event)) = (&self.control, &self.on_event) {
            on_event(
                ConfigurationEvent::Change {
                    id: control.id.clone(),
                    value: ConfigurationValue::String(value),
                },
                window,
                cx,
            );
        }
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ConfigurationControl {
            kind: ConfigurationKind::Text { submit_label, .. },
            ..
        }) = &self.control
        else {
            return div();
        };
        let field = cx.weak_entity();
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Input::new(&self.input)
                    .flex_1()
                    .min_w_0()
                    .h(px(40.))
                    .rounded(px(10.))
                    .bg(rgb(FIELD))
                    .text_color(rgb(TEXT))
                    .border_color(rgb(LINE))
                    .placeholder(rgb(MUTED).into())
                    .focus_border(rgb(ACCENT).into())
                    .caret(rgb(ACCENT).into()),
            )
            .child(action_button(
                "submit",
                submit_label.clone(),
                self.disabled,
                move |window, cx| {
                    let _ = field.update(cx, |this, cx| {
                        let value = this.input.read(cx).value().to_string();
                        this.submit(value, window, cx);
                    });
                },
            ))
    }
}
