use super::*;

fn fixture() -> (tempfile::TempDir, Workspace) {
    let directory = tempfile::tempdir().unwrap();
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/settings/workspace");
    for name in [
        "workspace.json",
        "keyboard.json",
        "pinyin.json",
        "symbols.json",
        "experimental.json",
    ] {
        std::fs::copy(source.join(name), directory.path().join(name)).unwrap();
    }
    let workspace = Workspace::load(directory.path().to_owned()).unwrap();
    (directory, workspace)
}

#[test]
fn mock_pages_are_independent_and_enable_changes_do_not_write_files() {
    let (directory, mut workspace) = fixture();
    let manifest = std::fs::read(directory.path().join("workspace.json")).unwrap();
    let pinyin_file = std::fs::read(directory.path().join("pinyin.json")).unwrap();
    let untouched = workspace.page("symbols", "zh-CN").unwrap().page;
    let page = workspace.page("pinyin", "zh-CN").unwrap();
    workspace
        .apply(
            &page,
            &ConfigurationEvent::Change {
                id: "profile".into(),
                value: mirix_core::ConfigurationValue::String("coding".into()),
            },
        )
        .unwrap();
    assert_ne!(workspace.page("pinyin", "zh-CN").unwrap().page, page.page);
    assert_eq!(workspace.page("symbols", "zh-CN").unwrap().page, untouched);
    workspace.set_enabled("experimental", true).unwrap();
    assert!(
        workspace
            .catalog()
            .entries
            .iter()
            .find(|entry| entry.name == "experimental")
            .unwrap()
            .enabled
    );
    assert_eq!(
        std::fs::read(directory.path().join("workspace.json")).unwrap(),
        manifest
    );
    assert_eq!(
        std::fs::read(directory.path().join("pinyin.json")).unwrap(),
        pinyin_file
    );
    workspace.reload_page("pinyin").unwrap();
    assert_eq!(workspace.page("pinyin", "zh-CN").unwrap().page, page.page);
}

#[test]
fn failed_workspace_reload_preserves_catalog_and_current_edits() {
    let (directory, mut workspace) = fixture();
    workspace.set_enabled("keyboard", false).unwrap();
    let catalog = workspace.catalog();
    let before = workspace.page("pinyin", "zh-CN").unwrap();
    std::fs::write(directory.path().join("symbols.json"), "{broken").unwrap();
    assert!(workspace.reload().is_err());
    assert_eq!(workspace.catalog(), catalog);
    assert_eq!(workspace.page("pinyin", "zh-CN").unwrap(), before);
}

#[test]
fn page_reload_and_actions_use_current_revision() {
    let (_directory, mut workspace) = fixture();
    let page = workspace.page("pinyin", "zh-CN").unwrap();
    workspace.reload_page("pinyin").unwrap();
    assert!(
        workspace
            .apply(
                &page,
                &ConfigurationEvent::Invoke {
                    id: "load-config".into()
                }
            )
            .is_err()
    );
    let current = workspace.page("pinyin", "zh-CN").unwrap();
    workspace
        .apply(
            &current,
            &ConfigurationEvent::Invoke {
                id: "load-config".into(),
            },
        )
        .unwrap();
    assert_eq!(
        workspace.page("pinyin", "zh-CN").unwrap().page,
        current.page
    );
}

#[test]
fn workspace_requires_unique_method_ids_and_an_existing_selection() {
    let (directory, _) = fixture();
    for source in [
        r#"{"selected":"missing","input_methods":[{"id":"keyboard","enabled":true,"page":"keyboard.json"}]}"#,
        r#"{"selected":"keyboard","input_methods":[{"id":"keyboard","enabled":true,"page":"keyboard.json"},{"id":"keyboard","enabled":false,"page":"pinyin.json"}]}"#,
    ] {
        std::fs::write(directory.path().join("workspace.json"), source).unwrap();
        assert!(Workspace::load(directory.path().to_owned()).is_err());
    }
}
