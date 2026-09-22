//! Seed the editable character dictionary once, then always read it from disk.
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use mirix_pinyin_core::{Lexicon, SyllableTable, import_character_tsv};

use crate::profiles::ProfileError;

pub(crate) const DEFAULT_CHARACTERS: &str = include_str!("../assets/characters.tsv");

pub(crate) fn load(config: &Path, table: &SyllableTable) -> Result<Lexicon, ProfileError> {
    let path = config.join("characters.tsv");
    let source = read_or_seed(&path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    let imported = import_character_tsv(&source, table);
    if !imported.issues.is_empty() {
        let details = imported
            .issues
            .iter()
            .map(|issue| format!("{}:{}: {}", path.display(), issue.line, issue.message))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(ProfileError::Dictionary(details));
    }
    Ok(imported.lexicon)
}

fn read_or_seed(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(source) => return Ok(source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let parent = path
        .parent()
        .expect("character dictionary has a configuration parent");
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(DEFAULT_CHARACTERS.as_bytes())?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(path) {
        Ok(_) => {}
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.error),
    }
    // Another process may have created it meanwhile. Read the winning file.
    fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_dictionary_is_seeded_but_existing_edits_are_authoritative() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("pinyin");
        let path = config.join("characters.tsv");
        let table = SyllableTable::standard();
        let first = load(&config, &table).unwrap();
        assert!(!first.entries().is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_CHARACTERS);

        let edited = "# My dictionary\n闺  guī gui 38845\n女\tnǚ nv\t 300  \n";
        fs::write(&path, edited).unwrap();
        let next = load(&config, &table).unwrap();
        assert_eq!(next.len(), 2);
        assert_eq!(next.exact(&[table.id("gui").unwrap()])[0].text, "闺");
        assert_eq!(
            next.exact(&[table.id("nv").unwrap()])[0].reading.display(),
            "nǚ"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        assert!(next.exact(&[table.id("ai").unwrap()]).is_empty());
    }

    #[test]
    fn invalid_rows_report_file_and_line_without_replacing_the_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("characters.tsv");
        let source = "# Header\n闺 guī gui 38845\n坏 huài huai nope\n";
        fs::write(&path, source).unwrap();
        let error = load(root.path(), &SyllableTable::standard())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(&format!("{}:3:", path.display())));
        assert!(error.contains("invalid character weight"));
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }

    #[test]
    fn empty_dictionary_and_non_file_errors_are_not_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("characters.tsv");
        fs::write(&path, "# Deliberately empty\n").unwrap();
        assert_eq!(
            load(root.path(), &SyllableTable::standard()).unwrap().len(),
            0
        );
        let other = root.path().join("other");
        fs::create_dir_all(other.join("characters.tsv")).unwrap();
        assert!(load(&other, &SyllableTable::standard()).is_err());
        assert!(other.join("characters.tsv").is_dir());
    }
}
