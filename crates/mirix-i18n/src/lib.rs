//! Fluent localization with system-language negotiation and external FTL packs.

use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Context, Result};
use fluent_bundle::{FluentResource, concurrent::FluentBundle};
use fluent_langneg::{NegotiationStrategy, negotiate_languages};
use unic_langid::LanguageIdentifier;

pub use fluent_bundle::FluentArgs;

const ENGLISH: &str = include_str!("../locales/en-US.ftl");
const CHINESE: &str = include_str!("../locales/zh-CN.ftl");
type Bundle = FluentBundle<FluentResource>;

pub struct Localizer {
    language: LanguageIdentifier,
    available: Vec<AvailableLanguage>,
    bundles: Vec<Bundle>,
}

pub struct AvailableLanguage {
    pub id: String,
    pub name: String,
}

impl Localizer {
    /// Later directories override earlier ones, including embedded translations.
    /// `None` uses system preferences; an explicit language is a BCP 47 tag.
    pub fn load(language: Option<&str>, directories: &[PathBuf]) -> Result<Self> {
        let requested = match language {
            Some(language) => vec![
                language
                    .parse()
                    .context("UI language must be a BCP 47 tag, e.g. en-US or zh-CN")?,
            ],
            None => sys_locale::get_locales()
                .filter_map(|locale| locale.parse().ok())
                .collect(),
        };
        Self::load_requested(&requested, directories)
    }

    fn load_requested(requested: &[LanguageIdentifier], directories: &[PathBuf]) -> Result<Self> {
        Self::load_catalogs(
            requested,
            directories,
            &[("en-US", ENGLISH), ("zh-CN", CHINESE)],
        )
    }

    /// Engine-owned catalogs use the same negotiation and external-pack loading.
    pub fn with_catalogs(
        language: &str,
        directories: &[PathBuf],
        catalogs: &[(&str, &str)],
    ) -> Result<Self> {
        Self::load_catalogs(&[language.parse()?], directories, catalogs)
    }

    fn load_catalogs(
        requested: &[LanguageIdentifier],
        directories: &[PathBuf],
        catalogs: &[(&str, &str)],
    ) -> Result<Self> {
        let english: LanguageIdentifier = "en-US".parse()?;
        let mut resources = BTreeMap::<LanguageIdentifier, Vec<FluentResource>>::new();
        let fallback = catalogs
            .iter()
            .find(|(id, _)| *id == "en-US")
            .context("catalogs must include en-US")?
            .1;
        for (id, source) in catalogs {
            resources.insert(id.parse()?, vec![parse_resource((*source).to_owned(), id)?]);
        }
        for directory in directories {
            let entries = match std::fs::read_dir(directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    tracing::warn!(path = %directory.display(), %error, "read language directory");
                    continue;
                }
            };
            let mut paths = entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect::<Vec<_>>();
            paths.sort();
            for path in paths {
                if path.extension().and_then(|extension| extension.to_str()) != Some("ftl") {
                    continue;
                }
                let Some(language) = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.parse::<LanguageIdentifier>().ok())
                else {
                    tracing::warn!(path = %path.display(), "language filename must be a BCP 47 tag");
                    continue;
                };
                let resource = std::fs::read_to_string(&path)
                    .with_context(|| format!("read {}", path.display()))
                    .and_then(|source| parse_resource(source, &path.display().to_string()));
                match resource {
                    Ok(resource) => resources.entry(language).or_default().push(resource),
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "skip invalid language file")
                    }
                }
            }
        }
        let available = resources.keys().cloned().collect::<Vec<_>>();
        let language = negotiate_languages(
            requested,
            &available,
            Some(&english),
            NegotiationStrategy::Lookup,
        )
        .first()
        .copied()
        .cloned()
        .unwrap_or_else(|| english.clone());
        let mut catalog = resources
            .into_iter()
            .map(|(language, resources)| {
                let bundle = make_bundle(language.clone(), resources);
                (language, bundle)
            })
            .collect::<BTreeMap<_, _>>();
        let available = catalog
            .iter()
            .map(|(language, bundle)| AvailableLanguage {
                id: language.to_string(),
                name: message(bundle, "language-name", None)
                    .unwrap_or_else(|| language.to_string()),
            })
            .collect();
        let order = if language == english {
            vec![english.clone()]
        } else {
            vec![language.clone(), english.clone()]
        };
        let mut bundles = order
            .into_iter()
            .filter_map(|language| catalog.remove(&language))
            .collect::<Vec<_>>();
        // Keep the built-in English messages available even when an external
        // English override has a missing reference or invalid arguments.
        bundles.push(make_bundle(
            english,
            vec![parse_resource(fallback.to_owned(), "built-in en-US")?],
        ));
        Ok(Self {
            language,
            available,
            bundles,
        })
    }

    pub fn language(&self) -> &LanguageIdentifier {
        &self.language
    }

    pub fn available_languages(&self) -> &[AvailableLanguage] {
        &self.available
    }

    pub fn text(&self, id: &str) -> String {
        self.format(id, None)
    }

    /// Missing or invalid messages use English; language packs can be partial.
    pub fn format(&self, id: &str, args: Option<&FluentArgs<'_>>) -> String {
        for bundle in &self.bundles {
            if let Some(text) = message(bundle, id, args) {
                return text;
            }
        }
        id.to_owned()
    }
}

fn make_bundle(language: LanguageIdentifier, resources: Vec<FluentResource>) -> Bundle {
    let mut bundle = Bundle::new_concurrent(vec![language]);
    for resource in resources {
        bundle.add_resource_overriding(resource);
    }
    bundle
}

fn message(bundle: &Bundle, id: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
    let pattern = bundle.get_message(id)?.value()?;
    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, args, &mut errors);
    if errors.is_empty() {
        Some(text.into_owned())
    } else {
        tracing::warn!(message = id, ?errors, "format UI translation");
        None
    }
}

fn parse_resource(source: String, name: &str) -> Result<FluentResource> {
    FluentResource::try_new(source)
        .map_err(|(_, errors)| anyhow::anyhow!("parse {name}: {errors:?}"))
}

#[cfg(test)]
mod tests;
