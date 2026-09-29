//! Embedded English and Serbian text selected once at startup.
use crate::error::Error;
use std::{collections::BTreeMap, sync::OnceLock};

/// Supported interface languages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    /// English, also used before startup selects a language.
    #[default]
    En,
    /// Serbian Latin.
    Sr,
}

/// Where to download the latest release.
pub const RELEASES_URL: &str = "https://github.com/stlk0/yettel-cwmp/releases/latest";
/// Issue tracker used for safe internal-error reports.
pub const ISSUES_URL: &str = "https://github.com/stlk0/yettel-cwmp/issues";
/// User guide.
pub const README_URL: &str = "https://github.com/stlk0/yettel-cwmp#readme";

const SOURCES: [&str; 2] = [
    include_str!("../locales/en.json"),
    include_str!("../locales/sr.json"),
];
static LANGUAGE: OnceLock<Language> = OnceLock::new();
static CATALOGS: OnceLock<[BTreeMap<String, String>; 2]> = OnceLock::new();

/// Select the interface language before starting the terminal or worker threads.
pub fn init(language: Language) {
    let _ = LANGUAGE.set(language);
}

/// Return the selected language, or English before initialization.
pub fn language() -> Language {
    LANGUAGE.get().copied().unwrap_or_default()
}

// Embedded catalogs are immutable and their JSON is checked by the tests below.
#[allow(clippy::expect_used)]
fn catalogs() -> &'static [BTreeMap<String, String>; 2] {
    CATALOGS.get_or_init(|| {
        SOURCES.map(|source| serde_json::from_str(source).expect("invalid bundled locale JSON"))
    })
}

/// Look up a key without changing the selected language.
pub fn lookup(language: Language, key: &str) -> &'static str {
    let index = match language {
        Language::En => 0,
        Language::Sr => 1,
    };
    let translated = catalogs()[index].get(key);
    debug_assert!(translated.is_some(), "missing translation key");
    translated
        .or_else(|| catalogs()[0].get(key))
        .map_or("", String::as_str)
}

/// Look up a key in the selected language.
pub fn t(key: &str) -> &'static str {
    lookup(language(), key)
}

/// Substitute placeholders in trusted catalog text without reprocessing inserted values.
pub fn tf(key: &str, values: &[(&str, &str)]) -> String {
    let mut template = t(key);
    let mut output = String::with_capacity(template.len());
    while let Some((before, after)) = template.split_once('{') {
        output.push_str(before);
        let Some((name, rest)) = after.split_once('}') else {
            output.push('{');
            output.push_str(after);
            return output;
        };
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
            output.push_str(value);
        } else {
            output.push('{');
            output.push_str(name);
            output.push('}');
        }
        template = rest;
    }
    output.push_str(template);
    output
}

/// Localized guidance for an error, in the selected language.
pub fn error_message(error: Error) -> String {
    let status = match error {
        Error::HttpStatus(status) => status.to_string(),
        _ => String::new(),
    };
    tf(
        &format!("error.{}", error.code()),
        &[
            ("n", &status),
            ("releases_url", RELEASES_URL),
            ("issues_url", ISSUES_URL),
        ],
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use std::{collections::BTreeSet, fs, path::Path};

    fn placeholders(value: &str) -> BTreeSet<&str> {
        value
            .split('{')
            .skip(1)
            .map(|part| part.split_once('}').unwrap().0)
            .collect()
    }

    #[test]
    fn languages_have_identical_keys_and_nonempty_translations() {
        let [english, serbian] = catalogs();
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            serbian.keys().collect::<Vec<_>>()
        );
        assert!(
            english
                .values()
                .chain(serbian.values())
                .all(|value| !value.trim().is_empty())
        );
    }

    #[test]
    fn languages_have_identical_placeholders_for_every_key() {
        let [english, serbian] = catalogs();
        for (key, value) in english {
            assert_eq!(placeholders(value), placeholders(&serbian[key]), "{key}");
        }
    }

    // Keys deferred until CLI parsing completes and menu keys are also quoted literals.
    // Only source before a test module counts: tests must not keep unused UI copy alive.
    fn source_keys(directory: &Path, used: &mut BTreeSet<String>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                source_keys(&path, used);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_some_and(|name| name != "tests.rs")
            {
                let source = fs::read_to_string(&path).unwrap();
                let source = source.split("#[cfg(test)]").next().unwrap();
                for literal in source.split('"').skip(1).step_by(2) {
                    if catalogs()[0].contains_key(literal) {
                        used.insert(literal.to_owned());
                    }
                }
                for call in ["t(", "tf("] {
                    for (offset, _) in source.match_indices(call) {
                        if source[..offset]
                            .chars()
                            .next_back()
                            .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
                        {
                            continue;
                        }
                        let argument = &source[offset + call.len()..];
                        if let Some(argument) = argument.trim_start().strip_prefix('"') {
                            let key = argument.split('"').next().unwrap();
                            assert!(
                                catalogs()[0].contains_key(key),
                                "missing key {key} in {}",
                                path.display()
                            );
                            used.insert(key.to_owned());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn source_keys_exist_and_catalogs_have_no_unused_keys() {
        let mut used = BTreeSet::new();
        source_keys(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut used,
        );
        // Error codes select their messages dynamically, without a duplicate dispatch table.
        used.extend(Error::ALL.map(|error| format!("error.{}", error.code())));
        assert_eq!(used, catalogs()[0].keys().cloned().collect());
    }

    #[test]
    fn every_error_code_has_a_translation() {
        let source = include_str!("error.rs");
        let code_match = source.split("pub const fn code(self)").nth(1).unwrap();
        let code_match = code_match.split("pub fn ").next().unwrap();
        let declared_codes: BTreeSet<_> = code_match
            .lines()
            .filter_map(|line| line.split_once("=> \""))
            .map(|(_, value)| value.split('"').next().unwrap())
            .collect();
        assert_eq!(
            declared_codes,
            Error::ALL.map(Error::code).into_iter().collect()
        );
        for error in Error::ALL {
            let key = format!("error.{}", error.code());
            for language in [Language::En, Language::Sr] {
                assert!(!lookup(language, &key).is_empty());
            }
        }
    }

    #[test]
    fn placeholder_values_are_not_interpreted_as_further_placeholders() {
        let result = tf(
            "help.body",
            &[
                ("path", "{device}"),
                ("device", "synthetic-router"),
                ("url", "{path}"),
            ],
        );
        assert!(result.contains("{device}"));
        assert!(result.contains("synthetic-router"));
        assert!(result.contains("{path}"));
    }
}
