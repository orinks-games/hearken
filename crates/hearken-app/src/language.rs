//! Which language the game speaks, and where its catalogs come from.
//!
//! Runtime matching lives in [`hearken_l10n`]. This module picks folders,
//! loads a catalog into [`Ctx`], and drives the Language Settings row.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hearken_l10n::{
    self as l10n, discover_languages, load_translator, offered_codes, override_folders,
    system_locale, Translator, AUTO, ENGLISH, LANG_ENV_SUFFIX, MISSES_ENV_SUFFIX,
};

use crate::ctx::Ctx;
use crate::game::Game;

/// Wide-character speaking weight used while a Chinese or
/// Japanese catalog is active (`PacerConfig::wide_char_weight`).
pub const CJK_WIDE_CHAR_WEIGHT: f64 = 3.0;

/// Folders a translator's file is looked for in: beside the executable,
/// then the data folder. First match wins.
pub fn lang_folders(data_dir: &Path) -> Vec<PathBuf> {
    let game_root = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    override_folders(game_root.as_deref(), Some(data_dir))
}

/// True when the game has at least one non-English catalog that translates
/// something (built-in or drop-in). The Language Settings row is shown only
/// then.
pub fn has_catalogs<G: Game>(data_dir: &Path) -> bool {
    let folders = lang_folders(data_dir);
    offered_codes(&folders, G::translation_catalogs())
        .into_iter()
        .any(|code| code != ENGLISH)
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

/// Whether `code` should use the heavier wide-character pacing.
pub fn uses_cjk_pacing(code: &str) -> bool {
    let base = code.split(['_', '-']).next().unwrap_or(code);
    matches!(base, "zh" | "ja")
}

impl<G: Game> Ctx<G> {
    /// Load the catalog the language setting asks for. Called at launch and
    /// when the player changes Language in Settings. Raises the event
    /// pacer's wide-character weight for Chinese and Japanese, and restores
    /// the game's ordinary weight otherwise.
    pub fn apply_language(&mut self) {
        let setting = self.settings.language.clone();
        let code = resolve_setting::<G>(&setting, &self.paths.data_dir);
        self.translator = load_for_game::<G>(&setting, &self.paths.data_dir);
        self.delivery
            .apply_language_pacing(self.translator.language());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::SoundConfig;
    use crate::testing::Headless;
    use crate::{Game, NoAction, NoCategory, State};
    use hearken_speech::PacerConfig;

    struct Demo;
    struct Title;

    impl Game for Demo {
        const TITLE: &'static str = "Demo";
        const DATA_DIR: &'static str = "hearken-l10n-cjk";
        const ENV_PREFIX: &'static str = "HEARKEN_L10N_CJK";
        type Event = ();
        type Data = ();
        type Settings = ();
        type Action = NoAction;
        type Category = NoCategory;

        fn translation_catalogs() -> &'static [(&'static str, &'static str)] {
            &[(
                "zh_CN",
                "msgid \"\"\nmsgstr \"\"\n\"X-Language-Name: Simplified Chinese\\n\"\n\"X-Language-Autonym: 简体中文\\n\"\n\nmsgid \"New screen.\"\nmsgstr \"新画面。\"\n",
            )]
        }

        fn sound() -> SoundConfig {
            SoundConfig {
                pacer: Some(PacerConfig::default()),
                ..SoundConfig::default()
            }
        }
    }

    impl State<Demo> for Title {
        fn name(&self) -> &'static str {
            "title"
        }
        fn help(&self, _ctx: &Ctx<Demo>) -> String {
            String::new()
        }
    }

    #[test]
    fn chinese_raises_wide_char_weight_and_english_restores_it() {
        let mut h = Headless::<Demo>::new((), Title);
        let base = h
            .app
            .ctx
            .delivery
            .pacer
            .as_ref()
            .expect("pacer")
            .config()
            .wide_char_weight;
        assert!((base - 1.0).abs() < 1e-9);

        h.app.ctx.set_language("zh_CN");
        let raised = h
            .app
            .ctx
            .delivery
            .pacer
            .as_ref()
            .expect("pacer")
            .config()
            .wide_char_weight;
        assert!((raised - CJK_WIDE_CHAR_WEIGHT).abs() < 1e-9);

        h.app.ctx.set_language(ENGLISH);
        let restored = h
            .app
            .ctx
            .delivery
            .pacer
            .as_ref()
            .expect("pacer")
            .config()
            .wide_char_weight;
        assert!((restored - base).abs() < 1e-9);
    }

    #[test]
    fn uses_cjk_pacing_for_chinese_and_japanese_only() {
        assert!(uses_cjk_pacing("zh_CN"));
        assert!(uses_cjk_pacing("ja"));
        assert!(!uses_cjk_pacing("ko"));
        assert!(!uses_cjk_pacing("en"));
        assert!(!uses_cjk_pacing("es"));
    }
}
