//! Finding and loading PO catalogs from folders a game names.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::l10n::{self, po, Catalog, Translator, ENGLISH};

/// Suffix after a game's `ENV_PREFIX` for a one-run language override
/// (`HELLO_LANG=es`).
pub const LANG_ENV_SUFFIX: &str = "_LANG";
/// Suffix after a game's `ENV_PREFIX` for a miss-log path
/// (`HELLO_L10N_MISSES=/tmp/misses.txt`).
pub const MISSES_ENV_SUFFIX: &str = "_L10N_MISSES";

/// The operating system's language tag, e.g. `"es-MX"`.
pub fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}

/// Folders to search for override catalogs, first match wins.
pub fn override_folders(game_root: Option<&Path>, data_dir: Option<&Path>) -> Vec<PathBuf> {
    game_root
        .into_iter()
        .chain(data_dir)
        .map(|dir| dir.join("lang"))
        .collect()
}

/// Catalog text for `code`: the first `{code}.po` in `folders`, else a
/// built-in entry from `built_in`.
pub fn catalog_text<'a>(
    folders: &[PathBuf],
    code: &str,
    built_in: &[(&str, &'a str)],
) -> Option<Cow<'a, str>> {
    for folder in folders {
        let path = folder.join(format!("{code}.po"));
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                log::info!("Using the translation file {}", path.display());
                return Some(Cow::Owned(text));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => log::warn!("Could not read {}: {err}", path.display()),
        }
    }
    built_in
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, text)| Cow::Borrowed(*text))
}

/// A translator for `code`, falling back to English when there is no usable
/// catalog. `folders` are searched first; `built_in` is `(code, po text)`.
pub fn load_translator(
    code: &str,
    folders: &[PathBuf],
    built_in: &[(&str, &str)],
    misses_path: Option<PathBuf>,
) -> Translator {
    if code == ENGLISH {
        return Translator::english();
    }
    let Some(text) = catalog_text(folders, code, built_in) else {
        log::warn!("No translation for {code}; speaking English");
        return Translator::english();
    };
    let catalog = match Catalog::from_po(code, &text) {
        Ok(catalog) => catalog,
        Err(err) => {
            log::error!("The {code} translation could not be read ({err}); speaking English");
            return Translator::english();
        }
    };
    for problem in &catalog.problems {
        log::warn!("{code} translation, {problem}");
    }
    log::info!("Speaking {code}: {} translated lines", catalog.len());
    let translator = Translator::new(catalog);
    match misses_path.filter(|path| !path.as_os_str().is_empty()) {
        Some(path) => translator.log_misses_to(path),
        None => translator,
    }
}

/// `(code, name, autonym)` for every `.po` in `folders` and every built-in
/// catalog, skipping codes already in [`l10n::LANGUAGES`].
pub fn added_languages_in(
    folders: &[PathBuf],
    built_in: &[(&str, &str)],
) -> Vec<(String, String, String)> {
    let mut found: Vec<(String, String, String)> = Vec::new();
    let mut consider = |code: &str, text: &str| {
        let plausible = !code.is_empty()
            && code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plausible
            || l10n::LANGUAGES.iter().any(|lang| lang.code == code)
            || found.iter().any(|(known, _, _)| known == code)
        {
            return;
        }
        let name = po::header_field(text, "X-Language-Name");
        let autonym = po::header_field(text, "X-Language-Autonym");
        let (name, autonym) = match (name, autonym) {
            (Some(name), Some(autonym)) => (name, autonym),
            (Some(one), None) | (None, Some(one)) => (one.clone(), one),
            (None, None) => (code.to_string(), code.to_string()),
        };
        found.push((code.to_string(), name, autonym));
    };
    for (code, text) in built_in {
        consider(code, text);
    }
    for folder in folders {
        consider_folder(folder, &mut consider);
    }
    found
}

fn consider_folder(folder: &Path, consider: &mut impl FnMut(&str, &str)) {
    let mut dropped: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "po"))
        .collect();
    dropped.sort();
    for path in dropped {
        let Some(code) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => consider(code, &text),
            Err(err) => log::warn!("Could not read {}: {err}", path.display()),
        }
    }
}

/// Register languages found in `folders` / `built_in` with [`l10n::add_languages`].
/// Safe to call more than once; later calls are ignored by `add_languages`.
pub fn discover_languages(folders: &[PathBuf], built_in: &[(&str, &str)]) {
    let added = added_languages_in(folders, built_in);
    for (code, _, autonym) in &added {
        log::info!("Offering {autonym} ({code}) from its translation file");
    }
    l10n::add_languages(added);
}

/// Language codes a Settings row should offer: English, then every discovered
/// language whose catalog translates at least one line.
pub fn offered_codes(folders: &[PathBuf], built_in: &[(&str, &str)]) -> Vec<&'static str> {
    discover_languages(folders, built_in);
    let mut codes = vec![ENGLISH];
    for lang in l10n::languages() {
        if lang.code == ENGLISH {
            continue;
        }
        let text = catalog_text(folders, lang.code, built_in);
        let count = text
            .and_then(|text| Catalog::from_po(lang.code, &text).ok())
            .map_or(0, |catalog| catalog.len());
        if count > 0 {
            codes.push(lang.code);
        }
    }
    codes
}

/// Shared translator handle used by Hearken's `Ctx`.
pub type SharedTranslator = Arc<Translator>;

#[cfg(test)]
mod tests {
    use super::*;

    fn write_catalog(folder: &Path, code: &str, line: &str) {
        std::fs::create_dir_all(folder).expect("lang folder");
        std::fs::write(
            folder.join(format!("{code}.po")),
            format!(
                "msgid \"\"\nmsgstr \"X-Language-Name: Example\\nX-Language-Autonym: Ejemplo\\n\"\n\nmsgid \"New screen.\"\nmsgstr \"{line}\"\n"
            ),
        )
        .expect("catalog written");
    }

    #[test]
    fn game_folder_wins_over_data_folder() {
        let game = tempfile::tempdir().expect("game");
        let data = tempfile::tempdir().expect("data");
        let folders = override_folders(Some(game.path()), Some(data.path()));
        write_catalog(&data.path().join("lang"), "es", "Desde los datos.");
        write_catalog(&game.path().join("lang"), "es", "Desde el juego.");
        let text = catalog_text(&folders, "es", &[]).expect("catalog");
        assert!(text.contains("Desde el juego."), "{text}");
    }

    #[test]
    fn load_translator_falls_back_when_missing() {
        let tr = load_translator("nope", &[], &[], None);
        assert!(tr.is_english());
        assert_eq!(tr.translate("Hello."), "Hello.");
    }

    #[test]
    fn discover_reads_header_names() {
        let game = tempfile::tempdir().expect("game");
        write_catalog(&game.path().join("lang"), "nl", "Hallo.");
        let folders = override_folders(Some(game.path()), None);
        let added = added_languages_in(&folders, &[]);
        assert!(added
            .iter()
            .any(|(c, n, a)| c == "nl" && n == "Example" && a == "Ejemplo"));
    }
}
