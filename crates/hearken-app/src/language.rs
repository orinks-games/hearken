//! Which language the game speaks, and where its catalogs come from.
//!
//! Runtime matching lives in [`hearken_l10n`]. This module picks folders,
//! loads a catalog into [`Ctx`], and drives the Language Settings row.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hearken_l10n::{
    self as l10n, discover_languages, load_translator, offered_codes, override_folders,
    system_locale, Translator, AUTO, LANG_ENV_SUFFIX, MISSES_ENV_SUFFIX,
};

use crate::ctx::Ctx;
use crate::game::Game;

/// Folders a translator's file is looked for in: beside the executable,
/// then the data folder. First match wins.
pub fn lang_folders(data_dir: &Path) -> Vec<PathBuf> {
    let game_root = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    override_folders(game_root.as_deref(), Some(data_dir))
}

/// Register languages from catalogs, then resolve `setting` (`auto` or a
/// code) against the system locale.
pub fn resolve_setting<G: Game>(setting: &str, data_dir: &Path) -> &'static str {
    let folders = lang_folders(data_dir);
    discover_languages(&folders, G::translation_catalogs());
    l10n::resolve(setting, system_locale().as_deref())
}

/// Load the translator for the player's language setting (or `<PREFIX>_LANG`).
pub fn load_for_game<G: Game>(setting: &str, data_dir: &Path) -> Arc<Translator> {
    let folders = lang_folders(data_dir);
    let built_in = G::translation_catalogs();
    discover_languages(&folders, built_in);
    let wanted = std::env::var(format!("{}{LANG_ENV_SUFFIX}", G::ENV_PREFIX))
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| setting.to_string());
    let code = l10n::resolve(&wanted, system_locale().as_deref());
    let misses = std::env::var_os(format!("{}{MISSES_ENV_SUFFIX}", G::ENV_PREFIX))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    Arc::new(load_translator(code, &folders, built_in, misses))
}

/// Codes the Language row cycles through: `auto`, then English, then every
/// catalog that translates at least one line.
pub fn offered_settings<G: Game>(data_dir: &Path) -> Vec<String> {
    let folders = lang_folders(data_dir);
    let mut options = vec![AUTO.to_string()];
    for code in offered_codes(&folders, G::translation_catalogs()) {
        options.push(code.to_string());
    }
    options
}

/// Label for the Language row value.
pub fn language_label(setting: &str, system: Option<&str>) -> String {
    let resolved = l10n::resolve(setting, system);
    let name = l10n::language(resolved).map_or_else(|| "English".to_string(), |lang| lang.label());
    if setting == AUTO || setting.is_empty() {
        format!("Automatic, {name}")
    } else {
        name
    }
}

impl<G: Game> Ctx<G> {
    /// Load the catalog the language setting asks for. Called at launch and
    /// when the player changes Language in Settings.
    pub fn apply_language(&mut self) {
        let setting = self.settings.language.clone();
        let code = resolve_setting::<G>(&setting, &self.paths.data_dir);
        self.translator = load_for_game::<G>(&setting, &self.paths.data_dir);
        log::info!("Language setting {setting:?} resolves to {code}");
    }

    /// `text` in the player's language.
    pub fn tr(&self, text: &str) -> String {
        self.translator.translate(text).into_owned()
    }

    /// Speak `setting` (a code, or `auto`) from now on, and save it.
    pub fn set_language(&mut self, setting: &str) {
        self.settings.language = setting.to_string();
        self.apply_language();
        let _ = self.settings.save(&self.paths);
    }
}
