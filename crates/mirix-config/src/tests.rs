use super::*;

#[test]
fn concurrent_preference_edits_preserve_other_fields_and_comments() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        "# Keep my settings\n[shortcuts]\nswitch = 'Ctrl+Space'\n",
    )
    .unwrap();
    std::thread::scope(|scope| {
        for key in ["input_methods", "panel", "ui"] {
            let path = &path;
            scope.spawn(move || {
                update(path, |document| {
                    document[key] = Item::Table(Table::new());
                    document[key]["test"] = value(key);
                    Ok(())
                })
                .unwrap();
            });
        }
    });
    let document = read(&path).unwrap();
    for key in ["input_methods", "panel", "ui"] {
        assert_eq!(document[key]["test"].as_str(), Some(key));
    }
    assert_eq!(document["shortcuts"]["switch"].as_str(), Some("Ctrl+Space"));
    assert!(document.to_string().contains("# Keep my settings"));
}

#[test]
fn failed_edit_does_not_replace_preferences() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    fs::write(&path, "[ui]\nlanguage = 'zh-CN'\n").unwrap();
    let original = fs::read(&path).unwrap();
    assert!(
        update(&path, |document| {
            document["ui"]["language"] = value("en-US");
            anyhow::bail!("cancelled")
        })
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), original);
}
