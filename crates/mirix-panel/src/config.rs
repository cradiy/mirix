//! Persistent user preferences; explicit startup arguments take precedence.

pub(crate) mod animation;

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use mirix_config::{Item, Table, read, update, value};

pub(crate) fn default_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("locate user configuration directory")?
        .join("mirix/config.toml"))
}

pub(crate) fn skin_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/usr/share/mirix/skins"),
        PathBuf::from("/usr/local/share/mirix/skins"),
    ];
    if let Some(data) = dirs::data_dir() {
        roots.push(data.join("mirix/skins"));
    }
    roots
}

fn installed_skin(id: &OsStr, roots: &[PathBuf]) -> Option<PathBuf> {
    roots.iter().rev().find_map(|root| {
        let directory = root.join(id);
        directory
            .join(mirix_skin::MANIFEST_FILE)
            .is_file()
            .then(|| directory.canonicalize().ok())
            .flatten()
    })
}

fn skin_preference(skin: &Path, roots: &[PathBuf]) -> Result<String> {
    if let Some(builtin) = mirix_skin::BuiltinSkin::from_path(skin) {
        return Ok(builtin.id().into());
    }
    let skin = skin
        .canonicalize()
        .context("resolve selected skin directory")?;
    if let Some(id) = skin.file_name()
        && mirix_skin::BuiltinSkin::from_path(Path::new(id)).is_none()
        && installed_skin(id, roots).as_deref() == Some(skin.as_path())
    {
        return Ok(id.to_str().context("skin ID is not UTF-8")?.to_owned());
    }
    Ok(skin.to_str().context("skin path is not UTF-8")?.to_owned())
}

pub(crate) fn resolve_skin(path: &Path, explicit: Option<PathBuf>) -> Result<Option<PathBuf>> {
    if explicit.is_some() {
        return Ok(canonical_skin(explicit));
    }
    let document = read(path)?;
    let Some(panel) = document.get("panel") else {
        return Ok(None);
    };
    let panel = panel
        .as_table_like()
        .context("panel must be a TOML table")?;
    let Some(skin) = panel.get("skin") else {
        return Ok(None);
    };
    let skin = skin
        .as_str()
        .context("panel.skin must be a skin ID or directory path string")?;
    if skin.is_empty() {
        bail!("panel.skin must not be empty; remove it to use the default skin");
    }
    if let Some(builtin) = mirix_skin::BuiltinSkin::from_path(Path::new(skin)) {
        return Ok(builtin.selection());
    }
    if !skin.contains('/') && skin != "." && skin != ".." {
        let installed = installed_skin(OsStr::new(skin), &skin_roots());
        if installed.is_none() {
            eprintln!("mirix-panel: warning: skin {skin:?} is not installed; using built-in Light");
        }
        return Ok(installed);
    }
    let skin = PathBuf::from(skin);
    Ok(Some(if skin.is_absolute() {
        skin
    } else {
        path.parent()
            .context("configuration path has no parent")?
            .join(skin)
    }))
}

/// Built-in IDs are identities, not paths relative to the process directory.
pub(crate) fn canonical_skin(skin: Option<PathBuf>) -> Option<PathBuf> {
    skin.and_then(|path| match mirix_skin::BuiltinSkin::from_path(&path) {
        Some(builtin) => builtin.selection(),
        None => Some(path.canonicalize().unwrap_or(path)),
    })
}

/// Load the effective selection before looking up its animation preferences.
/// Skin failures are cosmetic and must not prevent the panel from starting.
pub(crate) fn load_skin(
    config: Option<&Path>,
    explicit: Option<PathBuf>,
) -> Result<(mirix_skin::Skin, Option<PathBuf>)> {
    let path = match config {
        Some(config) => resolve_skin(config, explicit)?,
        None => canonical_skin(explicit),
    };
    Ok(load_selected_skin(path))
}

pub(crate) fn load_selected_skin(path: Option<PathBuf>) -> (mirix_skin::Skin, Option<PathBuf>) {
    if let Some(path) = path {
        match mirix_skin::load(&path) {
            Ok(skin) => return (skin, Some(path)),
            Err(error) => eprintln!(
                "mirix-panel: warning: cannot load skin {}: {error}; using built-in Light",
                path.display(),
            ),
        }
    }
    (mirix_skin::BuiltinSkin::Light.skin(), None)
}

pub(crate) fn save_skin(path: &Path, skin: Option<&Path>) -> Result<()> {
    // Read afresh so selecting a skin preserves other settings and comments.
    update(path, |document| {
        if !document.contains_key("panel") {
            document["panel"] = Item::Table(Table::new());
        }
        let panel = document["panel"]
            .as_table_like_mut()
            .context("panel must be a TOML table")?;
        match skin {
            Some(skin) => {
                panel.insert("skin", value(skin_preference(skin, &skin_roots())?));
            }
            None => {
                panel.remove("skin");
            }
        }
        Ok(())
    })
}

pub(crate) fn resolve_language(path: &Path, explicit: Option<String>) -> Result<Option<String>> {
    let language = match explicit {
        Some(language) => Some(language),
        None => read(path)?
            .get("ui")
            .map(|ui| -> Result<Option<String>> {
                let table = ui.as_table_like().context("ui must be a TOML table")?;
                table
                    .get("language")
                    .map(|language| {
                        language
                            .as_str()
                            .context("ui.language must be a language tag or auto")
                            .map(str::to_owned)
                    })
                    .transpose()
            })
            .transpose()?
            .flatten(),
    };
    Ok(language.filter(|language| language != "auto"))
}

pub(crate) fn save_language(path: &Path, language: Option<&str>) -> Result<()> {
    update(path, |document| {
        if !document.contains_key("ui") {
            document["ui"] = Item::Table(Table::new());
        }
        let table = document["ui"]
            .as_table_like_mut()
            .context("ui must be a TOML table")?;
        table.insert("language", value(language.unwrap_or("auto")));
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn selection_survives_reload_and_default_clears_it_without_losing_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        let skin = root.path().join("sample-skin");
        fs::create_dir(&skin).unwrap();
        fs::write(
            &path,
            "# My settings\n[input]\nengine = 'pinyin'\n[panel]\n# Layout\nlayout = 'vertical'\n",
        )
        .unwrap();
        save_skin(&path, Some(&skin)).unwrap();
        assert_eq!(
            resolve_skin(&path, None).unwrap(),
            Some(skin.canonicalize().unwrap())
        );
        save_skin(&path, None).unwrap();
        assert_eq!(resolve_skin(&path, None).unwrap(), None);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# My settings"));
        assert!(saved.contains("# Layout"));
        assert_eq!(
            read(&path).unwrap()["input"]["engine"].as_str(),
            Some("pinyin")
        );
        assert_eq!(
            read(&path).unwrap()["panel"]["layout"].as_str(),
            Some("vertical")
        );
    }

    #[test]
    fn explicit_skin_overrides_saved_skin_and_relative_paths_use_config_directory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        assert_eq!(resolve_skin(&path, None).unwrap(), None);
        fs::write(&path, "[panel]\nskin = 'skins/sample-skin'\n").unwrap();
        assert_eq!(
            resolve_skin(&path, None).unwrap(),
            Some(root.path().join("skins/sample-skin"))
        );
        let explicit = PathBuf::from("/custom/skin");
        assert_eq!(
            resolve_skin(&path, Some(explicit.clone())).unwrap(),
            Some(explicit)
        );
    }

    #[test]
    fn first_selection_creates_configuration_and_failed_save_keeps_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("mirix/config.toml");
        let skin = root.path().join("sample-skin");
        fs::create_dir(&skin).unwrap();
        save_skin(&path, Some(&skin)).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(
            resolve_skin(&path, None).unwrap(),
            Some(skin.canonicalize().unwrap())
        );
        assert!(save_skin(&path, Some(&root.path().join("missing"))).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
    }

    #[test]
    fn malformed_configuration_is_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        fs::write(&path, "[panel").unwrap();
        assert!(save_skin(&path, None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[panel");
    }

    #[test]
    fn language_selection_persists_without_overwriting_skin_or_shortcuts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        assert_eq!(resolve_language(&path, None).unwrap(), None);
        fs::write(
            &path,
            "# preferences\n[panel]\nskin = '/skins/night'\n[shortcuts]\nprofile = 'Alt+P'\n",
        )
        .unwrap();
        save_language(&path, Some("zh-CN")).unwrap();
        assert_eq!(
            resolve_language(&path, None).unwrap().as_deref(),
            Some("zh-CN")
        );
        assert_eq!(
            resolve_language(&path, Some("en-US".into()))
                .unwrap()
                .as_deref(),
            Some("en-US")
        );
        assert_eq!(resolve_language(&path, Some("auto".into())).unwrap(), None);
        save_language(&path, None).unwrap();
        assert_eq!(resolve_language(&path, None).unwrap(), None);
        let document = read(&path).unwrap();
        assert_eq!(document["ui"]["language"].as_str(), Some("auto"));
        assert_eq!(document["panel"]["skin"].as_str(), Some("/skins/night"));
        assert_eq!(document["shortcuts"]["profile"].as_str(), Some("Alt+P"));
        assert!(fs::read_to_string(&path).unwrap().contains("# preferences"));
    }

    #[test]
    fn installed_skin_ids_respect_directory_priority_and_preserve_explicit_selection() {
        let root = tempfile::tempdir().unwrap();
        let roots = vec![root.path().join("system"), root.path().join("user")];
        for directory in &roots {
            let skin = directory.join("sample");
            fs::create_dir_all(&skin).unwrap();
            fs::write(skin.join(mirix_skin::MANIFEST_FILE), "").unwrap();
        }
        let user_skin = roots[1].join("sample").canonicalize().unwrap();
        assert_eq!(
            installed_skin(OsStr::new("sample"), &roots),
            Some(user_skin.clone())
        );
        assert_eq!(skin_preference(&user_skin, &roots).unwrap(), "sample");

        // An explicitly selected lower-priority skin must remain that exact skin.
        let system_skin = roots[0].join("sample").canonicalize().unwrap();
        assert_eq!(
            skin_preference(&system_skin, &roots).unwrap(),
            system_skin.to_str().unwrap()
        );
        assert_eq!(installed_skin(OsStr::new("uninstalled"), &roots), None);
    }

    #[test]
    fn unavailable_skin_uses_default_animation_preferences_without_rewriting_config() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        let missing_id = format!(
            "uninstalled-{}",
            root.path().file_name().unwrap().to_string_lossy()
        );
        let original = format!("[panel]\nskin = '{missing_id}'\n[panel.animations]\n'' = false\n");
        fs::write(&config, &original).unwrap();
        let (_, effective) = load_skin(Some(&config), None).unwrap();
        assert_eq!(effective, None);
        assert!(!animation::enabled(Some(&config), effective.as_deref()).unwrap());
        assert_eq!(fs::read_to_string(&config).unwrap(), original);

        // A valid command-line choice still takes priority over an unavailable saved ID.
        let selected = root.path().join("selected");
        fs::create_dir(&selected).unwrap();
        fs::write(
            selected.join(mirix_skin::MANIFEST_FILE),
            "format_version = 1\n",
        )
        .unwrap();
        let (_, effective) = load_skin(Some(&config), Some(selected.clone())).unwrap();
        assert_eq!(effective, Some(selected.canonicalize().unwrap()));
    }

    #[test]
    fn unavailable_or_unreadable_skin_content_does_not_abort_loading() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        let original = "[panel]\nskin = 'skins/absent'\n";
        fs::write(&config, original).unwrap();
        assert_eq!(load_skin(Some(&config), None).unwrap().1, None);

        let empty = root.path().join("empty");
        fs::create_dir(&empty).unwrap();
        let invalid = root.path().join("invalid");
        fs::create_dir(&invalid).unwrap();
        fs::write(invalid.join(mirix_skin::MANIFEST_FILE), "[window").unwrap();
        for path in [root.path().join("absent"), empty, invalid] {
            // Both normal startup and offline CLI previews must use the fallback.
            assert_eq!(
                load_skin(Some(&config), Some(path.clone())).unwrap().1,
                None
            );
            assert_eq!(load_skin(None, Some(path)).unwrap().1, None);
        }
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
    }
}
