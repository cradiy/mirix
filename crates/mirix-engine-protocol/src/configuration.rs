//! Declarative engine settings. Values and persistence belong to the engine.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configuration {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub groups: Vec<ConfigurationGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationGroup {
    pub id: String,
    pub label: String,
    pub controls: Vec<ConfigurationControl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationControl {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub tray: bool,
    pub kind: ConfigurationKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationChoice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfigurationKind {
    Switch {
        value: bool,
    },
    Checkable {
        value: bool,
    },
    Checklist {
        values: Vec<String>,
        choices: Vec<ConfigurationChoice>,
    },
    Select {
        value: String,
        choices: Vec<ConfigurationChoice>,
    },
    Path {
        value: String,
        target: ConfigurationPathTarget,
    },
    Text {
        value: String,
        #[serde(default)]
        placeholder: String,
        submit_label: String,
        #[serde(default)]
        clear_on_submit: bool,
    },
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationPathTarget {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigurationValue {
    Bool(bool),
    String(String),
    Strings(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfigurationEvent {
    Change {
        id: String,
        value: ConfigurationValue,
    },
    Invoke {
        id: String,
    },
}

impl Configuration {
    pub fn validate(&self) -> Result<(), String> {
        let mut groups = std::collections::HashSet::new();
        let mut controls = std::collections::HashSet::new();
        for group in &self.groups {
            if group.id.is_empty() || !groups.insert(&group.id) {
                return Err("configuration groups need unique, nonempty IDs".into());
            }
            for control in &group.controls {
                if control.id.is_empty() || !controls.insert(&control.id) {
                    return Err("configuration controls need unique, nonempty IDs".into());
                }
                if let ConfigurationKind::Path { value, .. } = &control.kind
                    && value.contains('\0')
                {
                    return Err(format!("invalid path for {}", control.id));
                }
                let (choices, selected) = match &control.kind {
                    ConfigurationKind::Select { value, choices } => {
                        (choices, std::slice::from_ref(value))
                    }
                    ConfigurationKind::Checklist { values, choices } => {
                        (choices, values.as_slice())
                    }
                    _ => continue,
                };
                let mut unique = std::collections::HashSet::new();
                if choices.iter().any(|choice| !unique.insert(&choice.value))
                    || !valid_selection(selected, choices)
                {
                    return Err(format!("invalid choices for {}", control.id));
                }
            }
        }
        Ok(())
    }

    pub fn validate_event(&self, event: &ConfigurationEvent) -> Result<(), String> {
        self.validate()?;
        let (ConfigurationEvent::Change { id, .. } | ConfigurationEvent::Invoke { id }) = event;
        let control = self
            .groups
            .iter()
            .flat_map(|group| &group.controls)
            .find(|control| &control.id == id)
            .ok_or_else(|| format!("unknown setting: {id}"))?;
        if !control.enabled {
            return Err(format!("setting is disabled: {id}"));
        }
        match (&control.kind, event) {
            (ConfigurationKind::Action, ConfigurationEvent::Invoke { .. })
            | (
                ConfigurationKind::Switch { .. } | ConfigurationKind::Checkable { .. },
                ConfigurationEvent::Change {
                    value: ConfigurationValue::Bool(_),
                    ..
                },
            ) => Ok(()),
            (
                ConfigurationKind::Text { .. },
                ConfigurationEvent::Change {
                    value: ConfigurationValue::String(_),
                    ..
                },
            ) => Ok(()),
            (
                ConfigurationKind::Path { .. },
                ConfigurationEvent::Change {
                    value: ConfigurationValue::String(value),
                    ..
                },
            ) if !value.contains('\0') => Ok(()),
            (
                ConfigurationKind::Select { choices, .. },
                ConfigurationEvent::Change {
                    value: ConfigurationValue::String(value),
                    ..
                },
            ) if choices.iter().any(|choice| &choice.value == value) => Ok(()),
            (
                ConfigurationKind::Checklist { choices, .. },
                ConfigurationEvent::Change {
                    value: ConfigurationValue::Strings(values),
                    ..
                },
            ) if valid_selection(values, choices) => Ok(()),
            _ => Err(format!("invalid operation for setting: {id}")),
        }
    }
}

fn enabled_by_default() -> bool {
    true
}

fn valid_selection(values: &[String], choices: &[ConfigurationChoice]) -> bool {
    let mut seen = std::collections::HashSet::new();
    values
        .iter()
        .all(|value| seen.insert(value) && choices.iter().any(|choice| &choice.value == value))
}

impl ConfigurationKind {
    /// Produce deterministic checklist membership in display order.
    pub fn with_checked(&self, value: &str, checked: bool) -> Vec<String> {
        let Self::Checklist { values, choices } = self else {
            return Vec::new();
        };
        choices
            .iter()
            .filter(|choice| {
                if choice.value == value {
                    checked
                } else {
                    values.contains(&choice.value)
                }
            })
            .map(|choice| choice.value.clone())
            .collect()
    }
}
