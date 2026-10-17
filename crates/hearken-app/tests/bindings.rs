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

#[test]
fn activating_a_row_and_a_landed_rebind_each_play_select_once() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.clear();
    h.press(Key::Return); // Enter on Jump: starts listening
    assert_eq!(h.played(), ["ui/menu_select"]);
    h.clear();
    h.press(Key::Z); // the new key lands: a separate moment
    assert_eq!(h.played(), ["ui/menu_select"]);
}

#[test]
fn a_taken_key_plays_the_error_sound() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::Return);
    h.clear();
    h.press(Key::C); // already Duck's
    assert_eq!(h.played(), ["ui/error"]);
}

#[test]
fn reset_all_plays_its_own_select_sound_once() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::End); // the Reset row
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.played(),
        ["ui/menu_select"],
        "the row is silenced so only reset_all's own sound plays"
    );
}

#[test]
fn escape_while_not_listening_plays_the_back_sound() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.played(), ["ui/menu_back"]);
}

/// A game that words the engine's keys its own way and lets the pad's Start
/// and Back be chosen on the second layer: Freight Fate's rules, cut down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Own {
    Horn,
    Lights,
}

impl hearken_app::bindings::Action for Own {
    fn all() -> &'static [Self] {
        &[Own::Horn, Own::Lights]
    }
    fn id(self) -> &'static str {
        match self {
            Own::Horn => "horn",
            Own::Lights => "lights",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Own::Horn => "Horn",
            Own::Lights => "Lights",
        }
    }
    fn default_keys(self) -> &'static [Chord] {
        match self {
            Own::Horn => const { &[Chord::plain(Key::H)] },
            Own::Lights => const { &[Chord::plain(Key::L), Chord::plain(Key::Slash)] },
        }
    }
    fn default_pad(self) -> &'static [PadChord] {
        match self {
            Own::Horn => const { &[PadChord::plain(B::Y)] },
            Own::Lights => const { &[PadChord::plain(B::RightStickRight)] },
        }
    }
    fn reserved_key(chord: &Chord) -> Option<&'static str> {
        match chord.key {
            Key::Escape if chord.mods == Mods::NONE => Some("Escape is the pause menu"),
            Key::F2 => Some("F2 lists the driving commands"),
            Key::Return | Key::KpEnter if chord.mods == Mods::NONE => Some("Enter confirms"),
            _ => None,
        }
    }
    fn reserved_pad(chord: &PadChord) -> Option<&'static str> {
        match chord.button {
            B::Start if !chord.modified => Some("Start is pause"),
            B::Back if !chord.modified => Some("Back stops the driving voice and reads help"),
            _ => None,
        }
    }
}

/// The same rules, with the engine's list still on behind them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Alone(Own);

impl hearken_app::bindings::Action for Alone {
    fn all() -> &'static [Self] {
        &[Alone(Own::Horn), Alone(Own::Lights)]
    }
    fn id(self) -> &'static str {
        self.0.id()
    }
    fn label(self) -> &'static str {
        self.0.label()
    }
    fn default_keys(self) -> &'static [Chord] {
        self.0.default_keys()
    }
    fn default_pad(self) -> &'static [PadChord] {
        self.0.default_pad()
    }
    fn reserved_key(chord: &Chord) -> Option<&'static str> {
        Own::reserved_key(chord)
    }
    fn reserved_pad(chord: &PadChord) -> Option<&'static str> {
        Own::reserved_pad(chord)
    }
    fn keeps_engine_reserved() -> bool {
        false
    }
}

#[test]
fn the_games_reason_wins_over_the_engines() {
    let mut b = KeyBindings::<Own>::default();
    for (chord, reason) in [
        (Chord::plain(Key::Escape), "Escape is the pause menu"),
        (Chord::plain(Key::F2), "F2 lists the driving commands"),
        (Chord::plain(Key::Return), "Enter confirms"),
        (Chord::plain(Key::KpEnter), "Enter confirms"),
        // Not the game's: the engine still answers.
        (Chord::shift(Key::Escape), "Escape goes back"),
        (Chord::plain(Key::F1), "F1 is help"),
    ] {
        assert_eq!(b.set_chord(Own::Horn, chord), Rebind::Reserved(reason));
    }
    assert_eq!(
        b.set_pad_chord(Own::Horn, PadChord::plain(B::Start)),
        Rebind::Reserved("Start is pause")
    );
    assert_eq!(
        b.set_pad_chord(Own::Horn, PadChord::plain(B::Back)),
        Rebind::Reserved("Back stops the driving voice and reads help")
    );
    assert_eq!(
        b.set_pad_chord(Own::Horn, PadChord::modified(B::Start)),
        Rebind::Reserved("Start pauses and goes back"),
        "the engine keeps Start on both layers"
    );
}

#[test]
fn a_game_can_turn_the_engines_reserved_list_off() {
    let mut b = KeyBindings::<Alone>::default();
    let horn = Alone(Own::Horn);
    assert_eq!(
        b.set_chord(horn, Chord::plain(Key::Escape)),
        Rebind::Reserved("Escape is the pause menu")
    );
    assert_eq!(b.set_chord(horn, Chord::plain(Key::F1)), Rebind::Done);
    assert_eq!(
        b.set_pad_chord(horn, PadChord::plain(B::Start)),
        Rebind::Reserved("Start is pause")
    );
    assert_eq!(
        b.set_pad_chord(horn, PadChord::modified(B::Start)),
        Rebind::Done
    );
    assert_eq!(
        b.set_pad_chord(horn, PadChord::modified(B::Back)),
        Rebind::Done
    );
    assert_eq!(b.pad_action_for(B::Back, true), Some(horn));
}

#[test]
fn a_saved_binding_on_a_key_only_the_game_keeps_is_skipped() {
    let mut saved = SavedBindings::default();
    saved.keys.insert("horn".to_string(), "enter".to_string());
    saved
        .keys
        .insert("lights".to_string(), "ctrl+enter".to_string());
    let loaded = KeyBindings::<Own>::from_saved(&saved);
    assert_eq!(loaded.chords(Own::Horn), [Chord::plain(Key::H)]);
    assert_eq!(
        loaded.action_for(Key::Return, Mods::CTRL),
        Some(Own::Lights)
    );
}

#[test]
fn a_saved_binding_keeps_its_key_when_an_update_adds_that_default() {
    // Horn moved to slash on a build before Lights took slash by default:
    // the horn keeps it, and lights keep only their other key.
    let mut saved = SavedBindings::default();
    saved.keys.insert("horn".to_string(), "slash".to_string());
    let b = KeyBindings::<Own>::from_saved(&saved);
    assert_eq!(b.action_for(Key::Slash, Mods::NONE), Some(Own::Horn));
    assert_eq!(b.chords(Own::Lights), [Chord::plain(Key::L)]);
    assert_eq!(b.spoken(Own::Lights), "L");
}

#[test]
fn a_saved_pad_binding_keeps_a_new_default() {
    let mut saved = SavedBindings::default();
    saved
        .pad
        .insert("horn".to_string(), "rstick_right".to_string());
    let b = KeyBindings::<Own>::from_saved(&saved);
    assert_eq!(b.pad_action_for(B::RightStickRight, false), Some(Own::Horn));
    assert!(b.pad_chords(Own::Lights).is_empty());
    assert_eq!(b.pad_spoken(Own::Lights), "Lights");
}

#[test]
fn the_right_stick_directions_have_names() {
    for (button, saved, short) in [
        (B::RightStickUp, "rstick_up", "right stick up"),
        (B::RightStickDown, "rstick_down", "right stick down"),
        (B::RightStickLeft, "rstick_left", "right stick left"),
        (B::RightStickRight, "rstick_right", "right stick right"),
    ] {
        let chord = PadChord::plain(button);
        assert_eq!(chord.saved(), saved);
        assert_eq!(PadChord::parse(saved), Some(chord));
        assert_eq!(chord.spoken(), short);
        assert_eq!(
            PadChord::modified(button).spoken(),
            format!("right bumper plus {short}")
        );
    }
}

#[test]
fn reset_puts_keys_and_pad_back_together() {
    let mut b = KeyBindings::<T>::default();
    b.set_chord(T::Jump, Chord::plain(Key::Z));
    b.set_pad_chord(T::Horn, PadChord::plain(B::Y));
    b.reset();
    assert!(b.is_default());
    assert_eq!(b.action_for(Key::Space, Mods::NONE), Some(T::Jump));
}

#[test]
fn a_table_can_be_built_from_moves_and_read_back_in_order() {
    let b = KeyBindings::<T>::from_parts(
        [
            (T::Map, Chord::plain(Key::Escape)),
            (T::Jump, Chord::alt(Key::J)),
            // Horn is not on the keyboard: dropped.
            (T::Horn, Chord::plain(Key::H)),
        ],
        [
            (T::Horn, PadChord::plain(B::Start)),
            // Map is not on the pad: dropped.
            (T::Map, PadChord::plain(B::Y)),
        ],
    );
    assert_eq!(
        b.moved_keys().collect::<Vec<_>>(),
        [
            (T::Jump, Chord::alt(Key::J)),
            (T::Map, Chord::plain(Key::Escape)),
        ],
        "taken as given, reserved or not, in the order of Action::all"
    );
    assert_eq!(
        b.moved_pad().collect::<Vec<_>>(),
        [(T::Horn, PadChord::plain(B::Start))]
    );
    assert_eq!(b.key_override(T::Jump), Some(Chord::alt(Key::J)));
    assert_eq!(b.key_override(T::Duck), None);
    assert_eq!(b.pad_override(T::Horn), Some(PadChord::plain(B::Start)));
    assert_eq!(b.pad_override(T::Jump), None);
}
