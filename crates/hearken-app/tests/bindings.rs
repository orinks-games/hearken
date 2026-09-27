mod common;

use common::{Probe, TestAction as T, TestData, TestGame};
use hearken_app::bindings::{
    key_saved_name, parse_key_name, Chord, KeyBindings, PadChord, Rebind, SavedBindings,
};
use hearken_app::screens::{BindingDevice, BindingsScreen};
use hearken_app::{
    ControllerButton as B, Ctx, Headless, HeldKeys, InputEvent, Key, Mods, Paths, Settings,
};
use hearken_speech::CaptureSpeech;

#[test]
fn named_keys_read_back_from_their_saved_names() {
    for key in [
        Key::A,
        Key::Num0,
        Key::Kp5,
        Key::F12,
        Key::Up,
        Key::Quote,
        Key::RAlt,
    ] {
        assert_eq!(parse_key_name(key_saved_name(key).unwrap()), Some(key));
    }
    assert_eq!(key_saved_name(Key::Other(7)), None);
}

#[test]
fn chords_are_spoken_saved_and_parsed() {
    let chord = Chord {
        key: Key::J,
        mods: Mods {
            shift: false,
            ctrl: true,
            alt: true,
        },
    };
    assert_eq!(chord.spoken(), "Control Alt J");
    assert_eq!(chord.saved().as_deref(), Some("ctrl+alt+j"));
    assert_eq!(Chord::parse("ctrl+alt+j"), Some(chord));
    assert_eq!(Chord::plain(Key::Up).spoken(), "the Up arrow");
    assert_eq!(Chord::parse("alt+nonsense"), None);
    assert_eq!(PadChord::modified(B::Y).spoken(), "right bumper plus Y");
    assert_eq!(PadChord::plain(B::A).spoken(), "the A button");
    assert_eq!(
        PadChord::parse("mod+dpad_up"),
        Some(PadChord::modified(B::DPadUp))
    );
}

#[test]
fn moving_a_key_frees_the_old_one_and_refuses_a_taken_one() {
    let mut b = KeyBindings::<T>::default();
    assert_eq!(
        b.set_chord(T::Jump, Chord::plain(Key::C)),
        Rebind::Taken(T::Duck)
    );
    assert_eq!(b.set_chord(T::Jump, Chord::plain(Key::Z)), Rebind::Done);
    assert_eq!(b.action_for(Key::Z, Mods::NONE), Some(T::Jump));
    assert_eq!(b.action_for(Key::Space, Mods::NONE), None);
    assert_eq!(
        b.set_chord(T::Jump, Chord::plain(Key::Z)),
        Rebind::Unchanged
    );
    // Back to the default clears the change rather than recording it.
    assert_eq!(b.set_chord(T::Jump, Chord::plain(Key::Space)), Rebind::Done);
    assert!(b.is_default());
}

#[test]
fn the_engines_keys_and_the_games_own_are_refused_with_a_reason() {
    let mut b = KeyBindings::<T>::default();
    assert_eq!(
        b.set_chord(T::Jump, Chord::plain(Key::Escape)),
        Rebind::Reserved("Escape goes back")
    );
    assert_eq!(
        b.set_chord(T::Jump, Chord::shift(Key::F1)),
        Rebind::Reserved("F1 is help")
    );
    assert_eq!(
        b.set_chord(T::Jump, Chord::plain(Key::F2)),
        Rebind::Reserved("F2 stops speech")
    );
    assert_eq!(
        b.set_chord(T::Jump, Chord::plain(Key::Return)),
        Rebind::Reserved("Enter chooses")
    );
    assert_eq!(
        b.set_chord(T::Jump, Chord::alt(Key::Return)),
        Rebind::Done,
        "only a bare Enter is kept"
    );
    assert_eq!(
        b.set_chord(T::Jump, Chord::plain(Key::LShift)),
        Rebind::Reserved("a modifier key on its own cannot be a shortcut")
    );
    assert_eq!(
        b.set_chord(T::Jump, Chord::plain(Key::Tab)),
        Rebind::Reserved("Tab reads the score")
    );
    assert_eq!(
        b.set_pad_chord(T::Jump, PadChord::plain(B::Start)),
        Rebind::Reserved("Start pauses and goes back")
    );
    assert_eq!(
        b.set_pad_chord(T::Jump, PadChord::modified(B::Back)),
        Rebind::Reserved("Back reads help")
    );
    assert_eq!(
        b.set_pad_chord(T::Jump, PadChord::plain(B::RightShoulder)),
        Rebind::Reserved("the right bumper is the second layer")
    );
}

#[test]
fn a_control_with_two_default_keys_keeps_both_until_moved() {
    let mut b = KeyBindings::<T>::default();
    assert_eq!(b.action_for(Key::Kp0, Mods::NONE), Some(T::Duck));
    assert_eq!(b.spoken(T::Duck), "C", "a prompt names the first");
    b.set_chord(T::Duck, Chord::plain(Key::V));
    assert_eq!(b.chords(T::Duck), [Chord::plain(Key::V)]);
    assert_eq!(b.action_for(Key::Kp0, Mods::NONE), None);
}

#[test]
fn a_press_with_modifiers_nothing_claims_falls_back_to_the_bare_key() {
    let b = KeyBindings::<T>::default();
    assert_eq!(b.action_for(Key::Space, Mods::CTRL), Some(T::Jump));
    assert_eq!(b.action_for(Key::M, Mods::SHIFT), Some(T::Map));
    assert_eq!(
        b.action_for(Key::M, Mods::NONE),
        None,
        "the map needs its Shift"
    );
}

#[test]
fn pad_buttons_move_on_their_own_layer() {
    let mut b = KeyBindings::<T>::default();
    assert_eq!(b.pad_action_for(B::Y, true), Some(T::Horn));
    assert_eq!(b.pad_action_for(B::Y, false), None);
    assert_eq!(
        b.set_pad_chord(T::Horn, PadChord::plain(B::X)),
        Rebind::Taken(T::Duck)
    );
    assert_eq!(
        b.set_pad_chord(T::Horn, PadChord::plain(B::Y)),
        Rebind::Done
    );
    assert_eq!(b.pad_action_for(B::Y, false), Some(T::Horn));
    b.reset_pad();
    assert!(b.is_default());
}

#[test]
fn saved_bindings_round_trip_and_a_bad_entry_costs_only_itself() {
    let mut b = KeyBindings::<T>::default();
    b.set_chord(T::Jump, Chord::alt(Key::J));
    b.set_pad_chord(T::Horn, PadChord::plain(B::LeftStick));
    let mut saved = SavedBindings::default();
    b.store(&mut saved);
    assert_eq!(saved.keys.get("jump").map(String::as_str), Some("alt+j"));
    assert_eq!(
        saved.pad.get("horn").map(String::as_str),
        Some("left_stick")
    );
    assert_eq!(KeyBindings::<T>::from_saved(&saved), b);

    // A hand-edited file: a key the game cannot name, an action it no longer
    // has, and a key for a control the keyboard does not have.
    saved
        .keys
        .insert("duck".to_string(), "not_a_key".to_string());
    saved.keys.insert("retired".to_string(), "q".to_string());
    saved.keys.insert("horn".to_string(), "h".to_string());
    let loaded = KeyBindings::<T>::from_saved(&saved);
    assert_eq!(loaded.action_for(Key::J, Mods::ALT), Some(T::Jump));
    assert_eq!(
        loaded.action_for(Key::C, Mods::NONE),
        Some(T::Duck),
        "the bad line fell back to the default"
    );
    assert_eq!(loaded.action_for(Key::H, Mods::NONE), None);
    assert_eq!(loaded.action_for(Key::Q, Mods::NONE), None);
}

#[test]
fn a_saved_binding_on_a_kept_key_falls_back_to_the_default() {
    let mut saved = SavedBindings::default();
    saved.keys.insert("jump".to_string(), "escape".to_string());
    saved.keys.insert("duck".to_string(), "tab".to_string());
    saved.pad.insert("jump".to_string(), "start".to_string());
    let loaded = KeyBindings::<T>::from_saved(&saved);
    assert_eq!(loaded.action_for(Key::Escape, Mods::NONE), None);
    assert_eq!(loaded.action_for(Key::Space, Mods::NONE), Some(T::Jump));
    assert_eq!(loaded.action_for(Key::C, Mods::NONE), Some(T::Duck));
    assert_eq!(loaded.pad_action_for(B::A, false), Some(T::Jump));
    assert!(loaded.is_default());
}

#[test]
fn a_screen_asks_the_context_what_a_press_means_and_whether_a_control_is_held() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.held = HeldKeys::with_timing(500, 33);
    {
        let ctx = &h.app.ctx;
        assert_eq!(ctx.action_for(&InputEvent::key(Key::Space)), Some(T::Jump));
        assert_eq!(ctx.action_for(&InputEvent::button(B::A)), Some(T::Jump));
        assert_eq!(ctx.action_for(&InputEvent::key_up(Key::Space)), None);
        assert_eq!(ctx.control_name(T::Jump), "Space");
    }
    h.connect_pad();
    h.send(InputEvent::button(B::A));
    assert!(h.app.ctx.action_held(T::Jump));
    assert_eq!(
        h.app.ctx.control_name(T::Jump),
        "the A button",
        "the pad was used last"
    );
    h.send(InputEvent::button_up(B::A));
    assert!(!h.app.ctx.action_held(T::Jump));
    h.send(InputEvent::key_mods(Key::M, Mods::SHIFT));
    assert!(h.app.ctx.action_held(T::Map));
    assert_eq!(
        h.app.ctx.control_name(T::Jump),
        "Space",
        "the keyboard was used last"
    );
}

#[test]
fn bindings_come_back_from_the_settings_file() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path());
    let mut settings = Settings::<()>::default();
    settings
        .bindings
        .keys
        .insert("jump".to_string(), "z".to_string());
    settings.save(&paths).unwrap();
    let (loaded, notice) = Settings::<()>::load(&paths);
    assert!(notice.is_none());
    let ctx: Ctx<TestGame> = Ctx::new(
        Box::new(CaptureSpeech::new()),
        loaded,
        paths,
        TestData::default(),
    );
    assert_eq!(ctx.action_for(&InputEvent::key(Key::Z)), Some(T::Jump));
}

fn shortcuts(device: BindingDevice) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.connect_pad();
    h.app.ctx.push(BindingsScreen::<TestGame>::new(device));
    h.frame(0.0);
    h
}

#[test]
fn the_keyboard_screen_lists_each_control_with_its_key() {
    let h = shortcuts(BindingDevice::Keyboard);
    assert_eq!(h.spoken(), ["Keyboard shortcuts.", "Jump: Space. 1 of 4."]);
    assert_eq!(
        h.app.top_lines(),
        [
            "Keyboard shortcuts",
            "> Jump: Space",
            "  Duck: C",
            "  Open the map: Shift M",
            "  Reset every keyboard shortcut"
        ]
    );
}

#[test]
fn enter_then_a_key_moves_the_control_and_saves_it_at_once() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::Return);
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Press the new key for Jump. Escape keeps Space.")
    );
    h.clear();
    h.press(Key::Z);
    assert_eq!(h.spoken(), ["Jump is now Z."]);
    let saved = std::fs::read_to_string(h.app.ctx.paths.settings_file()).unwrap();
    assert!(saved.contains("\"jump\": \"z\""), "saved: {saved}");
    // Back to moving between rows.
    h.clear();
    h.press(Key::Down);
    assert_eq!(h.spoken(), ["Duck: C. 2 of 4."]);
}

#[test]
fn a_taken_or_reserved_key_is_refused_and_the_screen_keeps_listening() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::Return);
    h.clear();
    h.press(Key::C);
    h.press(Key::Tab);
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        [
            "C is already used for Duck. Press another, or keep Space with Escape.",
            "Tab reads the score. Press another, or keep Space with Escape.",
            "Enter chooses. Press another, or keep Space with Escape.",
        ]
    );
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), ["Kept Space."]);
    assert_eq!(
        h.app.top_name(),
        "bindings",
        "Escape ended the listening, not the screen"
    );
}

#[test]
fn a_modifier_waits_for_the_key_that_goes_with_it() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::Return);
    h.clear();
    h.send(InputEvent::key_mods(Key::LAlt, Mods::ALT));
    assert!(h.spoken().is_empty());
    h.send(InputEvent::key_mods(Key::J, Mods::ALT));
    assert_eq!(h.spoken(), ["Jump is now Alt J."]);
}

#[test]
fn f1_while_listening_repeats_what_the_screen_is_waiting_for() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::Return);
    h.clear();
    h.press(Key::F1);
    assert_eq!(
        h.spoken(),
        ["Press the new key for Jump. Escape keeps Space."]
    );
}

#[test]
fn reset_puts_every_key_back_and_saves() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::Return);
    h.press(Key::Z);
    h.press(Key::End);
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        ["Every keyboard shortcut is back to its default."]
    );
    assert!(h.app.ctx.bindings.is_default());
    let saved = std::fs::read_to_string(h.app.ctx.paths.settings_file()).unwrap();
    assert!(!saved.contains("\"jump\""), "saved: {saved}");
}

#[test]
fn the_controller_screen_takes_a_button_and_the_right_bumper_picks_the_second_layer() {
    let mut h = shortcuts(BindingDevice::Controller);
    assert_eq!(
        h.spoken(),
        ["Controller buttons.", "Jump: the A button. 1 of 4."]
    );
    h.send(InputEvent::button(B::A));
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some(
            "Press the new button for Jump, with the right bumper held for the second layer. \
             Press the A button to keep it."
        )
    );
    h.clear();
    h.press(Key::Z);
    assert!(h.spoken().is_empty(), "a key is not a button");
    h.send(InputEvent::button(B::RightShoulder));
    h.send(InputEvent::button(B::LeftStick));
    assert_eq!(
        h.spoken(),
        ["Jump is now right bumper plus the left stick click."]
    );
    assert_eq!(
        h.app.ctx.bindings.pad_action_for(B::LeftStick, true),
        Some(T::Jump)
    );
}

#[test]
fn pressing_the_button_a_control_already_has_keeps_it() {
    let mut h = shortcuts(BindingDevice::Controller);
    h.send(InputEvent::button(B::A));
    h.send(InputEvent::button_up(B::A));
    h.clear();
    h.send(InputEvent::button(B::A));
    assert_eq!(h.spoken(), ["Kept the A button."]);
}

#[test]
fn escape_on_the_controller_screen_keeps_the_button() {
    let mut h = shortcuts(BindingDevice::Controller);
    h.send(InputEvent::button(B::A));
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), ["Kept the A button."]);
}
