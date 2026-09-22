//! Per-skin motion preferences, independent of skin package contents.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result};
use mirix_config::{Item, Table, update, value};

pub(crate) fn key(skin: Option<&Path>) -> Result<String> {
    skin.map(|path| super::skin_preference(path, &super::skin_roots()))
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(crate) fn read(path: &Path) -> Result<BTreeMap<String, bool>> {
    let document = mirix_config::read(path)?;
    let Some(panel) = document.get("panel") else {
        return Ok(BTreeMap::new());
    };
    let panel = panel.as_table_like().context("panel must be a table")?;
    let Some(animations) = panel.get("animations") else {
        return Ok(BTreeMap::new());
    };
    animations
        .as_table_like()
        .context("panel.animations must be a table")?
        .iter()
        .map(|(key, value)| {
            Ok((
                key.to_owned(),
                value
                    .as_bool()
                    .context("skin animation preference must be a boolean")?,
            ))
        })
        .collect()
}

pub(crate) fn enabled(config: Option<&Path>, skin: Option<&Path>) -> Result<bool> {
    let Some(config) = config else {
        return Ok(true);
    };
    Ok(read(config)?.get(&key(skin)?).copied().unwrap_or(true))
}

pub(crate) fn save(path: &Path, key: &str, enabled: bool) -> Result<()> {
    update(path, |document| {
        if !document.contains_key("panel") {
            document["panel"] = Item::Table(Table::new());
        }
        let panel = document["panel"]
            .as_table_like_mut()
            .context("panel must be a table")?;
        if !panel.contains_key("animations") {
            panel.insert("animations", Item::Table(Table::new()));
        }
        panel
            .get_mut("animations")
            .unwrap()
            .as_table_like_mut()
            .context("panel.animations must be a table")?
            .insert(key, value(enabled));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_skin_choices_preserve_the_selected_skin_and_other_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(
            &path,
            "[panel]\nskin = 'sample'\n[ui]\nlanguage = 'en-US'\n",
        )
        .unwrap();
        save(&path, "sample", false).unwrap();
        save(&path, "/custom/skin", true).unwrap();
        save(&path, "", false).unwrap();
        assert_eq!(
            read(&path).unwrap(),
            BTreeMap::from([
                ("sample".into(), false),
                ("/custom/skin".into(), true),
                ("".into(), false),
            ])
        );
        let document = mirix_config::read(&path).unwrap();
        assert_eq!(document["panel"]["skin"].as_str(), Some("sample"));
        assert_eq!(document["ui"]["language"].as_str(), Some("en-US"));
        assert!(!enabled(Some(&path), None).unwrap());
        assert!(enabled(None, None).unwrap());
    }
}
