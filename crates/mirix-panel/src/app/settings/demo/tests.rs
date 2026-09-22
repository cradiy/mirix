use super::*;

fn fixture() -> Configuration {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/settings/controls.json"
    )))
    .unwrap()
}

fn change(id: &str, value: ConfigurationValue) -> ConfigurationEvent {
    ConfigurationEvent::Change {
        id: id.into(),
        value,
    }
}

#[test]
fn preview_interactions_change_memory_only_and_reload_restores_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let source = serde_json::to_string(&fixture()).unwrap();
    std::fs::write(&path, &source).unwrap();
    let mut document = Document::load(path.clone()).unwrap();
    for event in [
        change(
            "display-name",
            ConfigurationValue::String("编程 Work".into()),
        ),
        change("learning", ConfigurationValue::Bool(false)),
        change("confirm", ConfigurationValue::Bool(true)),
        change("profile", ConfigurationValue::String("coding".into())),
        change("sources", ConfigurationValue::Strings(vec!["emoji".into()])),
        change(
            "config-file",
            ConfigurationValue::String("/tmp/my settings.json".into()),
        ),
        change(
            "data-directory",
            ConfigurationValue::String("/tmp/input data".into()),
        ),
        change("data-directory", ConfigurationValue::String(String::new())),
    ] {
        let before = document.page.clone();
        document.apply(&event).unwrap();
        assert_ne!(document.page, before);
        document.page.validate().unwrap();
    }
    let before_action = document.page.clone();
    document
        .apply(&ConfigurationEvent::Invoke {
            id: "example-action".into(),
        })
        .unwrap();
    assert_eq!(document.page, before_action);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    document.reload().unwrap();
    assert_eq!(document.page, fixture());
}

#[test]
fn invalid_reload_and_disabled_edits_preserve_last_usable_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let mut document = Document {
        path: path.clone(),
        page: fixture(),
    };
    let before = document.page.clone();
    assert!(
        document
            .apply(&change("locked", ConfigurationValue::Bool(false)))
            .is_err()
    );
    assert!(
        document
            .apply(&change(
                "sources",
                ConfigurationValue::Strings(vec!["unknown".into()])
            ))
            .is_err()
    );
    assert_eq!(document.page, before);
    assert!(
        document
            .apply(&change(
                "locked-text",
                ConfigurationValue::String("edit".into())
            ))
            .is_err()
    );
    assert_eq!(document.page, before);
    std::fs::write(&path, "{broken").unwrap();
    assert!(document.reload().is_err());
    assert_eq!(document.page, before);
    let mut invalid = fixture();
    invalid.groups[0].controls[1].id = invalid.groups[0].controls[0].id.clone();
    std::fs::write(&path, serde_json::to_string(&invalid).unwrap()).unwrap();
    assert!(document.reload().is_err());
    assert_eq!(document.page, before);
}

#[test]
fn demo_load_action_reads_file_and_preserves_current_page_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let mut document = Document {
        path: path.clone(),
        page: fixture(),
    };
    document
        .apply(&change(
            "profile",
            ConfigurationValue::String("coding".into()),
        ))
        .unwrap();
    let mut from_file = fixture();
    from_file.title = "Reloaded from disk".into();
    std::fs::write(&path, serde_json::to_string(&from_file).unwrap()).unwrap();
    let reload = ConfigurationEvent::Invoke {
        id: "load-config".into(),
    };
    assert!(document.apply(&reload).unwrap());
    assert_eq!(document.page, from_file);

    std::fs::write(&path, "{ invalid JSON").unwrap();
    assert!(document.apply(&reload).is_err());
    assert_eq!(document.page, from_file);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ invalid JSON");
}

#[test]
fn resettable_text_in_demo_clears_only_after_accepted_submission() {
    let mut page = fixture();
    let field = page
        .groups
        .iter_mut()
        .flat_map(|group| &mut group.controls)
        .find(|control| control.id == "display-name")
        .unwrap();
    if let ConfigurationKind::Text {
        clear_on_submit, ..
    } = &mut field.kind
    {
        *clear_on_submit = true;
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("demo.json");
    std::fs::write(&path, serde_json::to_string(&page).unwrap()).unwrap();
    let mut document = Document::load(path).unwrap();
    let submitted = change("display-name", ConfigurationValue::String("Work".into()));
    document.apply(&submitted).unwrap();
    assert_eq!(document.page, page);
    assert_eq!(
        super::super::form::text_reset(&document.page, &submitted).as_deref(),
        Some("display-name")
    );
    assert!(
        document
            .apply(&change("display-name", ConfigurationValue::Bool(true)))
            .is_err()
    );
    assert_eq!(document.page, page);
    assert_eq!(Document::load(document.path.clone()).unwrap().page, page);
}
