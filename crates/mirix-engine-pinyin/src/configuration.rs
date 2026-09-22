use mirix_engine_protocol::{
    Configuration, ConfigurationChoice, ConfigurationControl, ConfigurationGroup,
    ConfigurationKind, PunctuationMode,
};

use crate::profiles::Profiles;

pub(crate) fn localizer(locale: &str) -> Result<mirix_i18n::Localizer, String> {
    let directories = std::env::var_os("MIRIX_CONFIG_DIR")
        .map(|root| std::path::PathBuf::from(root).join("locales"))
        .into_iter()
        .collect::<Vec<_>>();
    mirix_i18n::Localizer::with_catalogs(
        locale,
        &directories,
        &[
            ("en-US", include_str!("../locales/en-US.ftl")),
            ("zh-CN", include_str!("../locales/zh-CN.ftl")),
        ],
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn describe(
    locale: &str,
    profiles: &Profiles,
    can_reload: bool,
    show_reading: bool,
) -> Result<Configuration, String> {
    let text = localizer(locale)?;
    let mut page = Configuration {
        title: text.text("title"),
        description: text.text("description"),
        groups: vec![ConfigurationGroup {
            id: "input".into(),
            label: text.text("input"),
            controls: vec![
                ConfigurationControl {
                    id: "profile".into(),
                    label: text.text("profile"),
                    description: text.text("profile-description"),
                    enabled: true,
                    tray: true,
                    kind: ConfigurationKind::Select {
                        value: profiles.active.clone(),
                        choices: profiles
                            .entries
                            .keys()
                            .map(|name| ConfigurationChoice {
                                value: name.clone(),
                                label: name.clone(),
                            })
                            .collect(),
                    },
                },
                ConfigurationControl {
                    id: "punctuation".into(),
                    label: text.text("punctuation"),
                    description: text.text("punctuation-description"),
                    enabled: true,
                    tray: true,
                    kind: ConfigurationKind::Select {
                        value: match profiles.punctuation_mode {
                            PunctuationMode::Full => "full",
                            PunctuationMode::Half => "half",
                        }
                        .into(),
                        choices: ["full", "half"]
                            .into_iter()
                            .map(|value| ConfigurationChoice {
                                value: value.into(),
                                label: text.text(value),
                            })
                            .collect(),
                    },
                },
                ConfigurationControl {
                    id: "create-profile".into(),
                    label: text.text("create-profile"),
                    description: text.text("create-profile-description"),
                    enabled: true,
                    tray: false,
                    kind: ConfigurationKind::Text {
                        value: String::new(),
                        placeholder: text.text("profile-name-placeholder"),
                        submit_label: text.text("create"),
                        clear_on_submit: true,
                    },
                },
                ConfigurationControl {
                    id: "show-reading".into(),
                    label: text.text("show-reading"),
                    description: text.text("show-reading-description"),
                    enabled: true,
                    tray: false,
                    kind: ConfigurationKind::Switch {
                        value: show_reading,
                    },
                },
            ],
        }],
    };
    page.groups.push(ConfigurationGroup {
        id: "shortcuts".into(),
        label: text.text("shortcuts"),
        controls: vec![ConfigurationControl {
            id: "profile-shortcut".into(),
            label: text.text("profile-shortcut"),
            description: text.text("profile-shortcut-description"),
            enabled: true,
            tray: false,
            kind: ConfigurationKind::Text {
                value: profiles.menu_shortcut.to_string(),
                placeholder: "Ctrl+Shift+P".into(),
                submit_label: text.text("apply"),
                clear_on_submit: false,
            },
        }],
    });
    page.groups.push(ConfigurationGroup {
        id: "profiles".into(),
        label: text.text("manage-profiles"),
        controls: vec![
            profile_text("rename-profile", &text),
            profile_text("duplicate-profile", &text),
            ConfigurationControl {
                id: "archive-profile".into(),
                label: text.text("archive-profile"),
                description: text.text("archive-profile-description"),
                enabled: profiles.entries.len() > 1,
                tray: false,
                kind: ConfigurationKind::Action,
            },
        ],
    });
    page.groups.push(ConfigurationGroup {
        id: "dictionaries".into(),
        label: text.text("dictionaries"),
        controls: vec![ConfigurationControl {
            id: "reload-dictionaries".into(),
            label: text.text("reload-dictionaries"),
            description: text.text("reload-dictionaries-description"),
            enabled: can_reload,
            tray: true,
            kind: ConfigurationKind::Action,
        }],
    });
    if let Some(error) = profiles.storage_error() {
        page.groups.push(ConfigurationGroup {
            id: "storage".into(),
            label: text.text("storage-error"),
            controls: vec![ConfigurationControl {
                id: "retry-save".into(),
                label: text.text("retry-save"),
                description: error,
                enabled: true,
                tray: false,
                kind: ConfigurationKind::Action,
            }],
        });
    }
    Ok(page)
}

fn profile_text(id: &str, text: &mirix_i18n::Localizer) -> ConfigurationControl {
    ConfigurationControl {
        id: id.into(),
        label: text.text(id),
        description: text.text(&format!("{id}-description")),
        enabled: true,
        tray: false,
        kind: ConfigurationKind::Text {
            value: String::new(),
            placeholder: text.text("profile-name-placeholder"),
            submit_label: text.text(id),
            clear_on_submit: true,
        },
    }
}
