//! Application-owned UI language; switching never changes engine state.

use gpui::Global;
use mirix_i18n::Localizer;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct UiLanguage {
    pub localizer: Arc<Localizer>,
    pub preference: Option<String>,
    pub directories: Vec<PathBuf>,
}

impl Global for UiLanguage {}

pub(crate) fn directories(config: Option<&Path>) -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/usr/share/mirix/locales"),
        PathBuf::from("/usr/local/share/mirix/locales"),
    ];
    if let Some(directory) = config.and_then(Path::parent) {
        directories.push(directory.join("locales"));
    }
    directories
}
