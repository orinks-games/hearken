//! Drop-in lang files translate what Ctx::say speaks.
#![cfg(feature = "testing")]

use hearken_app::testing::Headless;
use hearken_app::{Ctx, Game, NoAction, NoCategory, State};

struct Demo;
struct Title;

impl Game for Demo {
    const TITLE: &'static str = "Demo";
    const DATA_DIR: &'static str = "hearken-l10n-demo";
    const ENV_PREFIX: &'static str = "HEARKEN_L10N_DEMO";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;
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
fn drop_in_catalog_translates_say() {
    let dir = tempfile::tempdir().expect("tmp");
    let lang = dir.path().join("lang");
    std::fs::create_dir_all(&lang).expect("lang");
    std::fs::write(
        lang.join("es.po"),
        "msgid \"\"\nmsgstr \"X-Language-Name: Spanish\\nX-Language-Autonym: Español\\n\"\n\nmsgid \"New screen.\"\nmsgstr \"Nueva pantalla.\"\n",
    )
    .expect("po");

    let mut h = Headless::<Demo>::new((), Title);
    h.app.ctx.paths = hearken_app::Paths::at(dir.path());
    h.app.ctx.settings.language = "es".into();
    h.app.ctx.apply_language();
    h.app.ctx.say("New screen.");
    let spoken = h.spoken();
    assert!(
        spoken.iter().any(|line| line.contains("Nueva pantalla.")),
        "spoken={spoken:?}"
    );
}

#[test]
fn missing_entry_stays_english() {
    let mut h = Headless::<Demo>::new((), Title);
    h.app.ctx.say("No matching entry.");
    let spoken = h.spoken();
    assert!(
        spoken
            .iter()
            .any(|line| line.contains("No matching entry.")),
        "spoken={spoken:?}"
    );
}

#[test]
fn language_row_appears_only_when_a_catalog_exists() {
    use hearken_app::screens::SettingsScreen;
    let mut h = Headless::<Demo>::new((), SettingsScreen::<Demo>::new());
    h.frame(0.0);
    let without = h.spoken().join(" ");
    assert!(!without.contains("Language:"), "no catalog yet: {without}");

    let dir = tempfile::tempdir().expect("tmp");
    let lang = dir.path().join("lang");
    std::fs::create_dir_all(&lang).expect("lang");
    std::fs::write(
        lang.join("es.po"),
        "msgid \"\"\nmsgstr \"X-Language-Name: Spanish\\nX-Language-Autonym: Español\\n\"\n\nmsgid \"New screen.\"\nmsgstr \"Nueva pantalla.\"\n",
    )
    .expect("po");
    h.app.ctx.paths = hearken_app::Paths::at(dir.path());
    // Rebuild settings screen so build_items sees the catalog.
    h.app.ctx.pop();
    h.app.ctx.push(SettingsScreen::<Demo>::new());
    h.frame(0.0);
    let with = h.spoken().join(" ");
    assert!(with.contains("Language:"), "catalog present: {with}");
}
