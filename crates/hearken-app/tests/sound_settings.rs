mod common;

use common::*;
use hearken_app::menu::MenuItem;
use hearken_app::screens::{SettingsRow, SettingsScreen};
use hearken_app::{Bus, Ctx, Game, Headless, Key, NoAction, NoCategory, Settings};

fn settings() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(SettingsScreen::<TestGame>::new());
    h.frame(0.0);
    h.clear();
    h
}

fn row_labels<G: Game>(h: &Headless<G>) -> Vec<String> {
    h.app
        .top_lines()
        .into_iter()
        .skip(1)
        .map(|l| l[2..].to_string())
        .collect()
}

#[test]
fn the_sound_rows_come_after_speech_and_include_the_games_own_bus() {
    let h = settings();
    let rows = row_labels(&h);
    let start = rows
        .iter()
        .position(|r| r.starts_with("Speech detail"))
        .expect("a verbosity row");
    assert_eq!(
        &rows[start..start + 9],
        [
            "Speech detail: Full",
            "Lower game sound under speech: off",
            "Master volume: 100 percent",
            "Interface volume: 80 percent",
            "Effects volume: 80 percent",
            "Ambience volume: 80 percent",
            "Music volume: 80 percent",
            "Character voices volume: 80 percent",
            "Engine volume: 80 percent",
        ]
    );
}

#[test]
fn a_volume_row_moves_by_five_clamps_and_is_heard_on_its_own_bus() {
    let mut h = settings();
    h.press(Key::M); // Master volume
    h.press(Key::M); // Music volume
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.app.ctx.settings.audio.volume(Bus::MUSIC), 85);
    assert!((h.app.ctx.audio.bus_volume(Bus::MUSIC) - 0.85).abs() < 1e-4);
    assert_eq!(h.played(), ["ui/menu_move"]);
    let ch = h.sounds.channel_of("ui/menu_move").unwrap();
    assert!(
        (h.sounds.last_mix(ch).unwrap().gain - 0.85).abs() < 1e-4,
        "played through the music bus"
    );
    assert!(h.spoken()[0].starts_with("Music volume: 85 percent."));
    for _ in 0..5 {
        h.press(Key::Right);
    }
    assert_eq!(h.app.ctx.settings.audio.volume(Bus::MUSIC), 100);
}

#[test]
fn the_verbosity_row_steps_through_the_games_levels_and_wraps() {
    let mut h = settings();
    h.press(Key::S); // from Speech rate, the first row, to Speech detail
    assert!(h
        .spoken()
        .last()
        .unwrap()
        .starts_with("Speech detail: Full"));
    h.press(Key::Right);
    assert_eq!(h.app.ctx.settings.speech.verbosity, "quiet");
    h.press(Key::Left);
    h.press(Key::Left);
    assert_eq!(h.app.ctx.settings.speech.verbosity, "minimal");
}

#[test]
fn the_duck_row_toggles() {
    let mut h = settings();
    h.press(Key::L);
    h.press(Key::Return);
    assert!(h.app.ctx.settings.audio.duck_for_speech);
    assert!(h
        .spoken()
        .last()
        .unwrap()
        .starts_with("Lower game sound under speech: on"));
}

#[test]
fn a_hand_edited_file_with_odd_sound_values_loads_quietly_and_keeps_what_it_does_not_know() {
    let dir = tempfile::tempdir().unwrap();
    let paths = hearken_app::Paths::at(dir.path());
    std::fs::write(
        paths.settings_file(),
        r#"{"audio": {"volumes": {"music": 250, "gone": 30}}, "speech": {"verbosity": "gone"}}"#,
    )
    .unwrap();
    let (loaded, notice) = Settings::<()>::load(&paths);
    assert_eq!(notice, None);
    assert_eq!(loaded.audio.volume(Bus::MUSIC), 100);
    loaded.save(&paths).unwrap();
    let text = std::fs::read_to_string(paths.settings_file()).unwrap();
    assert!(
        text.contains("\"gone\": 30"),
        "a bus the game dropped stays in the file"
    );
}

/// A game whose `settings_rows` drops the built-in `menu_positions` row,
/// appends its own `difficulty` row, and moves `speech_rate` to the very
/// end (after `difficulty`) -- so it is no longer row 0, and a test can
/// tell the id-based lookup in `speak_adjusted` apart from a hard-coded
/// "row 0" check. Has no verbosity categories of its own.
struct CustomGame;

impl Game for CustomGame {
    const TITLE: &'static str = "Custom Settings Test";
    const DATA_DIR: &'static str = "HearkenCustomSettingsTest";
    const ENV_PREFIX: &'static str = "HEARKEN_CUSTOM_SETTINGS_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn settings_rows(_ctx: &Ctx<Self>, rows: Vec<SettingsRow<Self>>) -> Vec<SettingsRow<Self>> {
        let mut rows: Vec<_> = rows
            .into_iter()
            .filter(|r| r.id != "menu_positions")
            .collect();
        let rate_row = rows
            .iter()
            .position(|r| r.id == "speech_rate")
            .map(|i| rows.remove(i));
        rows.push(SettingsRow::new(
            "difficulty",
            MenuItem::inert("Difficulty: normal"),
        ));
        rows.extend(rate_row);
        rows
    }
}

fn custom_settings() -> Headless<CustomGame> {
    let mut h = Headless::new((), SettingsScreen::<CustomGame>::new());
    h.frame(0.0);
    h.clear();
    h
}

#[test]
fn a_games_settings_rows_hook_hides_and_adds_rows_and_speech_rate_is_still_found_by_id() {
    let mut h = custom_settings();
    let rows = row_labels(&h);
    assert!(!rows.iter().any(|r| r.starts_with("Menu positions")));
    assert!(rows.iter().any(|r| r.as_str() == "Difficulty: normal"));

    // The game's settings_rows moved Speech rate to the very end, after its
    // own Difficulty row, so it is no longer row 0. It must still be read
    // back through the previewing voice, found by id rather than position.
    assert_eq!(
        rows.last().map(String::as_str),
        Some("Speech rate: 50 percent")
    );
    h.press(Key::End); // Speech rate, now the last row
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.app.ctx.settings.speech.rate, 0.55);
    assert!(
        h.speech
            .borrow()
            .previews()
            .iter()
            .any(|(setting, text, _)| {
                setting.as_str() == "speech_rate" && text.starts_with("Speech rate: 55 percent")
            }),
        "the speech-rate row is still previewed by id, not by position"
    );
    assert!(
        h.spoken().is_empty(),
        "read once, by the previewing voice, not the main channel"
    );
}

#[test]
fn a_game_with_no_categories_shows_no_speech_detail_row() {
    let h = custom_settings();
    let rows = row_labels(&h);
    assert!(!rows.iter().any(|r| r.starts_with("Speech detail")));
}

/// A game whose Master volume and speech-rate rows read back value first,
/// and whose own Difficulty row does too.
struct ValueFirstGame;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Hard(bool);

impl Game for ValueFirstGame {
    const TITLE: &'static str = "Value First Test";
    const DATA_DIR: &'static str = "HearkenValueFirstTest";
    const ENV_PREFIX: &'static str = "HEARKEN_VALUE_FIRST_TEST";
    type Event = ();
    type Data = ();
    type Settings = Hard;
    type Action = NoAction;
    type Category = NoCategory;

    fn settings_rows(_ctx: &Ctx<Self>, rows: Vec<SettingsRow<Self>>) -> Vec<SettingsRow<Self>> {
        let mut rows: Vec<_> = rows
            .into_iter()
            .map(|row| match row.id.as_str() {
                "volume.master" => row.value_first(|ctx: &Ctx<Self>| {
                    format!("{} percent, master volume", ctx.volume_percent(Bus::MASTER))
                }),
                "speech_rate" => row.value_first(|ctx: &Ctx<Self>| {
                    format!("{} percent", (ctx.settings.speech.rate * 100.0).round())
                }),
                _ => row,
            })
            .collect();
        rows.push(
            SettingsRow::new(
                "difficulty",
                MenuItem::inert(hearken_app::menu::Label::dynamic(
                    |_s: &SettingsScreen<Self>, ctx: &Ctx<Self>| {
                        let hard = ctx.settings.game.0;
                        format!("Difficulty: {}", if hard { "hard" } else { "normal" })
                    },
                ))
                .adjustable(|_s, ctx: &mut Ctx<Self>, _delta| {
                    ctx.settings.game.0 = !ctx.settings.game.0;
                }),
            )
            .value_first(|ctx: &Ctx<Self>| {
                let hard = ctx.settings.game.0;
                format!("{}, difficulty", if hard { "Hard" } else { "Normal" })
            }),
        );
        rows
    }
}

#[test]
fn a_value_first_row_reads_only_its_new_value_on_left_and_right() {
    let mut h = Headless::new((), SettingsScreen::<ValueFirstGame>::new());
    h.frame(0.0);
    h.press(Key::End);
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Difficulty: normal. 11 of 11.")
    );
    h.clear();
    h.press(Key::Right);
    h.press(Key::Left);
    assert_eq!(h.spoken(), ["Hard, difficulty.", "Normal, difficulty."]);

    // Moving onto a value-first row still reads the whole row.
    h.press(Key::Home);
    h.press(Key::M);
    h.clear();
    h.press(Key::Left);
    assert_eq!(h.spoken(), ["95 percent, master volume."]);

    // Speech rate stays on the previewing voice, value first.
    h.press(Key::Home);
    h.clear();
    h.press(Key::Right);
    assert_eq!(
        h.speech.borrow().previews(),
        [("speech_rate".to_string(), "55 percent.".to_string(), true)]
    );
    assert!(h.spoken().is_empty());
}
