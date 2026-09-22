//! Serialized, atomic edits of shared framework preferences.
use anyhow::{Context, Result};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
};
pub use toml_edit::{Array, DocumentMut, Item, Table, value};

pub fn read(path: &Path) -> Result<DocumentMut> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .parse()
            .with_context(|| format!("parse {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

pub fn update(path: &Path, edit: impl FnOnce(&mut DocumentMut) -> Result<()>) -> Result<()> {
    let parent = path.parent().context("configuration path has no parent")?;
    fs::create_dir_all(parent)?;
    // A separate stable lock survives replacement of the preferences file.
    let lock = fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("toml.lock"))?;
    lock.lock()?;
    let mut document = read(path)?;
    edit(&mut document)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(document.to_string().as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .with_context(|| format!("save {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
