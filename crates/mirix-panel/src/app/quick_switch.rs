//! Compact modal list above the unchanged candidate layout.

use std::ops::Range;

use gpui::{Context, FontWeight, Pixels, Size, Window, div, prelude::*, px, rgba, size};
use mirix_core::{QuickSwitchKind, QuickSwitchSnapshot};
use mirix_ipc::Request;

use super::{layout::measure, view::CandidatePanel};

pub(super) const GAP: f32 = 6.0;
const ROW: f32 = 32.0;
const HEADER: f32 = 34.0;
const FOOTER: f32 = 28.0;

#[derive(Default)]
pub(super) struct PickerLayout {
    pub size: Size<Pixels>,
    pub rows: Range<usize>,
}

pub(super) fn calculate(
    state: Option<&QuickSwitchSnapshot>,
    available: Size<Pixels>,
    body: Size<Pixels>,
    window: &Window,
    i18n: &mirix_i18n::Localizer,
) -> PickerLayout {
    let Some(state) = state else {
        return PickerLayout::default();
    };
    let width = state
        .entries
        .iter()
        .map(|entry| measure(window, &entry.label, 15.0, FontWeight::NORMAL) + 64.0)
        .fold(
            measure(window, &i18n.text("picker-help"), 11.0, FontWeight::NORMAL).max(216.0) + 24.0,
            f32::max,
        )
        .min(420.0)
        .min(f32::from(available.width));
    // The candidate layout uses at most 60% of the output height. Use the
    // remaining space for this list without re-paginating the composition.
    let height_budget = f32::from(available.height) / 0.6 - f32::from(body.height) - 32.0;
    let capacity = ((height_budget - HEADER - FOOTER) / ROW)
        .floor()
        .clamp(1.0, 6.0) as usize;
    let rows = visible_rows(state.selected, state.entries.len(), capacity);
    PickerLayout {
        size: size(
            px(width),
            px(HEADER + FOOTER + rows.len().max(1) as f32 * ROW),
        ),
        rows,
    }
}

fn visible_rows(selected: usize, count: usize, capacity: usize) -> Range<usize> {
    let length = capacity.min(count);
    let start = selected
        .saturating_sub(length / 2)
        .min(count.saturating_sub(length));
    start..start + length
}

impl CandidatePanel {
    pub(super) fn render_quick_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.quick_switch.as_ref().unwrap();
        let skin = &self.skin;
        let i18n = &cx.global::<crate::i18n::UiLanguage>().localizer;
        let mut args = mirix_i18n::FluentArgs::new();
        args.set(
            "current",
            if state.entries.is_empty() {
                0
            } else {
                state.selected + 1
            },
        );
        args.set("total", state.entries.len());
        let position = i18n.format("picker-position", Some(&args));
        let title = match &state.kind {
            QuickSwitchKind::Engine { .. } => state.label.clone(),
            QuickSwitchKind::Skin => i18n.text("menu-skins"),
        };
        let empty = i18n.text("picker-empty");
        let help = i18n.text("picker-help");
        let mut menu = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(self.picker_layout.size.width)
            .h(self.picker_layout.size.height)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(skin.window.radius.min(14.0)))
            .border_1()
            .border_color(rgba(skin.window.border.0))
            .bg(rgba(skin.window.background.0 | 0x000000ff))
            .text_color(rgba(skin.candidate.text.0))
            .text_size(px(15.0))
            .child(
                div()
                    .h(px(HEADER))
                    .flex_shrink_0()
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.0))
                    .text_color(rgba(skin.candidate.comment.0))
                    .child(title)
                    .child(position),
            );
        if state.entries.is_empty() {
            menu = menu.child(div().h(px(ROW)).px(px(12.0)).child(empty));
        }
        for index in self.picker_layout.rows.clone() {
            let entry = &state.entries[index];
            let highlighted = index == state.selected;
            let active = state.active.as_ref() == Some(&entry.id);
            let session = state.session;
            let commands = self.services.commands();
            menu = menu.child(
                div()
                    .id(("quick-switch-item", index))
                    .h(px(ROW))
                    .mx(px(6.0))
                    .px(px(8.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .when(highlighted, |item| {
                        item.bg(rgba(skin.candidate.selected_background.0))
                            .text_color(rgba(skin.candidate.selected_text.0))
                    })
                    .when(!highlighted, |item| {
                        item.hover(|style| style.bg(rgba(skin.candidate.hover_background.0)))
                    })
                    .on_click(cx.listener(move |_, _, _, _| {
                        if let Some(commands) = &commands {
                            let _ = commands.send(Request::ChooseQuickSwitch { session, index });
                        }
                    }))
                    .child(
                        div()
                            .size(px(5.0))
                            .flex_shrink_0()
                            .rounded_full()
                            .when(active, |dot| dot.bg(rgba(skin.candidate.text.0))),
                    )
                    .child(div().min_w_0().truncate().child(entry.label.clone())),
            );
        }
        menu.child(
            div()
                .h(px(FOOTER))
                .flex_shrink_0()
                .px(px(12.0))
                .flex()
                .items_center()
                .text_size(px(11.0))
                .text_color(rgba(skin.candidate.comment.0))
                .child(div().truncate().child(state.error.clone().unwrap_or(help))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_slice_follows_selection_and_handles_small_or_empty_lists() {
        assert_eq!(visible_rows(0, 0, 6), 0..0);
        assert_eq!(visible_rows(1, 2, 6), 0..2);
        assert_eq!(visible_rows(0, 20, 6), 0..6);
        assert_eq!(visible_rows(10, 20, 6), 7..13);
        assert_eq!(visible_rows(19, 20, 6), 14..20);
    }
}
