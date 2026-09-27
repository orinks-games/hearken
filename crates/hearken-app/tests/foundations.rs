use hearken_app::settings::UNREADABLE_NOTICE;
use hearken_app::{InputEvent, Key, Mods, Paths, Settings};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
struct GameBits {
    difficulty: u8,
}

fn temp_paths() -> (Paths, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Paths::at(dir.path()), dir)
}

#[test]
fn typed_characters_land_on_their_keys() {
    assert_eq!(Key::from_char('Q'), Key::Q);
    assert_eq!(Key::from_char('5'), Key::Num5);
    assert!(matches!(Key::from_char('$'), Key::Other(_)));
    assert_eq!(
        InputEvent::typed('a'),
        InputEvent::KeyDown {
            key: Key::A,
            mods: Mods::NONE,
            text: Some('a'),
            repeat: false
        }
    );
    assert!(!InputEvent::key(Key::Down).is_repeat());
}

#[test]
fn a_controller_button_stands_in_for_the_key_a_screen_reads() {
    use hearken_app::ControllerButton as B;
    assert_eq!(InputEvent::key(Key::Z).pressed(), Some(Key::Z));
    assert_eq!(InputEvent::button(B::A).pressed(), Some(Key::Return));
    assert_eq!(InputEvent::button(B::B).pressed(), Some(Key::Escape));
    assert_eq!(InputEvent::button(B::Start).pressed(), Some(Key::Escape));
    assert_eq!(InputEvent::button(B::Back).pressed(), Some(Key::F1));
    assert_eq!(InputEvent::button(B::DPadLeft).pressed(), Some(Key::Left));
    assert_eq!(InputEvent::button(B::X).pressed(), None);
    assert_eq!(InputEvent::key_up(Key::Z).pressed(), None);
    assert_eq!(InputEvent::button_up(B::A).pressed(), None);
    let held = InputEvent::ControllerButtonDown {
        button: B::DPadRight,
        instance_id: 0,
        repeat: true,
    };
    assert!(held.is_repeat());
    assert!(!InputEvent::button(B::DPadRight).is_repeat());
}

#[test]
fn spoken_text_helpers() {
    assert_eq!(hearken_app::text::end_sentence("Sleep"), "Sleep.");
    assert_eq!(hearken_app::text::end_sentence("Done."), "Done.");
    assert_eq!(hearken_app::text::spoken_char(' '), "space");
    assert_eq!(hearken_app::text::spoken_char('J'), "cap j");
}

#[test]
fn the_data_dir_override_wins_and_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let wanted = dir.path().join("elsewhere");
    std::env::set_var("HEARKEN_TEST_FOUNDATIONS_DATA_DIR", &wanted);
    let paths = Paths::discover("HearkenFoundationsTest", "HEARKEN_TEST_FOUNDATIONS");
    std::env::remove_var("HEARKEN_TEST_FOUNDATIONS_DATA_DIR");
    assert_eq!(paths.data_dir, wanted);
    assert!(wanted.is_dir());
    assert_eq!(paths.settings_file(), wanted.join("settings.json"));
    assert_eq!(paths.log_file(), wanted.join("session.log"));
}

#[test]
fn missing_settings_load_as_defaults_without_a_notice() {
    let (paths, _dir) = temp_paths();
    let (s, notice) = Settings::<GameBits>::load(&paths);
    assert_eq!(s, Settings::default());
    assert_eq!(notice, None);
}

#[test]
fn settings_round_trip() {
    let (paths, _dir) = temp_paths();
    let mut s = Settings::<GameBits>::default();
    s.speech.rate = 0.8;
    s.game.difficulty = 3;
    s.save(&paths).unwrap();
    assert_eq!(Settings::<GameBits>::load(&paths), (s, None));
}

#[test]
fn partial_and_unknown_fields_load() {
    let (paths, _dir) = temp_paths();
    std::fs::write(
        paths.settings_file(),
        r#"{"speech": {"rate": 0.3}, "extra": 1}"#,
    )
    .unwrap();
    let (s, notice) = Settings::<GameBits>::load(&paths);
    assert_eq!(s.speech.rate, 0.3);
    assert_eq!(s.speech.pitch, 0.5);
    assert_eq!(s.game, GameBits::default());
    assert!(s.menus.announce_position, "a file from before the setting");
    assert_eq!(notice, None);
}

#[test]
fn an_unreadable_settings_file_is_kept_aside_and_reported() {
    let (paths, _dir) = temp_paths();
    std::fs::write(paths.settings_file(), "{ not json").unwrap();
    let (s, notice) = Settings::<GameBits>::load(&paths);
    assert_eq!(s, Settings::default());
    assert_eq!(notice.as_deref(), Some(UNREADABLE_NOTICE));
    assert!(
        !paths.settings_file().exists(),
        "the bad file is moved, not left to be overwritten"
    );
    let bad = paths.data_dir.join("settings.json.bad");
    assert_eq!(std::fs::read_to_string(bad).unwrap(), "{ not json");
}

#[test]
fn a_settings_file_cut_off_mid_character_is_kept_aside_and_reported() {
    let (paths, _dir) = temp_paths();
    std::fs::write(paths.settings_file(), b"{\"speech\": {\"voice\": \"caf\xC3").unwrap();
    let (s, notice) = Settings::<GameBits>::load(&paths);
    assert_eq!(s, Settings::default());
    assert_eq!(notice.as_deref(), Some(UNREADABLE_NOTICE));
    assert!(
        !paths.settings_file().exists(),
        "the bad file is moved, not left to be overwritten"
    );
    let bad = paths.data_dir.join("settings.json.bad");
    assert_eq!(
        std::fs::read(bad).unwrap(),
        b"{\"speech\": {\"voice\": \"caf\xC3"
    );
}
