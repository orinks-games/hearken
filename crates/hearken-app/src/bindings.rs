//! Rebindable controls: the game's actions, the keys and pad buttons they
//! are on, and the player's changes to them, saved in `settings.json`.
//!
//! Menu keys, Escape, Enter, F1 and F2 belong to the engine and never move;
//! a game keeps more for itself through [`Action::reserved_key`] and
//! [`Action::reserved_pad`].
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `bindings.rs`, with its
//! driving actions replaced by a game-supplied [`Action`] type.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::held_keys::HeldKeys;
use crate::input::{ControllerButton, Key, Mods};

mod names;

pub use names::{key_saved_name, key_spoken_name, pad_button_short_name, parse_key_name};

/// One control a player can move to another key or button. Usually a
/// plain enum; `all` gives the order the rebinding screen lists them in.
pub trait Action: Copy + Eq + Hash + Debug + 'static {
    /// Every action, in the order the rebinding screen lists them.
    fn all() -> &'static [Self];
    /// The id the settings file stores. Never change one a release has
    /// shipped: players' saved bindings are keyed by it.
    fn id(self) -> &'static str;
    /// What the control does, as the rebinding screen reads it.
    fn label(self) -> &'static str;
    /// The keys that trigger it until the player moves it. Empty: not on
    /// the keyboard. Once moved, a control has exactly one key.
    fn default_keys(self) -> &'static [Chord];
    /// The pad buttons that trigger it until the player moves it. Empty:
    /// not on the controller.
    fn default_pad(self) -> &'static [PadChord] {
        &[]
    }
    /// A key the game keeps for itself, with the reason the rebinding
    /// screen gives, after the engine's own.
    fn reserved_key(_chord: &Chord) -> Option<&'static str> {
        None
    }
    /// A pad button the game keeps for itself, likewise.
    fn reserved_pad(_chord: &PadChord) -> Option<&'static str> {
        None
    }
}

/// The actions of a game with nothing to rebind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoAction {}

impl Action for NoAction {
    fn all() -> &'static [Self] {
        &[]
    }
    fn id(self) -> &'static str {
        match self {}
    }
    fn label(self) -> &'static str {
        match self {}
    }
    fn default_keys(self) -> &'static [Chord] {
        match self {}
    }
}

/// A key with the modifiers held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

impl Chord {
    pub const fn plain(key: Key) -> Chord {
        Chord {
            key,
            mods: Mods::NONE,
        }
    }

    pub const fn shift(key: Key) -> Chord {
        Chord {
            key,
            mods: Mods::SHIFT,
        }
    }

    pub const fn alt(key: Key) -> Chord {
        Chord {
            key,
            mods: Mods::ALT,
        }
    }

    /// The chord as a player hears it: "Alt J", "the Up arrow", "T".
    pub fn spoken(&self) -> String {
        let mut out = String::new();
        if self.mods.ctrl {
            out.push_str("Control ");
        }
        if self.mods.alt {
            out.push_str("Alt ");
        }
        if self.mods.shift {
            out.push_str("Shift ");
        }
        out.push_str(&key_spoken_name(self.key));
        out
    }

    /// The chord as the settings file stores it: "alt+j", "up", "shift+k".
    pub fn saved(&self) -> Option<String> {
        let key = key_saved_name(self.key)?;
        let mut out = String::new();
        if self.mods.ctrl {
            out.push_str("ctrl+");
        }
        if self.mods.alt {
            out.push_str("alt+");
        }
        if self.mods.shift {
            out.push_str("shift+");
        }
        out.push_str(key);
        Some(out)
    }

    pub fn parse(text: &str) -> Option<Chord> {
        let mut mods = Mods::NONE;
        let mut key = None;
        for part in text.split('+') {
            match part {
                "ctrl" => mods.ctrl = true,
                "alt" => mods.alt = true,
                "shift" => mods.shift = true,
                name => key = Some(parse_key_name(name)?),
            }
        }
        key.map(|key| Chord { key, mods })
    }
}

/// A pad button, on the plain layer or with the right bumper held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PadChord {
    pub button: ControllerButton,
    pub modified: bool,
}

impl PadChord {
    pub const fn plain(button: ControllerButton) -> PadChord {
        PadChord {
            button,
            modified: false,
        }
    }

    pub const fn modified(button: ControllerButton) -> PadChord {
        PadChord {
            button,
            modified: true,
        }
    }

    /// As a prompt names it: "the A button", "D-pad down", "right bumper
    /// plus Y".
    pub fn spoken(&self) -> String {
        let short = pad_button_short_name(self.button);
        if self.modified {
            format!("right bumper plus {short}")
        } else if matches!(
            self.button,
            ControllerButton::A | ControllerButton::B | ControllerButton::X | ControllerButton::Y
        ) {
            format!("the {short} button")
        } else {
            short.to_string()
        }
    }

    pub fn saved(&self) -> String {
        let name = names::pad_button_saved_name(self.button);
        if self.modified {
            format!("mod+{name}")
        } else {
            name.to_string()
        }
    }

    pub fn parse(text: &str) -> Option<PadChord> {
        let (modified, name) = match text.strip_prefix("mod+") {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        names::parse_pad_button_name(name).map(|button| PadChord { button, modified })
    }
}

/// Why the engine will not let a key be chosen, as the rebinding screen
/// says it.
pub fn reserved_key_reason(chord: &Chord) -> Option<&'static str> {
    match chord.key {
        Key::Escape => Some("Escape goes back"),
        Key::F1 => Some("F1 is help"),
        Key::F2 => Some("F2 stops speech"),
        Key::LCtrl | Key::RCtrl | Key::LShift | Key::RShift | Key::LAlt | Key::RAlt => {
            Some("a modifier key on its own cannot be a shortcut")
        }
        Key::Other(_) => Some("that key cannot be used"),
        // Menus choose on a bare Enter; with a modifier it is free.
        Key::Return | Key::KpEnter if chord.mods == Mods::NONE => Some("Enter chooses"),
        _ => None,
    }
}

/// Why the engine will not let a pad button be chosen, as the rebinding
/// screen says it.
pub fn reserved_pad_reason(chord: &PadChord) -> Option<&'static str> {
    match chord.button {
        ControllerButton::Guide => Some("the guide button belongs to the system"),
        ControllerButton::Back => Some("Back reads help"),
        ControllerButton::Start => Some("Start pauses and goes back"),
        ControllerButton::RightShoulder => Some("the right bumper is the second layer"),
        _ => None,
    }
}

/// What asking for a new binding did.
#[derive(Debug, PartialEq, Eq)]
pub enum Rebind<A> {
    Done,
    /// The control already has that key or button.
    Unchanged,
    Reserved(&'static str),
    /// Another control has it. Refused, never swapped.
    Taken(A),
}

/// The player's changes as `settings.json` keeps them: action id to chord
/// (`"jump": "alt+j"`), keys and pad buttons apart. Only moved controls
/// appear.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct SavedBindings {
    pub keys: BTreeMap<String, String>,
    pub pad: BTreeMap<String, String>,
}

/// The live table: the defaults plus what the player moved.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyBindings<A: Action> {
    keys: HashMap<A, Chord>,
    pad: HashMap<A, PadChord>,
}

impl<A: Action> Default for KeyBindings<A> {
    fn default() -> Self {
        KeyBindings {
            keys: HashMap::new(),
            pad: HashMap::new(),
        }
    }
}

impl<A: Action> KeyBindings<A> {
    /// The table `saved` describes. An entry that does not read (an unknown
    /// id or key name, a key for a control the keyboard does not have, or a
    /// key the engine or the game keeps for itself) is skipped, so one bad
    /// line never costs the rest.
    pub fn from_saved(saved: &SavedBindings) -> Self {
        let mut out = Self::default();
        for (id, value) in &saved.keys {
            if let (Some(action), Some(chord)) = (from_id::<A>(id), Chord::parse(value)) {
                let reserved = reserved_key_reason(&chord)
                    .or_else(|| A::reserved_key(&chord))
                    .is_some();
                if !action.default_keys().is_empty() && !reserved {
                    out.keys.insert(action, chord);
                }
            }
        }
        for (id, value) in &saved.pad {
            if let (Some(action), Some(chord)) = (from_id::<A>(id), PadChord::parse(value)) {
                let reserved = reserved_pad_reason(&chord)
                    .or_else(|| A::reserved_pad(&chord))
                    .is_some();
                if !action.default_pad().is_empty() && !reserved {
                    out.pad.insert(action, chord);
                }
            }
        }
        out
    }

    /// Write the moved controls into `saved`, replacing what it held.
    pub fn store(&self, saved: &mut SavedBindings) {
        saved.keys = A::all()
            .iter()
            .filter_map(|action| Some((action.id().to_string(), self.keys.get(action)?.saved()?)))
            .collect();
        saved.pad = A::all()
            .iter()
            .filter_map(|action| Some((action.id().to_string(), self.pad.get(action)?.saved())))
            .collect();
    }

    /// True when nothing has been moved from its default.
    pub fn is_default(&self) -> bool {
        self.keys.is_empty() && self.pad.is_empty()
    }

    pub fn reset_keys(&mut self) {
        self.keys.clear();
    }

    pub fn reset_pad(&mut self) {
        self.pad.clear();
    }

    // -- keyboard ----------------------------------------------------------------

    /// The chords that trigger `action` today.
    pub fn chords(&self, action: A) -> Vec<Chord> {
        match self.keys.get(&action) {
            Some(chord) => vec![*chord],
            None => action.default_keys().to_vec(),
        }
    }

    /// The control's key for a spoken prompt, first chord only.
    pub fn spoken(&self, action: A) -> String {
        self.chords(action)
            .first()
            .map(Chord::spoken)
            .unwrap_or_else(|| action.label().to_string())
    }

    /// The action a key press means: the exact chord if one is bound, else
    /// the bare key's action.
    pub fn action_for(&self, key: Key, mods: Mods) -> Option<A> {
        self.exact_action(&Chord { key, mods }).or_else(|| {
            if mods == Mods::NONE {
                None
            } else {
                self.exact_action(&Chord::plain(key))
            }
        })
    }

    fn exact_action(&self, chord: &Chord) -> Option<A> {
        A::all()
            .iter()
            .copied()
            .find(|action| self.chords(*action).contains(chord))
    }

    /// Whether `action`'s key is down right now, its modifiers included.
    pub fn pressed(&self, held: &HeldKeys, action: A) -> bool {
        let mods = held.mods();
        self.chords(action).iter().any(|chord| {
            held.is_pressed(chord.key)
                && (!chord.mods.shift || mods.shift)
                && (!chord.mods.ctrl || mods.ctrl)
                && (!chord.mods.alt || mods.alt)
        })
    }

    /// Move `action` to `chord`.
    pub fn set_chord(&mut self, action: A, chord: Chord) -> Rebind<A> {
        if let Some(reason) = reserved_key_reason(&chord).or_else(|| A::reserved_key(&chord)) {
            return Rebind::Reserved(reason);
        }
        if self.chords(action).contains(&chord) {
            return Rebind::Unchanged;
        }
        if let Some(other) = self.exact_action(&chord) {
            return Rebind::Taken(other);
        }
        if action.default_keys() == [chord] {
            self.keys.remove(&action);
        } else {
            self.keys.insert(action, chord);
        }
        Rebind::Done
    }

    // -- pad ---------------------------------------------------------------------

    pub fn pad_chords(&self, action: A) -> Vec<PadChord> {
        match self.pad.get(&action) {
            Some(chord) => vec![*chord],
            None => action.default_pad().to_vec(),
        }
    }

    pub fn pad_spoken(&self, action: A) -> String {
        self.pad_chords(action)
            .first()
            .map(PadChord::spoken)
            .unwrap_or_else(|| action.label().to_string())
    }

    pub fn pad_action_for(&self, button: ControllerButton, modified: bool) -> Option<A> {
        let chord = PadChord { button, modified };
        A::all()
            .iter()
            .copied()
            .find(|action| self.pad_chords(*action).contains(&chord))
    }

    pub fn set_pad_chord(&mut self, action: A, chord: PadChord) -> Rebind<A> {
        if let Some(reason) = reserved_pad_reason(&chord).or_else(|| A::reserved_pad(&chord)) {
            return Rebind::Reserved(reason);
        }
        if self.pad_chords(action).contains(&chord) {
            return Rebind::Unchanged;
        }
        if let Some(other) = self.pad_action_for(chord.button, chord.modified) {
            return Rebind::Taken(other);
        }
        if action.default_pad() == [chord] {
            self.pad.remove(&action);
        } else {
            self.pad.insert(action, chord);
        }
        Rebind::Done
    }
}

fn from_id<A: Action>(id: &str) -> Option<A> {
    A::all().iter().copied().find(|action| action.id() == id)
}
