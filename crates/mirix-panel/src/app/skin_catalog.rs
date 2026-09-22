//! Explicit skin discovery; no filesystem watcher or background reload.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SkinEntry {
    pub name: String,
    pub path: Option<PathBuf>,
}

pub(super) fn scan(active: Option<&Path>) -> Vec<SkinEntry> {
    discover(&crate::config::skin_roots(), active)
}

pub(super) fn discover(roots: &[PathBuf], active: Option<&Path>) -> Vec<SkinEntry> {
    let mut paths = BTreeMap::new();
    for root in roots {
        for name in mirix_skin::discover(std::slice::from_ref(root)) {
            if let Ok(path) = root.join(&name).canonicalize() {
                paths.insert(name, path);
            }
        }
    }
    if let Some(path) = active
        && mirix_skin::BuiltinSkin::from_path(path).is_none()
        && let Some(name) = path.file_name().and_then(|name| name.to_str())
    {
        paths.insert(name.to_owned(), path.to_owned());
    }
    mirix_skin::BuiltinSkin::ALL
        .into_iter()
        .map(|skin| SkinEntry {
            name: skin.name().into(),
            path: skin.selection(),
        })
        .chain(paths.into_iter().map(|(name, path)| SkinEntry {
            name,
            path: Some(path),
        }))
        .collect()
}
