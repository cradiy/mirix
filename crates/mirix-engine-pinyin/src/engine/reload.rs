//! Stage all dictionary sources before replacing the active in-memory snapshot.
use super::PinyinEngine;
use crate::{
    extensions::Extensions,
    profiles::{ProfileError, load_dictionaries},
};

impl PinyinEngine {
    pub(crate) fn reload_dictionaries(&mut self) -> Result<(), ProfileError> {
        let config = self
            .config_directory
            .as_ref()
            .ok_or_else(|| std::io::Error::other("this engine has no dictionary directory"))?;
        let characters = crate::characters::load(config, self.parser.table())?;
        let words = load_dictionaries(&config.join("dictionaries"), self.parser.table())?;
        let shared = characters.into_iter().chain(words).collect();
        let extensions = Extensions::load(&config.join("extensions"))?;
        let bigrams = super::bigram::load(config)?;
        let profiles = self
            .profiles
            .entries
            .keys()
            .map(|name| {
                let directory = config.join("profiles").join(name);
                Ok((
                    name.clone(),
                    load_dictionaries(&directory.join("dictionaries"), self.parser.table())?,
                    Extensions::load(&directory.join("extensions"))?,
                ))
            })
            .collect::<Result<Vec<_>, ProfileError>>()?;

        let selected = self
            .candidates
            .get(self.selected)
            .map(|candidate| candidate.text.clone());
        self.core_lexicon = shared;
        self.extensions = extensions;
        self.bigrams = bigrams;
        for (name, lexicon, extensions) in profiles {
            let profile = self.profiles.entries.get_mut(&name).unwrap();
            profile.lexicon = lexicon;
            profile.extensions = extensions;
        }
        self.rebuild_active_lexicon();
        self.refresh_candidates();
        if let Some(index) = selected.and_then(|text| {
            self.candidates
                .iter()
                .position(|candidate| candidate.text == text)
        }) {
            self.selected = index;
        }
        self.configuration_revision = self.configuration_revision.wrapping_add(1);
        Ok(())
    }
}
