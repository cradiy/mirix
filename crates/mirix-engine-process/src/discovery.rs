//! Directory discovery and input-method identity.
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const ENTRYPOINT: &str = "engine";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputMethod {
    pub name: String,
    pub directory: PathBuf,
    pub executable: PathBuf,
}

pub fn default_search_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/usr/share/mirix/input-methods"),
        PathBuf::from("/usr/local/share/mirix/input-methods"),
    ];
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        roots.push(directory.join("input-methods"));
    }
    if let Some(data) = dirs::data_dir() {
        roots.push(data.join("mirix/input-methods"));
    }
    roots
}

/// Discover valid input methods. Later roots override earlier roots with the
/// same directory name, so the user data directory overrides system installs.
pub fn discover(roots: impl IntoIterator<Item = PathBuf>) -> Vec<InputMethod> {
    let mut methods = BTreeMap::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_name(&name) {
                continue;
            }
            let directory = entry.path();
            let executable = directory.join(ENTRYPOINT);
            if !is_executable_file(&executable) {
                continue;
            }
            methods.insert(
                name.clone(),
                InputMethod {
                    name,
                    directory,
                    executable,
                },
            );
        }
    }
    methods.into_values().collect()
}

pub fn find(name: &str, roots: impl IntoIterator<Item = PathBuf>) -> Option<InputMethod> {
    discover(roots)
        .into_iter()
        .find(|method| method.name == name)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.chars().all(|character| !character.is_control())
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mirix-engine-discovery-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn add_method(root: &Path, name: &str, executable: bool) -> PathBuf {
        let directory = root.join(name);
        fs::create_dir(&directory).unwrap();
        let entry = directory.join(ENTRYPOINT);
        fs::write(&entry, "#!/bin/sh\n").unwrap();
        let mode = if executable { 0o755 } else { 0o644 };
        fs::set_permissions(&entry, fs::Permissions::from_mode(mode)).unwrap();
        directory
    }

    #[test]
    fn directory_name_is_identity_and_entrypoint_is_fixed() {
        let root = TempDir::new();
        let directory = add_method(&root.0, "cloud-pinyin", true);
        add_method(&root.0, "not-executable", false);
        add_method(&root.0, "bad\nname", true);

        assert_eq!(
            discover([root.0.clone()]),
            vec![InputMethod {
                name: "cloud-pinyin".into(),
                executable: directory.join(ENTRYPOINT),
                directory,
            }]
        );
    }

    #[test]
    fn later_search_root_overrides_same_name() {
        let system = TempDir::new();
        let user = TempDir::new();
        add_method(&system.0, "pinyin", true);
        let expected = add_method(&user.0, "pinyin", true);

        let methods = discover([system.0.clone(), user.0.clone()]);
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].directory, expected);
    }
}
