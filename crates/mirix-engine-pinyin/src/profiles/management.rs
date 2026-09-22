//! Engine-owned profile lifecycle. Archive preserves files outside discovery roots.
use super::*;

#[derive(Clone, Copy)]
pub(crate) enum ProfileAction {
    Rename,
    Duplicate,
    Archive,
}

impl Profiles {
    pub fn manage(&mut self, action: ProfileAction, name: &str) -> Result<String, String> {
        let name = name.trim();
        if !matches!(action, ProfileAction::Archive) {
            if name.is_empty()
                || matches!(name, "." | "..")
                || name.contains(['/', '\\'])
                || name.chars().any(char::is_control)
            {
                return Err(
                    "Enter a profile name without path separators or control characters".into(),
                );
            }
            if self.entries.contains_key(name) {
                return Err(format!("Profile already exists: {name}"));
            }
        } else if self.entries.len() <= 1 {
            return Err("Keep at least one profile".into());
        }
        self.checkpoint()?;
        let old = self.active.clone();
        let message = if let (Some(writer), Some(directories)) = (&self.writer, &self.directories) {
            let (reply, result) = mpsc::channel();
            writer
                .sender
                .as_ref()
                .unwrap()
                .send(WriteEvent::Manage {
                    action,
                    old: old.clone(),
                    name: name.into(),
                    config: directories.config.clone(),
                    reply,
                })
                .map_err(|_| "Profile writer stopped")?;
            // Completion is authoritative: do not report a failure while an operation can still finish.
            result.recv().map_err(|_| "Profile writer stopped")??
        } else {
            String::new()
        };
        match action {
            ProfileAction::Rename => {
                let profile = self.entries.remove(&old).unwrap();
                self.entries.insert(name.into(), profile);
                self.activate(name);
            }
            ProfileAction::Duplicate => {
                self.entries.insert(name.into(), self.current().clone());
                self.activate(name);
            }
            ProfileAction::Archive => {
                self.entries.remove(&old);
                let next = self.entries.keys().next().unwrap().clone();
                self.activate(&next);
            }
        }
        Ok(message)
    }
}

pub(super) fn files(
    action: ProfileAction,
    old: &str,
    name: &str,
    config: &Path,
    data: &Path,
) -> Result<String, String> {
    let old_config = config.join(old);
    let old_data = data.join(old);
    let (new_config, new_data) = if matches!(action, ProfileAction::Archive) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let suffix = format!("{old}-{stamp}");
        (
            config
                .parent()
                .unwrap()
                .join("archived-profiles")
                .join(&suffix),
            data.parent()
                .unwrap()
                .join("archived-profiles")
                .join(suffix),
        )
    } else {
        (config.join(name), data.join(name))
    };
    let run = || -> io::Result<()> {
        for path in [&new_config, &new_data] {
            if path.try_exists()? {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} already exists", path.display()),
                ));
            }
            fs::create_dir_all(path.parent().unwrap())?;
        }
        if matches!(action, ProfileAction::Duplicate) {
            // Stage both trees outside discovery before publishing the copied profile.
            let config_stage = tempfile::Builder::new()
                .prefix("profile-copy-")
                .tempdir_in(config.parent().unwrap())?;
            let data_stage = tempfile::Builder::new()
                .prefix("profile-copy-")
                .tempdir_in(data.parent().unwrap())?;
            let prepared_config = config_stage.path().join("profile");
            let prepared_data = data_stage.path().join("profile");
            copy_tree(&old_config, &prepared_config)?;
            if old_data.exists() {
                copy_tree(&old_data, &prepared_data)?;
            } else {
                fs::create_dir(&prepared_data)?;
            }
            fs::rename(&prepared_data, &new_data)?;
            if let Err(error) = fs::rename(&prepared_config, &new_config) {
                fs::rename(&new_data, &prepared_data)?;
                return Err(error);
            }
        } else {
            let has_data = old_data.exists();
            if has_data {
                fs::rename(&old_data, &new_data)?;
            }
            if let Err(error) = fs::rename(&old_config, &new_config) {
                if has_data {
                    fs::rename(&new_data, &old_data)?;
                }
                return Err(error);
            }
        }
        Ok(())
    };
    run().map_err(|error| format!("Profile operation failed: {error}"))?;
    Ok(if matches!(action, ProfileAction::Archive) {
        format!(
            "Profile archived. Files retained at {} and {}",
            new_config.display(),
            new_data.display()
        )
    } else {
        format!("Profile selected: {name}")
    })
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(io::Error::other(format!(
                "Cannot copy non-regular profile entry: {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_duplicate_and_archive_preserve_independent_files() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config/profiles");
        let data = root.path().join("data/profiles");
        fs::create_dir_all(config.join("work/dictionaries")).unwrap();
        fs::create_dir_all(data.join("work")).unwrap();
        fs::write(config.join("work/dictionaries/custom.tsv"), "content").unwrap();
        fs::write(data.join("work/learning.json"), "{}").unwrap();
        files(ProfileAction::Duplicate, "work", "copy", &config, &data).unwrap();
        fs::write(data.join("copy/learning.json"), "changed").unwrap();
        assert_eq!(
            fs::read_to_string(data.join("work/learning.json")).unwrap(),
            "{}"
        );
        files(ProfileAction::Rename, "copy", "renamed", &config, &data).unwrap();
        assert!(!config.join("copy").exists());
        assert!(config.join("renamed/dictionaries/custom.tsv").exists());
        let message = files(ProfileAction::Archive, "renamed", "", &config, &data).unwrap();
        assert!(message.contains("archived-profiles"));
        assert!(!data.join("renamed").exists());
        assert_eq!(
            fs::read_dir(data.parent().unwrap().join("archived-profiles"))
                .unwrap()
                .count(),
            1
        );
    }
}
