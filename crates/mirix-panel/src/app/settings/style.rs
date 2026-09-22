//! Shared settings chrome, following GXER's compact preferences layout.
use gpui::{
    App, Div, ElementId, FontWeight, KeyDownEvent, Role, ScrollHandle, Svg, Window, div,
    prelude::*, px, rgb, svg,
};
use std::rc::Rc;
use uic::{
    assets::LucideIcons,
    components::{
        scrollbar::{Scrollbar, ScrollbarAppearance, ScrollbarState},
        selection::{CheckboxAppearance, SwitchAppearance},
    },
};

pub(super) const BG: u32 = 0x101117;
pub(super) const PANEL: u32 = 0x191b25;
pub(super) const LINE: u32 = 0x2b2d3b;
pub(super) const TEXT: u32 = 0xeeeef5;

pub(super) const MUTED: u32 = 0x9495aa;
pub(super) const ACCENT: u32 = 0xa89afa;
pub(super) const FIELD: u32 = 0x242633;

pub(super) fn icon(name: LucideIcons, size: f32) -> Svg {
    svg()
        .path(name.path())
        .size(px(size))
        .flex_shrink_0()
        .text_color(rgb(MUTED))
}

pub(super) fn card() -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_5()
        .p_5()
        .rounded(px(16.))
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(LINE))
}

pub(super) fn eyebrow(label: String) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(ACCENT))
        .child(label)
}

pub(super) fn header(title: String) -> Div {
    div()
        .h(px(64.))
        .flex_shrink_0()
        .px_6()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(rgb(LINE))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(21.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
}

pub(super) fn switch_appearance() -> SwitchAppearance {
    SwitchAppearance::default()
        .on_track(rgb(0x9e8bed).into())
        .hover_on_track(rgb(0xb3a4f5).into())
        .off_track(rgb(0x3c3d4d).into())
        .hover_off_track(rgb(0x515367).into())
        .focus_ring(rgb(ACCENT).into())
}

pub(super) fn checkbox_appearance() -> CheckboxAppearance {
    CheckboxAppearance::default()
        .unchecked(rgb(FIELD).into())
        .checked(rgb(0x9e8bed).into())
        .mark(rgb(0x14111f).into())
        .indicator_border(rgb(0x656078).into())
        .focus_ring(rgb(ACCENT).into())
        .hover_border(rgb(ACCENT).into())
}

pub(super) fn action_button(
    id: impl Into<ElementId>,
    label: String,
    disabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_click = Rc::new(on_click);
    let on_key = on_click.clone();
    div()
        .id(id)
        .focusable()
        .tab_stop(!disabled)
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(38.))
        .px_3()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(10.))
        .border_1()
        .border_color(rgb(LINE))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .bg(rgb(FIELD))
        .text_color(rgb(0xd1d0df))
        .opacity(if disabled { 0.45 } else { 1.0 })
        .when(!disabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x303344)))
                .active(|style| style.opacity(0.75))
                .on_click(move |_, window, cx| on_click(window, cx))
                .on_key_down(move |event, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        on_key(window, cx);
                        cx.stop_propagation();
                    }
                })
        })
        .focus_visible(|style| style.border_color(rgb(ACCENT)))
        .child(label)
}

pub(super) fn keyboard_navigation(event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    if event.keystroke.key == "tab" {
        if event.keystroke.modifiers.shift {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        cx.stop_propagation();
    }
}

#[derive(Default)]
pub(super) struct PageScroll {
    handle: ScrollHandle,
    bar: ScrollbarState,
}

impl PageScroll {
    pub(super) fn render(&self, id: &'static str, content: impl IntoElement) -> Div {
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                div()
                    .id(id)
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.handle)
                    .p_6()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(800.))
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(content),
                    ),
            )
            .child(
                Scrollbar::vertical(format!("{id}-scrollbar"), &self.bar, &self.handle)
                    .absolute()
                    .right_1()
                    .top_2()
                    .bottom_2()
                    .w(px(7.))
                    .appearance(
                        ScrollbarAppearance::default()
                            .thumb(rgb(0x484556).into())
                            .hover_thumb(rgb(ACCENT).into())
                            .dragging_thumb(rgb(ACCENT).into()),
                    ),
            )
    }
}
