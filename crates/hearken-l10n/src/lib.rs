//! Language catalogs for Hearken games.
//!
//! A game keeps spoken lines in English and ships gettext PO files under
//! `lang/`. Load a [`Translator`], then pass finished lines through
//! [`Translator::translate`] (Hearken's `Ctx::say` does this when a
//! translator is installed). Missing entries stay English.
//!
//! See `docs/l10n.md` for the file layout and a minimal getting-started
//! example.

#![allow(missing_docs)]

mod check;
mod l10n;
mod load;

pub use check::{check_catalog, check_dir, CheckIssue};
pub use l10n::po::{self, PoEntry, PoError};
pub use l10n::{
    add_languages, language, languages, match_locale, resolve, Catalog, Language, Translator, AUTO,
    ENGLISH, LANGUAGES, POSITION_CUE,
};
pub use load::{
    added_languages_in, catalog_text, discover_languages, load_translator, offered_codes,
    override_folders, system_locale, with_system_locale, SharedTranslator, LANG_ENV_SUFFIX,
    MISSES_ENV_SUFFIX,
};
