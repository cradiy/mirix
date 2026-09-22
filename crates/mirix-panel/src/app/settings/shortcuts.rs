//! Framework routing controls. No input-method-specific settings are interpreted here.
use super::{Settings, catalog, form::Form, style::*};
use crate::i18n::UiLanguage;
use gpui::{App, Div, Window, div, prelude::*, rgb};
use mirix_core::{
    Configuration, ConfigurationControl, ConfigurationEvent, ConfigurationGroup, ConfigurationKind,
    ConfigurationValue,
};
use mirix_ipc::{InputMethodCatalog, Request};
use std::rc::Rc;

pub(super) fn render(form: &mut Form, window: &mut Window, cx: &mut App) -> Div {
    let i18n = cx.global::<UiLanguage>().localizer.clone();
    let state = cx.global::<Settings>();
    let Some(catalog) = state.catalog.clone() else {
        return div().child(i18n.text("settings-unavailable"));
    };
    let busy = !state.connected || state.catalog_pending.is_some();
    let error = state.catalog_error.clone();
    let controls = ["switch", "skin"]
        .into_iter()
        .map(|id| {
            let locked = catalog.locked_shortcuts.iter().any(|key| key == id);
            ConfigurationControl {
                id: id.into(),
                label: i18n.text(&format!("settings-shortcut-{id}")),
                description: if locked {
                    i18n.text("settings-shortcut-locked")
                } else {
                    String::new()
                },
                enabled: !locked,
                tray: false,
                kind: ConfigurationKind::Text {
                    value: catalog.shortcuts.get(id).cloned().unwrap_or_default(),
                    placeholder: "Ctrl+Space".into(),
                    submit_label: i18n.text("settings-apply"),
                    clear_on_submit: false,
                },
            }
        })
        .collect();
    let page = Configuration {
        title: String::new(),
        description: String::new(),
        groups: vec![ConfigurationGroup {
            id: "shortcuts".into(),
            label: i18n.text("settings-nav-shortcuts"),
            controls,
        }],
    };
    let mut order = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(i18n.text("settings-method-order"));
    let visible = installed_order_indices(&catalog);
    for (position, &index) in visible.iter().enumerate() {
        let name = &catalog.enabled_order[index];
        let label = state
            .entries
            .get(name)
            .and_then(|entry| entry.page.as_ref())
            .map(|page| page.page.title.clone())
            .unwrap_or_else(|| name.clone());
        let locked = busy || catalog.selection_locked;
        order = order.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().child(format!("{}. {label}", position + 1)))
                .child(action_button(
                    format!("method-up-{index}"),
                    i18n.text("settings-move-up"),
                    locked || position == 0,
                    move |_, cx| move_method(position, -1, cx),
                ))
                .child(action_button(
                    format!("method-down-{index}"),
                    i18n.text("settings-move-down"),
                    locked || position + 1 == visible.len(),
                    move |_, cx| move_method(position, 1, cx),
                )),
        );
    }
    let rendered = form.render(
        &page,
        busy,
        &i18n,
        Rc::new(|event, _, cx| {
            if let ConfigurationEvent::Change {
                id,
                value: ConfigurationValue::String(value),
            } = event
            {
                set_shortcut(id, value, cx);
            }
        }),
        window,
        cx,
    );
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(rendered)
        .child(order)
        .when_some(error, |body, message| {
            body.child(div().text_color(rgb(0xeea5ad)).child(message))
        })
}

fn installed_order_indices(catalog: &InputMethodCatalog) -> Vec<usize> {
    catalog
        .enabled_order
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            catalog
                .entries
                .iter()
                .any(|entry| &entry.name == name)
                .then_some(index)
        })
        .collect()
}

fn move_method(position: usize, direction: isize, cx: &mut App) {
    let Some(catalog) = cx.global::<Settings>().catalog.as_ref() else {
        return;
    };
    let visible = installed_order_indices(catalog);
    let Some(&index) = visible.get(position) else {
        return;
    };
    let Some(target) = position
        .checked_add_signed(direction)
        .and_then(|i| visible.get(i))
        .copied()
    else {
        return;
    };
    let mut names = catalog.enabled_order.clone();
    names.swap(index, target);
    if cx.global::<Settings>().mock.is_some() {
        cx.global_mut::<Settings>()
            .catalog
            .as_mut()
            .unwrap()
            .enabled_order = names;
        super::repaint(cx);
        return;
    }
    catalog::send(cx, |request, state| {
        let catalog = state.catalog.as_ref()?;
        Some(Request::ReorderInputMethods {
            request,
            session: catalog.session.clone(),
            revision: catalog.revision,
            names,
        })
    });
}

fn set_shortcut(name: String, value: String, cx: &mut App) {
    let shortcut = match value.parse::<mirix_core::Shortcut>() {
        Ok(shortcut) => shortcut.to_string(),
        Err(message) => {
            super::feedback(false, message, cx);
            return;
        }
    };
    if cx.global::<Settings>().mock.is_some() {
        let catalog = cx.global_mut::<Settings>().catalog.as_mut().unwrap();
        if catalog
            .shortcuts
            .iter()
            .any(|(key, value)| key != &name && value == &shortcut)
        {
            super::feedback(false, "Shortcut is already assigned".into(), cx);
            return;
        }
        catalog.shortcuts.insert(name, shortcut);
        super::repaint(cx);
        return;
    }
    catalog::send(cx, |request, state| {
        let catalog = state.catalog.as_ref()?;
        Some(Request::SetShortcut {
            request,
            session: catalog.session.clone(),
            revision: catalog.revision,
            name,
            value: shortcut,
        })
    });
}
