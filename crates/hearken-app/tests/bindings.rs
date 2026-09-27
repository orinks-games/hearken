mod common;

use common::{Probe, TestAction as T, TestData, TestGame};
use hearken_app::bindings::{
    key_saved_name, parse_key_name, Chord, KeyBindings, PadChord, Rebind, SavedBindings,
};
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
