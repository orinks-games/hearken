//! Keyboard and controller input as screens see it.

/// A key on the keyboard, as one of the SDL keycodes the game reads.
/// `Other` carries any SDL keycode outside that set so text entry still sees its `text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Num0,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Kp0,
    Kp1,
    Kp2,
    Kp3,
    Kp4,
    Kp5,
    Kp6,
    Kp7,
    Kp8,
    Kp9,
    KpEnter,
    KpPlus,
    KpMinus,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Return,
    Escape,
    Space,
    Tab,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Comma,
    Period,
    Slash,
    Backslash,
    Backquote,
    Equals,
    Plus,
    Minus,
    Semicolon,
    Quote,
    LeftBracket,
    RightBracket,
    LCtrl,
    RCtrl,
    LShift,
    RShift,
    LAlt,
    RAlt,
    /// An SDL keycode the game has no binding for.
    Other(i32),
}

impl Key {
    /// The key a typed character arrives on: letters and digits map to their keys,
    /// anything else to the few punctuation keys the game binds, else `Other(codepoint)`.
    pub fn from_char(ch: char) -> Key {
        match ch.to_ascii_lowercase() {
            'a' => Key::A,
            'b' => Key::B,
            'c' => Key::C,
            'd' => Key::D,
            'e' => Key::E,
            'f' => Key::F,
            'g' => Key::G,
            'h' => Key::H,
            'i' => Key::I,
            'j' => Key::J,
            'k' => Key::K,
            'l' => Key::L,
            'm' => Key::M,
            'n' => Key::N,
            'o' => Key::O,
            'p' => Key::P,
            'q' => Key::Q,
            'r' => Key::R,
            's' => Key::S,
            't' => Key::T,
            'u' => Key::U,
            'v' => Key::V,
            'w' => Key::W,
            'x' => Key::X,
            'y' => Key::Y,
            'z' => Key::Z,
            '0' => Key::Num0,
            '1' => Key::Num1,
            '2' => Key::Num2,
            '3' => Key::Num3,
            '4' => Key::Num4,
            '5' => Key::Num5,
            '6' => Key::Num6,
            '7' => Key::Num7,
            '8' => Key::Num8,
            '9' => Key::Num9,
            ' ' => Key::Space,
            ',' => Key::Comma,
            '.' => Key::Period,
            '=' => Key::Equals,
            '+' => Key::Plus,
            '-' => Key::Minus,
            ';' => Key::Semicolon,
            '\'' => Key::Quote,
            '[' => Key::LeftBracket,
            ']' => Key::RightBracket,
            other => Key::Other(other as i32),
        }
    }
}

/// Modifier state on a key event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
    };
    pub const SHIFT: Mods = Mods {
        shift: true,
        ctrl: false,
        alt: false,
    };
    pub const CTRL: Mods = Mods {
        shift: false,
        ctrl: true,
        alt: false,
    };
    pub const ALT: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: true,
    };
}

/// One event the app loop hands to a state or the controller manager.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// Keyboard key down with optional character and modifier state.
    KeyDown {
        key: Key,
        mods: Mods,
        text: Option<char>,
        /// True when the operating system auto-repeated a held key.
        repeat: bool,
    },
    KeyUp {
        key: Key,
        mods: Mods,
    },
    ControllerButtonDown {
        button: ControllerButton,
        instance_id: u32,
        /// True for the controller manager's own repeats of a held D-pad
        /// left or right.
        repeat: bool,
    },
    ControllerButtonUp {
        button: ControllerButton,
        instance_id: u32,
    },
    ControllerAxis {
        axis: ControllerAxis,
        value: i16,
        instance_id: u32,
    },
    ControllerAdded {
        device_index: u32,
    },
    ControllerRemoved {
        instance_id: u32,
    },
    WindowFocusGained,
    /// SDL releases every key when the window loses focus; the app drops
    /// what it thinks is held with them, so alt-tabbing away mid-action can
    /// never leave a control down.
    WindowFocusLost,
    Quit,
}

impl InputEvent {
    /// A key press without modifiers or text.
    pub fn key(key: Key) -> InputEvent {
        InputEvent::KeyDown {
            key,
            mods: Mods::NONE,
            text: None,
            repeat: false,
        }
    }

    /// A key press carrying its typed character.
    pub fn key_text(key: Key, text: char) -> InputEvent {
        InputEvent::KeyDown {
            key,
            mods: Mods::NONE,
            text: Some(text),
            repeat: false,
        }
    }

    /// A key press with modifiers held.
    pub fn key_mods(key: Key, mods: Mods) -> InputEvent {
        InputEvent::KeyDown {
            key,
            mods,
            text: None,
            repeat: false,
        }
    }

    pub fn key_up(key: Key) -> InputEvent {
        InputEvent::KeyUp {
            key,
            mods: Mods::NONE,
        }
    }

    /// A typed character on the key it lives on.
    pub fn typed(ch: char) -> InputEvent {
        InputEvent::key_text(Key::from_char(ch), ch)
    }

    /// A controller button press from instance 0.
    pub fn button(button: ControllerButton) -> InputEvent {
        InputEvent::ControllerButtonDown {
            button,
            instance_id: 0,
            repeat: false,
        }
    }

    pub fn button_up(button: ControllerButton) -> InputEvent {
        InputEvent::ControllerButtonUp {
            button,
            instance_id: 0,
        }
    }

    pub fn axis(axis: ControllerAxis, value: i16) -> InputEvent {
        InputEvent::ControllerAxis {
            axis,
            value,
            instance_id: 0,
        }
    }

    /// Whether this is one of the `CONTROLLER*` events the manager owns.
    pub fn is_controller_event(&self) -> bool {
        matches!(
            self,
            InputEvent::ControllerButtonDown { .. }
                | InputEvent::ControllerButtonUp { .. }
                | InputEvent::ControllerAxis { .. }
                | InputEvent::ControllerAdded { .. }
                | InputEvent::ControllerRemoved { .. }
        )
    }

    /// The key of a `KeyDown`, else `None`.
    pub fn key_down(&self) -> Option<(Key, Mods, Option<char>)> {
        match self {
            InputEvent::KeyDown {
                key, mods, text, ..
            } => Some((*key, *mods, *text)),
            _ => None,
        }
    }

    /// Whether a held key or a held D-pad direction repeated this press,
    /// rather than a fresh press (false for everything else).
    pub fn is_repeat(&self) -> bool {
        matches!(
            self,
            InputEvent::KeyDown { repeat: true, .. }
                | InputEvent::ControllerButtonDown { repeat: true, .. }
        )
    }

    /// The key this press stands for: a key press's own key, or the key a
    /// controller button stands in for (see [`controller_key_map`]), so a
    /// screen that reads keys works from a controller with no code of its
    /// own. `None` for anything that is not a press.
    pub fn pressed(&self) -> Option<Key> {
        match self {
            InputEvent::KeyDown { key, .. } => Some(*key),
            InputEvent::ControllerButtonDown { button, .. } => controller_key_map(*button),
            _ => None,
        }
    }
}

/// The key a controller button stands in for on a screen that reads keys:
/// A is Enter, B and Start are Escape, Back is F1 (help), and the D-pad is
/// the arrow keys.
pub fn controller_key_map(button: ControllerButton) -> Option<Key> {
    match button {
        ControllerButton::A => Some(Key::Return),
        ControllerButton::B | ControllerButton::Start => Some(Key::Escape),
        ControllerButton::Back => Some(Key::F1),
        ControllerButton::DPadUp => Some(Key::Up),
        ControllerButton::DPadDown => Some(Key::Down),
        ControllerButton::DPadLeft => Some(Key::Left),
        ControllerButton::DPadRight => Some(Key::Right),
        _ => None,
    }
}

/// SDL GameController buttons mapped onto a standard layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControllerButton {
    A,
    B,
    X,
    Y,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Misc1,
    Paddle1,
    Paddle2,
    Paddle3,
    Paddle4,
    Touchpad,
    /// The right stick pushed past its threshold, as a button. SDL never
    /// sends these; a game that turns the stick into four buttons makes them
    /// itself.
    RightStickUp,
    RightStickDown,
    RightStickLeft,
    RightStickRight,
}

/// SDL GameController analog axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControllerAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    TriggerLeft,
    TriggerRight,
}
