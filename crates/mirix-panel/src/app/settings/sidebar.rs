//! Framework navigation and installed input methods, separate from engine forms.
use super::{Section, Settings, catalog, style::*, view::select_method};
use gpui::{App, Div, Entity, FontWeight, div, prelude::*, px, rgb, rgba};
use mirix_i18n::Localizer;
use std::rc::Rc;
use uic::{
    assets::LucideIcons as I,
    components::{
        input::{Input, TextInput},
        selection::Switch,
    },
};

type OnSelect = Rc<dyn Fn(Section, &mut App)>;

pub(super) fn navigation(
    compact: bool,
    i18n: &Localizer,
    current: Section,
    on_select: OnSelect,
) -> Div {
    let mut entries = div().flex().flex_col().gap_2();
    for (id, symbol, label, section) in [
        (
            "methods",
            I::Keyboard,
            "settings-nav-methods",
            Some(Section::Methods),
        ),
        ("appearance", I::Paintbrush, "settings-nav-appearance", None),
        (
            "language",
            I::Globe,
            "settings-nav-language",
            Some(Section::Language),
        ),
        (
            "skins",
            I::Shirt,
            "settings-nav-skins",
            Some(Section::Skins),
        ),
        (
            "shortcuts",
            I::Zap,
            "settings-nav-shortcuts",
            Some(Section::Shortcuts),
        ),
        ("about", I::Info, "settings-nav-about", None),
    ] {
        let selected = section == Some(current);
        let click = on_select.clone();
        let key = on_select.clone();
        entries = entries.child(
            div()
                .id(id)
                .h(px(52.))
                .px_3()
                .flex()
                .items_center()
                .gap_3()
                .rounded(px(10.))
                .border_1()
                .border_color(if selected {
                    rgba(0xa89afa66)
                } else {
                    rgba(0x00000000)
                })
                .text_color(rgb(if selected { TEXT } else { MUTED }))
                .when(selected, |row| row.bg(rgba(0x8b70ff30)))
                .when(section.is_none(), |row| row.opacity(0.5))
                .when_some(section, |row, section| {
                    row.focusable()
                        .tab_stop(true)
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(0xa89afa18)))
                        .focus_visible(|style| style.border_color(rgb(ACCENT)))
                        .on_click(move |_, _, cx| click(section, cx))
                        .on_key_down(move |event, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                key(section, cx);
                                cx.stop_propagation();
                            }
                        })
                })
                .when(compact, |row| row.justify_center())
                .aria_label(i18n.text(label))
                .child(icon(symbol, 21.).text_color(rgb(if selected { ACCENT } else { MUTED })))
                .when(!compact, |row| row.child(i18n.text(label))),
        );
    }
    div()
        .w(px(if compact { 76. } else { 200. }))
        .h_full()
        .flex_shrink_0()
        .p_3()
        .flex()
        .flex_col()
        .gap_6()
        .bg(rgb(0x171a28))
        .border_r_1()
        .border_color(rgb(LINE))
        .child(
            div()
                .h(px(64.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_3()
                .px_1()
                .when(compact, |row| row.justify_center())
                .child(
                    div()
                        .size(px(38.))
                        .flex_shrink_0()
                        .rounded(px(12.))
                        .bg(rgb(0x9e8bed))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(I::Keyboard, 22.).text_color(rgb(0x221a35))),
                )
                .when(!compact, |row| {
                    row.child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Mirix"),
                    )
                }),
        )
        .child(
            div()
                .id("settings-navigation")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(entries),
        )
        .when(!compact, |rail| {
            rail.child(
                div()
                    .px_2()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(i18n.text("settings-nav-unavailable")),
            )
        })
}

pub(super) fn methods(
    state: &Settings,
    search: &Entity<TextInput>,
    compact: bool,
    i18n: &Localizer,
    cx: &App,
) -> Div {
    let filter = search.read(cx).value().trim().to_lowercase();
    let mut rows = div()
        .id("installed-method-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_2();
    let mut count = 0;
    for method in state.catalog.iter().flat_map(|catalog| &catalog.entries) {
        let name = method.name.clone();
        let label = state
            .entries
            .get(&name)
            .and_then(|entry| entry.page.as_ref())
            .map(|page| page.page.title.clone())
            .unwrap_or_else(|| name.clone());
        if !label.to_lowercase().contains(&filter) && !name.to_lowercase().contains(&filter) {
            continue;
        }
        count += 1;
        let toggle_name = name.clone();
        let key_name = name.clone();
        let is_selected = state.selected.as_ref() == Some(&name);
        let disabled = !state.connected
            || state.catalog_pending.is_some()
            || state
                .catalog
                .as_ref()
                .is_some_and(|catalog| catalog.selection_locked)
            || (method.directory.is_none() && !method.enabled);
        let monogram = label
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_default();
        rows = rows.child(
            div()
                .id(name.clone())
                .min_h(px(72.))
                .px_3()
                .py_3()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_3()
                .rounded(px(12.))
                .border_1()
                .border_color(if is_selected {
                    rgba(0xa89afaaa)
                } else {
                    rgba(0x00000000)
                })
                .text_size(px(14.))
                .focusable()
                .tab_stop(true)
                .cursor_pointer()
                .when(is_selected, |row| row.bg(rgba(0x8b70ff24)))
                .hover(|style| style.bg(rgba(0xa89afa18)))
                .focus_visible(|style| style.border_color(rgb(ACCENT)))
                .child(
                    div()
                        .size(px(36.))
                        .flex_shrink_0()
                        .rounded(px(10.))
                        .bg(rgb(if is_selected { 0x544695 } else { 0x272c3d }))
                        .text_color(rgb(if is_selected { TEXT } else { 0xbfc8df }))
                        .text_size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(monogram),
                )
                .child(div().flex_1().min_w_0().child(label.clone()))
                .child(
                    Switch::new(format!("enable-{name}"), method.enabled)
                        .label(label)
                        .disabled(disabled)
                        .appearance(switch_appearance())
                        .on_change(move |enabled, _, cx| {
                            cx.stop_propagation();
                            catalog::set_enabled(toggle_name.clone(), enabled, cx);
                        }),
                )
                .child(icon(I::ChevronRight, 14.))
                .on_click(move |_, _, cx| select_method(&name, cx))
                .on_key_down(move |event, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        select_method(&key_name, cx);
                        cx.stop_propagation();
                    }
                }),
        );
    }
    if count == 0 {
        rows = rows.child(
            div()
                .p_3()
                .text_size(px(13.))
                .text_color(rgb(MUTED))
                .child(i18n.text(if filter.is_empty() {
                    "settings-no-methods"
                } else {
                    "settings-search-empty"
                })),
        );
    }
    div()
        .w(px(if compact { 272. } else { 320. }))
        .h_full()
        .flex_shrink_0()
        .p_4()
        .flex()
        .flex_col()
        .gap_4()
        .bg(rgb(0x141722))
        .border_r_1()
        .border_color(rgb(LINE))
        .child(
            div()
                .h(px(44.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(i18n.text("settings-methods")),
                ),
        )
        .child(
            Input::new(search)
                .h(px(40.))
                .text_size(px(13.))
                .bg(rgb(0x1b1e2d))
                .text_color(rgb(TEXT))
                .border_color(rgb(LINE))
                .placeholder(rgb(MUTED).into())
                .focus_border(rgb(ACCENT).into())
                .prefix(icon(I::Search, 17.)),
        )
        .child(rows)
        .child(
            div()
                .pt_3()
                .border_t_1()
                .border_color(rgb(LINE))
                .child(action_button(
                    "refresh-input-methods",
                    i18n.text("settings-scan"),
                    !state.connected || state.catalog_pending.is_some(),
                    |_, cx| catalog::refresh_catalog(cx),
                )),
        )
}
