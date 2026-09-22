//! A controlled single-value field using UIC's dropdown surface.
use super::super::style::{ACCENT, FIELD, LINE, PANEL, TEXT, icon};
use super::OnEvent;
use gpui::{Context, Entity, KeyDownEvent, Render, ScrollHandle, Window, div, prelude::*, px, rgb};
use mirix_core::{ConfigurationControl, ConfigurationEvent, ConfigurationKind, ConfigurationValue};
use uic::assets::LucideIcons as I;
use uic::components::dropdown::{DropdownState, dropdown};

pub(super) struct SelectField {
    dropdown: Entity<DropdownState>,
    control: Option<ConfigurationControl>,
    disabled: bool,
    cursor: usize,
    on_event: Option<OnEvent>,
    scroll: ScrollHandle,
}

impl SelectField {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            dropdown: cx.new(|cx| DropdownState::new(window, cx)),
            control: None,
            disabled: false,
            cursor: 0,
            on_event: None,
            scroll: ScrollHandle::new(),
        }
    }

    pub(super) fn sync(
        &mut self,
        control: ConfigurationControl,
        disabled: bool,
        on_event: OnEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.control.as_ref() != Some(&control) || self.disabled != disabled {
            cx.notify();
            self.dropdown
                .update(cx, |state, cx| state.close(window, cx));
            if let ConfigurationKind::Select { value, choices } = &control.kind {
                self.cursor = choices
                    .iter()
                    .position(|choice| &choice.value == value)
                    .unwrap_or(0);
            }
        }
        self.control = Some(control);
        self.disabled = disabled;
        self.on_event = Some(on_event);
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(control) = &self.control else {
            return;
        };
        let ConfigurationKind::Select { choices, .. } = &control.kind else {
            return;
        };
        let Some(choice) = choices.get(index) else {
            return;
        };
        let event = ConfigurationEvent::Change {
            id: control.id.clone(),
            value: ConfigurationValue::String(choice.value.clone()),
        };
        self.dropdown
            .update(cx, |state, cx| state.close(window, cx));
        if let Some(on_event) = &self.on_event {
            on_event(event, window, cx);
        }
    }
}

impl Render for SelectField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(control) = &self.control else {
            return div().into_any_element();
        };
        let ConfigurationKind::Select { value, choices } = &control.kind else {
            return div().into_any_element();
        };
        let label = choices
            .iter()
            .find(|choice| &choice.value == value)
            .map(|choice| choice.label.clone())
            .unwrap_or_default();
        let trigger = div()
            .id("select-trigger")
            .focusable()
            .tab_stop(!self.disabled)
            .aria_label(control.label.clone())
            .w_full()
            .min_w_0()
            .px_3()
            .h(px(40.))
            .text_size(px(13.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(FIELD))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .focus_visible(|style| style.border_color(rgb(ACCENT)))
            .child(div().min_w_0().truncate().child(label))
            .child(icon(I::ChevronDown, 16.));
        if self.disabled {
            return div()
                .w_full()
                .opacity(0.45)
                .child(trigger)
                .into_any_element();
        }
        let count = choices.len();
        let mut menu = div()
            .id("select-options")
            .w_full()
            .max_h(px(256.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap_1();
        for (index, choice) in choices.iter().enumerate() {
            let selected = &choice.value == value;
            menu = menu.child(
                div()
                    .id(index)
                    .px_3()
                    .py_2()
                    .rounded(px(7.))
                    .text_size(px(13.))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .flex()
                    .gap_2()
                    .bg(rgb(if self.cursor == index {
                        0x343044
                    } else {
                        PANEL
                    }))
                    .hover(|style| style.bg(rgb(0x353444)))
                    .child(
                        icon(I::Check, 15.)
                            .text_color(rgb(ACCENT))
                            .when(!selected, |icon| icon.opacity(0.)),
                    )
                    .child(choice.label.clone())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.choose(index, window, cx)),
                    ),
            );
        }
        div()
            .w_full()
            .text_color(rgb(TEXT))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                let open = this.dropdown.read(cx).is_open();
                match event.keystroke.key.as_str() {
                    "enter" | "space" if !open => {
                        this.dropdown.update(cx, |state, cx| state.open(window, cx));
                        this.scroll.scroll_to_item(this.cursor);
                    }
                    "enter" if open => this.choose(this.cursor, window, cx),
                    "up" | "down" if count > 0 => {
                        if !open {
                            this.dropdown.update(cx, |state, cx| state.open(window, cx));
                        } else if event.keystroke.key == "down" {
                            this.cursor = (this.cursor + 1).min(count - 1);
                        } else {
                            this.cursor = this.cursor.saturating_sub(1);
                        }
                        this.scroll.scroll_to_item(this.cursor);
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                dropdown(&self.dropdown)
                    .w(px(260.))
                    .max_h(px(272.))
                    .bg(rgb(PANEL))
                    .border_color(rgb(LINE))
                    .trigger(trigger)
                    .menu(menu),
            )
            .into_any_element()
    }
}
