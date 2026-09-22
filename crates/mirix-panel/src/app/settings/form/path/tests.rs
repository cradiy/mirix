use super::selected_path;
use std::path::PathBuf;

#[test]
fn cancellation_emits_no_value_and_paths_preserve_unicode_and_spaces() {
    assert_eq!(selected_path(None).unwrap(), None);
    assert_eq!(selected_path(Some(vec![])).unwrap(), None);
    let path = "/tmp/my dictionary/用户词库.json";
    assert_eq!(
        selected_path(Some(vec![PathBuf::from(path)])).unwrap(),
        Some(path.to_owned())
    );
    assert!(selected_path(Some(vec!["/tmp/a".into(), "/tmp/b".into()])).is_err());
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_are_not_silently_changed() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let path = PathBuf::from(OsString::from_vec(b"/tmp/\xff".to_vec()));
    assert!(selected_path(Some(vec![path])).is_err());
}
