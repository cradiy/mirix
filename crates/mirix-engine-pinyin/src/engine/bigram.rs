use std::{
    fs::File,
    io::{BufReader, ErrorKind},
    path::Path,
};

use mirix_pinyin_core::BigramModel;

use crate::profiles::ProfileError;

pub(super) fn load(config: &Path) -> Result<BigramModel, ProfileError> {
    let path = config.join("bigrams.tsv");
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BigramModel::default()),
        Err(error) => {
            return Err(ProfileError::LanguageModel(format!(
                "{}: {error}",
                path.display()
            )));
        }
    };
    BigramModel::read(BufReader::new(file))
        .map_err(|error| ProfileError::LanguageModel(format!("{}: {error}", path.display())))
}
