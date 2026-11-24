//! Finding and loading PO catalogs from folders a game names.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::l10n::{self, po, Catalog, Translator, ENGLISH};

/// Suffix after a game's `ENV_PREFIX` for a one-run language override
/// (`HELLO_LANG=es`).
pub const LANG_ENV_SUFFIX: &str = "_LANG";
/// Suffix after a game's `ENV_PREFIX` for a miss-log path
/// (`HELLO_L10N_MISSES=/tmp/misses.txt`).
pub const MISSES_ENV_SUFFIX: &str = "_L10N_MISSES";

/// The operating system's language tag, e.g. `"es-MX"`.
///
/// [`with_system_locale`] can pin a tag for one call stack (tests).
pub fn system_locale() -> Option<String> {
    if let Ok(guard) = SYSTEM_LOCALE_OVERRIDE.lock() {
        if let Some(over) = guard.as_ref() {
            return over.clone();
        }
    }
    sys_locale::get_locale()
}

static SYSTEM_LOCALE_OVERRIDE: std::sync::Mutex<Option<Option<String>>> =
    std::sync::Mutex::new(None);

/// Run `f` as if the OS locale were `locale` (`None` = no locale).
///
/// Used by Hearken's own tests so Automatic → Chinese pacing can be checked
/// without depending on the machine's real language.
pub fn with_system_locale<R>(locale: Option<&str>, f: impl FnOnce() -> R) -> R {
    let previous = {
        let mut guard = SYSTEM_LOCALE_OVERRIDE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.replace(locale.map(str::to_string))
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    {
        let mut guard = SYSTEM_LOCALE_OVERRIDE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = previous;
    }
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
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
    offered_codes_by(folders, built_in, |code, text| translates_any(code, &text))
}

/// [`offered_codes`] for catalogs built into the game. Whether a built-in
/// catalog translates anything is worked out once per run and remembered,
/// because reading every catalog takes about a second and the Settings
/// screen asks each time it opens. A drop-in file in `folders` is still read
/// every time, so a translator's edits show without a restart.
pub fn offered_codes_built_in(
    folders: &[PathBuf],
    built_in: &'static [(&'static str, &'static str)],
) -> Vec<&'static str> {
    offered_codes_by(folders, built_in, |code, text| match text {
        Cow::Borrowed(text) => built_in_translates(code, text),
        Cow::Owned(text) => translates_any(code, &text),
    })
}

/// Work out in the background which built-in catalogs translate anything,
/// so the first [`offered_codes_built_in`] (the Settings screen) has the
/// answer ready instead of reading every catalog while the player waits.
pub fn warm_built_in(built_in: &'static [(&'static str, &'static str)]) {
    if built_in.iter().all(|(code, _)| *code == ENGLISH) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("hearken-l10n-warm".into())
        .spawn(move || {
            for (code, text) in built_in {
                if *code != ENGLISH {
                    built_in_translates(code, text);
                }
            }
        });
    if let Err(err) = spawned {
        log::warn!("Could not check the translations ahead of time: {err}");
    }
}

fn offered_codes_by<'a>(
    folders: &[PathBuf],
    built_in: &[(&str, &'a str)],
    translates: impl Fn(&str, Cow<'a, str>) -> bool,
) -> Vec<&'static str> {
    discover_languages(folders, built_in);
    let mut codes = vec![ENGLISH];
    for lang in l10n::languages() {
        if lang.code == ENGLISH {
            continue;
        }
        if catalog_text(folders, lang.code, built_in)
            .is_some_and(|text| translates(lang.code, text))
        {
            codes.push(lang.code);
        }
    }
    codes
}

/// True when `text` reads as a catalog with at least one translated line.
fn translates_any(code: &str, text: &str) -> bool {
    Catalog::from_po(code, text).is_ok_and(|catalog| !catalog.is_empty())
}

/// [`translates_any`] for a built-in catalog, remembered by the text's
/// address: a `'static` catalog never moves or changes.
fn built_in_translates(code: &str, text: &'static str) -> bool {
    type Key = (usize, usize);
    static KNOWN: Mutex<Vec<(Key, bool)>> = Mutex::new(Vec::new());
    let key = (text.as_ptr() as usize, text.len());
    let known = |list: &Vec<(Key, bool)>| list.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    if let Some(answer) = KNOWN.lock().ok().and_then(|list| known(&list)) {
        return answer;
    }
    let answer = translates_any(code, text);
    if let Ok(mut list) = KNOWN.lock() {
        if known(&list).is_none() {
            list.push((key, answer));
        }
    }
    answer
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
    fn built_in_offers_stay_right_when_remembered() {
        static BUILT_IN: &[(&str, &str)] = &[
            (
                "es",
                "msgid \"\"\nmsgstr \"\"\n\nmsgid \"New screen.\"\nmsgstr \"Pantalla nueva.\"\n",
            ),
            (
                "fr",
                "msgid \"\"\nmsgstr \"\"\n\nmsgid \"New screen.\"\nmsgstr \"\"\n",
            ),
        ];
        warm_built_in(BUILT_IN);
        let first = offered_codes_built_in(&[], BUILT_IN);
        assert!(first.contains(&"es"), "{first:?}");
        assert!(
            !first.contains(&"fr"),
            "an empty catalog is not offered: {first:?}"
        );
        assert_eq!(offered_codes_built_in(&[], BUILT_IN), first);
        assert_eq!(offered_codes(&[], BUILT_IN), first);
        // A drop-in file is read fresh, so a translator's new line shows.
        let game = tempfile::tempdir().expect("game");
        let folders = override_folders(Some(game.path()), None);
        write_catalog(&game.path().join("lang"), "fr", "Nouvel écran.");
        assert!(offered_codes_built_in(&folders, BUILT_IN).contains(&"fr"));
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
