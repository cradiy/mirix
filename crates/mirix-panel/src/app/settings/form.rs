//! Engine-independent rendering shared by live settings and file previews.
mod path;
mod select;
#[cfg(test)]
mod tests;
mod text;

use super::style::*;
use gpui::{AnyElement, App, Entity, Window, div, prelude::*, px, rgb};
use mirix_core::{Configuration, ConfigurationEvent, ConfigurationKind, ConfigurationValue};
use std::{collections::BTreeMap, rc::Rc};
use uic::components::selection::{Checkbox, Switch};

pub(super) type OnEvent = Rc<dyn Fn(ConfigurationEvent, &mut Window, &mut App)>;

pub(super) fn text_reset(page: &Configuration, event: &ConfigurationEvent) -> Option<String> {
    let ConfigurationEvent::Change {
        id,
        value: ConfigurationValue::String(_),
    } = event
    else {
        return None;
    };
    page.groups
        .iter()
        .flat_map(|group| &group.controls)
        .find_map(|control| {
            (control.id == *id
                && matches!(
                    control.kind,
                    ConfigurationKind::Text {
                        clear_on_submit: true,
                        ..
                    }
                ))
            .then(|| id.clone())
        })
}

#[derive(Default)]
pub(super) struct Form {
    selects: BTreeMap<String, Entity<select::SelectField>>,
    paths: BTreeMap<String, Entity<path::PathField>>,
    texts: BTreeMap<String, Entity<text::TextField>>,
    reset_text: Option<String>,
    last_reset: Option<u64>,
}

impl Form {
    // Queue resets for render, rather than updating an input reentrantly from its callback.
    pub(super) fn clear_text(&mut self, id: String) {
        self.reset_text = Some(id);
    }

    pub(super) fn acknowledge(&mut self, receipt: Option<(u64, String)>) {
        if let Some((request, id)) = receipt
            && self.last_reset != Some(request)
        {
            self.last_reset = Some(request);
            self.clear_text(id);
        }
    }

    pub(super) fn render(
        &mut self,
        page: &Configuration,
        busy: bool,
        localizer: &mirix_i18n::Localizer,
        on_event: OnEvent,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let reset_text = self.reset_text.take();
        self.texts.retain(|id, _| {
            page.groups.iter().any(|group| {
                group.controls.iter().any(|control| {
                    control.id == *id && matches!(control.kind, ConfigurationKind::Text { .. })
                })
            })
        });
        self.paths.retain(|id, _| {
            page.groups.iter().any(|group| {
                group.controls.iter().any(|control| {
                    control.id == *id && matches!(control.kind, ConfigurationKind::Path { .. })
                })
            })
        });
        self.selects.retain(|id, _| {
            page.groups.iter().any(|group| {
                group.controls.iter().any(|control| {
                    control.id == *id && matches!(control.kind, ConfigurationKind::Select { .. })
                })
            })
        });
        let mut form = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_4()
            .text_size(px(15.))
            .text_color(rgb(TEXT));
        for group in &page.groups {
            let mut rows = card();
            if !group.label.is_empty() {
                rows = rows.child(eyebrow(group.label.clone()));
            }
            for (index, control) in group.controls.iter().enumerate() {
                let disabled = busy || !control.enabled;
                let id = control.id.clone();
                let on_event = on_event.clone();
                let widget = match &control.kind {
                    ConfigurationKind::Switch { value } => Switch::new(id.clone(), *value)
                        .label(control.label.clone())
                        .disabled(disabled)
                        .appearance(switch_appearance())
                        .on_change(move |value, window, cx| {
                            on_event(
                                ConfigurationEvent::Change {
                                    id: id.clone(),
                                    value: ConfigurationValue::Bool(value),
                                },
                                window,
                                cx,
                            )
                        })
                        .into_any_element(),
                    ConfigurationKind::Checkable { value } => Checkbox::new(id.clone(), *value)
                        .aria_label(control.label.clone())
                        .disabled(disabled)
                        .appearance(checkbox_appearance())
                        .on_change(move |value, window, cx| {
                            on_event(
                                ConfigurationEvent::Change {
                                    id: id.clone(),
                                    value: ConfigurationValue::Bool(value),
                                },
                                window,
                                cx,
                            )
                        })
                        .into_any_element(),
                    ConfigurationKind::Checklist { values, choices } => {
                        let mut list = div()
                            .id(format!("checklist-{id}"))
                            .w_full()
                            .max_h(px(224.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_2();
                        for (index, choice) in choices.iter().enumerate() {
                            let checked = values.contains(&choice.value);
                            let kind = control.kind.clone();
                            let value = choice.value.clone();
                            let id = id.clone();
                            let on_event = on_event.clone();
                            list = list.child(
                                Checkbox::new(format!("{id}-{index}"), checked)
                                    .label_text(choice.label.clone())
                                    .disabled(disabled)
                                    .appearance(checkbox_appearance())
                                    .py_1()
                                    .on_change(move |checked, window, cx| {
                                        on_event(
                                            ConfigurationEvent::Change {
                                                id: id.clone(),
                                                value: ConfigurationValue::Strings(
                                                    kind.with_checked(&value, checked),
                                                ),
                                            },
                                            window,
                                            cx,
                                        )
                                    }),
                            );
                        }
                        list.into_any_element()
                    }
                    ConfigurationKind::Select { .. } => {
                        let field = self
                            .selects
                            .entry(id.clone())
                            .or_insert_with(|| cx.new(|cx| select::SelectField::new(window, cx)));
                        field.update(cx, |field, cx| {
                            field.sync(control.clone(), disabled, on_event, window, cx)
                        });
                        field.clone().into_any_element()
                    }
                    ConfigurationKind::Path { .. } => {
                        let field = self
                            .paths
                            .entry(id.clone())
                            .or_insert_with(|| cx.new(|_| path::PathField::default()));
                        field.update(cx, |field, cx| {
                            field.sync(control.clone(), disabled, on_event, localizer, cx)
                        });
                        field.clone().into_any_element()
                    }
                    ConfigurationKind::Text { .. } => {
                        let field = self
                            .texts
                            .entry(id.clone())
                            .or_insert_with(|| cx.new(|cx| text::TextField::new(window, cx)));
                        field.update(cx, |field, cx| {
                            field.sync(
                                control.clone(),
                                disabled,
                                reset_text.as_deref() == Some(&id),
                                on_event,
                                cx,
                            )
                        });
                        field.clone().into_any_element()
                    }
                    ConfigurationKind::Action => action_button(
                        id.clone(),
                        control.label.clone(),
                        disabled,
                        move |window, cx| {
                            on_event(ConfigurationEvent::Invoke { id: id.clone() }, window, cx)
                        },
                    )
                    .into_any_element(),
                };
                let mut label = div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(control.label.clone());
                if !control.description.is_empty() {
                    label = label.child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(MUTED))
                            .child(control.description.clone()),
                    );
                }
                let row = div().w_full().when(index > 0, |row| {
                    row.pt_5().border_t_1().border_color(rgb(LINE))
                });
                let row = match &control.kind {
                    ConfigurationKind::Switch { .. } | ConfigurationKind::Checkable { .. } => {
                        row.flex().items_center().gap_4().child(label).child(widget)
                    }
                    ConfigurationKind::Select { .. } => row
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(label)
                        .child(div().w(px(260.)).max_w_full().child(widget)),
                    ConfigurationKind::Checklist { .. }
                    | ConfigurationKind::Path { .. }
                    | ConfigurationKind::Text { .. } => {
                        row.flex().flex_col().gap_3().child(label).child(widget)
                    }
                    ConfigurationKind::Action => {
                        let mut row = row.flex().flex_col().items_start().gap_2().child(widget);
                        if !control.description.is_empty() {
                            row = row.child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(MUTED))
                                    .child(control.description.clone()),
                            );
                        }
                        row
                    }
                };
                rows = rows.child(row);
            }
            form = form.child(rows);
        }
        form.into_any_element()
    }
}
