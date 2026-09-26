# M1 notes: Freight Fate menus, input, and app plumbing

Source: `C:\Users\joshu\Freight-fate\crates\freight-fate\src` at commit `c5d3ef8d`
(crate `freight-fate`; sibling crates `ff-core`, `bass-sys`). All paths below are
relative to that `src` directory unless stated. Line numbers are at `c5d3ef8d`.

STATUS: complete. Sections 1-10 plus the cut list at the end.

## 1. Key, Mods, InputEvent, State trait, TimedMessageState (`states/base.rs`, 798 lines)

File header / imports (`states/base.rs:26-40`):

```rust
use std::any::Any;

use ff_core::message_log::{Message, MessageLog};
use ff_core::radio::RadioState;

use crate::app::GameContext;
pub use crate::app::Say;
use crate::controller::{ControllerAxis, ControllerButton};
use crate::discord_presence::PresenceState;

mod menu;

pub use menu::{
    DynamicLabel, Label, Menu, MenuAction, MenuCore, MenuItem, SimpleMenuState, DEFAULT_INTRO_HELP,
};
```

Also in this file (not requested but used by menus / text entry):
`end_sentence` (50-57) and `spoken_char` (67-75):

```rust
/// A spoken fragment ending in exactly one sentence mark, never two.
///
/// Menu/list labels are sometimes plain ("Sleep 10 hours") and sometimes
/// whole sentences that already end in a period (settlement summary lines).
/// Appending "." unconditionally produces ".." which a screen reader voices as
/// "dot dot", so add the period only when one is not already there.
pub fn end_sentence(text: &str) -> String {
    let text = text.trim_end();
    if text.ends_with(['.', '!', '?', ':']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

/// A single typed character the way an accessible text field announces it.
///
/// Shared by every text-entry field in the game (currently just driver name
/// entry) so a character reviewed one at a time always sounds the same way.
/// A space reads as "space" rather than silence. A capital letter gets a
/// leading "cap" -- speech synthesis pronounces "J" and "j" identically, so
/// without the marker a player arrowing through a typed name has no way to
/// hear that the first letter capitalized the way they meant it to.
pub fn spoken_char(ch: char) -> String {
    if ch == ' ' {
        return "space".to_string();
    }
    if ch.is_alphabetic() && ch.is_uppercase() {
        return format!("cap {}", ch.to_lowercase());
    }
    ch.to_string()
}
```

### Key (`states/base.rs:79-236`)

```rust
/// The pygame `K_*` constants the game reads, as one enum. `Other` carries
/// any SDL keycode outside that set so text entry still sees its `text`.
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
    /// The key a typed character arrives on (`ord(ch.lower())` in the Python
    /// tests): letters and digits map to their keys, anything else to the
    /// few punctuation keys the game binds, else `Other(codepoint)`.
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
```

### Mods (`states/base.rs:238-267`)

```rust
/// Modifier state on a key event (`event.mod` masked with `KMOD_*`).
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
```

### InputEvent (`states/base.rs:269-403`)

Note: `ControllerButton` / `ControllerAxis` come from `crate::controller` (SDL
GameController enums re-modelled, see section 5). The SDL keycode <-> `Key`
conversion lives in `app::sdl_shell` (not in this file).

```rust
/// One event the app loop hands to a state or the controller manager.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// `KEYDOWN` with pygame's fused `unicode` (`None` when it was empty).
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
    /// what it thinks is held with them, so alt-tabbing away mid-drive can
    /// never leave a pedal down.
    WindowFocusLost,
    Quit,
}

impl InputEvent {
    /// `pygame.event.Event(KEYDOWN, key=key, unicode="")`.
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

    /// `CONTROLLERBUTTONDOWN` from instance 0.
    pub fn button(button: ControllerButton) -> InputEvent {
        InputEvent::ControllerButtonDown {
            button,
            instance_id: 0,
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

    /// Whether this is an OS auto-repeat `KeyDown` (false for everything else).
    pub fn key_repeat(&self) -> bool {
        matches!(self, InputEvent::KeyDown { repeat: true, .. })
    }
}
```

### AsAny + State trait (`states/base.rs:405-552`)

Every hook takes `ctx: &mut GameContext` (the whole game context). Trucking /
FF-specific hooks: `paces_main_speech` (driving), `presence`/`online_presence`
(Discord + drivers board, `PresenceState`), `ticks_covered_music`/
`tick_covered_music`, `applies_radio_settings`/`apply_radio_settings_now`,
`radio` (`ff_core::radio::RadioState`), `handle_message_review`
(`ctx.message_log`). Default `update` calls `ctx.update_music_rotation(dt)`.

```rust
// -- the State contract -------------------------------------------------------------

/// Downcasting support every state gets for free (`app.state()` as a
/// concrete screen in tests and the harness).
pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: 'static> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Base contract for all game screens.
pub trait State: AsAny {
    /// While this state is on top of the stack, main-channel speech
    /// (`ctx.say`) queues instead of interrupting. False for menus and
    /// readers -- cancelling on navigation is how screen readers behave --
    /// and True for the driving state, where an achievement or assist
    /// notice landing with interrupt=True would stamp on whatever the
    /// player was being told (speech priority research, R2). A menu pushed
    /// over a drive is the top state, so it keeps immediate speech.
    fn paces_main_speech(&self) -> bool {
        false
    }

    /// Called when the state becomes active (pushed or revealed).
    fn enter(&mut self, _ctx: &mut GameContext) {}

    /// Called when the state is removed from the stack.
    fn exit(&mut self, _ctx: &mut GameContext) {}

    fn handle_event(&mut self, _ctx: &mut GameContext, _event: &InputEvent) {}

    /// Controller buttons a plain keyboard-driven state understands,
    /// translated into the key events it already handles. This keeps simple
    /// screens (update prompts, name entry, the help reader) usable from a
    /// controller without bespoke code. Menus and the driving state
    /// override this entirely.
    fn handle_controller(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        let InputEvent::ControllerButtonDown { button, .. } = event else {
            return;
        };
        if let Some(key) = controller_key_map(*button) {
            self.handle_event(ctx, &InputEvent::key(key));
        }
    }

    /// Called when the active controller is unplugged. The driving state
    /// pauses; menus keep going on the keyboard.
    fn on_controller_disconnect(&mut self, _ctx: &mut GameContext) {}

    /// Per-frame update. Overrides MUST call `ctx.update_music_rotation(dt)`
    /// (the Python `super().update(dt)`).
    fn update(&mut self, ctx: &mut GameContext, dt: f64) {
        ctx.update_music_rotation(dt);
    }

    /// Visible text for the window, mirroring what speech says.
    fn lines(&self, _ctx: &GameContext) -> Vec<String> {
        Vec::new()
    }

    /// Broad, privacy-safe activity for Discord Rich Presence, or None to
    /// leave presence unchanged. The app polls the active state each frame
    /// and hands the result to the presence service; states only report
    /// status here and never touch Discord itself.
    fn presence(&self, _ctx: &GameContext) -> Option<PresenceState> {
        None
    }

    /// The on-duty snapshot for the live drivers board, or None.
    ///
    /// Only the active hauling states (driving, pulled over, resting,
    /// delivering, and the pause menu over any of them, which reports
    /// `PAUSED_ACTIVITY`) report themselves; everything else returns None so
    /// the player drops off the public board when they are not on a job.
    fn online_presence(&self, _ctx: &GameContext) -> Option<PresenceState> {
        None
    }

    /// `captures_text_input`: a screen that takes typed text declines the
    /// global message-review keys, so a comma reaches the field.
    fn captures_text_input(&self) -> bool {
        false
    }

    /// `wants_controller_repeat`: whether held D-pad left/right auto-repeat
    /// events are forwarded to `handle_controller` (menus say yes).
    fn wants_controller_repeat(&self) -> bool {
        false
    }

    /// Whether this state keeps music turning over while covered by a menu
    /// (the Python `hasattr(state, "tick_covered_music")`).
    fn ticks_covered_music(&self) -> bool {
        false
    }

    /// Advance a covered drive's own playlist (see `ticks_covered_music`).
    fn tick_covered_music(&mut self, _ctx: &mut GameContext, _dt: f64) {}

    /// Whether this state can apply a radio settings change right now (the
    /// Python `hasattr(state, "apply_radio_settings_now")`).
    fn applies_radio_settings(&self) -> bool {
        false
    }

    /// Apply a radio settings change to a covered drive, right now.
    fn apply_radio_settings_now(&mut self, _ctx: &mut GameContext) {}

    /// The state's radio, when it carries one (`getattr(state, "radio")`).
    fn radio(&self) -> Option<&RadioState> {
        None
    }

    /// Walk the message log. True when the key belonged to review.
    ///
    /// The app offers every key event here before the active state sees it,
    /// so the review controls work on every screen without a state opting
    /// in. Screens that take typed text are the exception: a driver name may
    /// well contain a comma, and punctuation has to reach the field.
    fn handle_message_review(&mut self, ctx: &mut GameContext, event: &InputEvent) -> bool {
        if self.captures_text_input() {
            return false;
        }
        handle_message_review(ctx, event)
    }
}

/// `State._controller_key_map`: A->Return, B->Escape, Back->F1, D-pad->arrows.
pub fn controller_key_map(button: ControllerButton) -> Option<Key> {
    match button {
        ControllerButton::A => Some(Key::Return),
        ControllerButton::B => Some(Key::Escape),
        ControllerButton::Back => Some(Key::F1),
        ControllerButton::DPadUp => Some(Key::Up),
        ControllerButton::DPadDown => Some(Key::Down),
        ControllerButton::DPadLeft => Some(Key::Left),
        ControllerButton::DPadRight => Some(Key::Right),
        _ => None,
    }
}
```

Message review (global `[ ] , .` keys; couples to `ff_core::message_log`),
`states/base.rs:554-664`:

```rust
/// The global review keys: `[`/`]` cycle the category filter, `,`/`.` step
/// through the log, `Ctrl+,`/`Ctrl+.` jump to the ends, `Ctrl+C` copies the
/// message in review. True when the key was one of them.
pub fn handle_message_review(ctx: &mut GameContext, event: &InputEvent) -> bool {
    let Some((key, mods, _)) = event.key_down() else {
        return false;
    };
    let ctrl = mods.ctrl;
    match key {
        Key::LeftBracket => {
            if let Some(category) = ctx.message_log.previous_category() {
                let notice = hidden_notice(&mut ctx.message_log);
                ctx.say_with(
                    format!("{category} messages.{notice}"),
                    Say::new().review(false),
                );
            }
            true
        }
        Key::RightBracket => {
            if let Some(category) = ctx.message_log.next_category() {
                let notice = hidden_notice(&mut ctx.message_log);
                ctx.say_with(
                    format!("{category} messages.{notice}"),
                    Say::new().review(false),
                );
            }
            true
        }
        Key::Comma if ctrl => {
            let message = ctx.message_log.first_message().cloned();
            speak_review_message(ctx, message.as_ref());
            true
        }
        Key::Period if ctrl => {
            let message = ctx.message_log.last_message().cloned();
            speak_review_message(ctx, message.as_ref());
            true
        }
        Key::Comma => {
            let message = ctx.message_log.previous_message().cloned();
            speak_review_message(ctx, message.as_ref());
            true
        }
        Key::Period => {
            let message = ctx.message_log.next_message().cloned();
            match message {
                None => {
                    // Already on the newest one this filter shows. Saying
                    // nothing is what let a whole delivery settlement sit
                    // invisible behind a filter left on Event, so the end of
                    // the list is exactly where the count has to be spoken.
                    let notice = hidden_notice(&mut ctx.message_log);
                    if !notice.is_empty() {
                        ctx.say_with(notice.trim().to_string(), Say::new().review(false));
                    }
                }
                Some(message) => speak_review_message(ctx, Some(&message)),
            }
            true
        }
        Key::C if ctrl => {
            let Some(message) = ctx.message_log.message_in_review().cloned() else {
                return true;
            };
            if ctx.write_clipboard_text(&message.text) {
                ctx.say_with("Message copied to clipboard.", Say::new().review(false));
            }
            true
        }
        _ => false,
    }
}

/// How many newer messages the chosen category is holding back.
///
/// Empty when nothing is filtered out, so the common case stays silent. The
/// filter is the driver's stated preference and now survives a lapse in
/// review (Tim S, 2026-08-21) -- which is safe only while the review keeps
/// saying what the preference costs.
pub fn hidden_notice(log: &mut MessageLog) -> String {
    let hidden = log.hidden_newer_count();
    if hidden == 0 {
        return String::new();
    }
    let plural = if hidden == 1 { "message" } else { "messages" };
    format!(" {hidden} newer {plural} outside this filter.")
}

/// Speak a reviewed message, with the hidden-count notice on the newest one.
pub fn speak_review_message(ctx: &mut GameContext, message: Option<&Message>) {
    let Some(message) = message else {
        return;
    };
    // Reviewing is a deliberate act: silence the event voice first, or a
    // hazard call still playing talks over the line being reviewed.
    ctx.stop_event_speech();
    let visible_len = ctx.message_log.filtered_messages().len();
    // Only on the newest one the filter shows: appending the count to every
    // step back through the history would read it a dozen times.
    let at_newest = visible_len > 0 && ctx.message_log.index >= visible_len as isize - 1;
    let notice = if at_newest {
        hidden_notice(&mut ctx.message_log)
    } else {
        String::new()
    };
    ctx.say_with(
        format!("{}{notice}", message.text),
        Say::new().review(false),
    );
}
```

### TimedMessageState (`states/base.rs:666-768`)

Uses `ctx.audio.play(key)` (default clip `"ui/notify"`), `ctx.say`,
`ctx.say_with(.., Say::queued())`, `ctx.update_music_rotation(dt)`.

```rust
// -- TimedMessageState ---------------------------------------------------------------

/// What a [`TimedMessageState`] does when its countdown ends.
pub type OnComplete = Box<dyn FnOnce(&mut GameContext)>;

/// A brief, spoken transition that ignores stray held navigation keys.
pub struct TimedMessageState {
    pub title: String,
    pub message: String,
    pub status: String,
    pub seconds: f64,
    pub remaining: f64,
    pub complete_text: String,
    pub sound_key: Option<String>,
    on_complete: Option<OnComplete>,
    complete: bool,
}

impl TimedMessageState {
    pub fn new(
        title: &str,
        message: &str,
        status: &str,
        seconds: f64,
        on_complete: impl FnOnce(&mut GameContext) + 'static,
    ) -> Self {
        let seconds = seconds.max(0.0);
        Self {
            title: title.to_string(),
            message: message.to_string(),
            status: status.to_string(),
            seconds,
            remaining: seconds,
            complete_text: String::new(),
            sound_key: Some("ui/notify".to_string()),
            on_complete: Some(Box::new(on_complete)),
            complete: false,
        }
    }

    pub fn complete_text(mut self, text: &str) -> Self {
        self.complete_text = text.to_string();
        self
    }

    /// `sound_key=...`; `None` plays nothing on entry.
    pub fn sound_key(mut self, key: Option<&str>) -> Self {
        self.sound_key = key.map(str::to_string);
        self
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }
}

impl State for TimedMessageState {
    fn enter(&mut self, ctx: &mut GameContext) {
        if let Some(key) = &self.sound_key {
            ctx.audio.play(key);
        }
        ctx.say(&self.message);
    }

    fn handle_event(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        let Some((key, _, _)) = event.key_down() else {
            return;
        };
        if matches!(
            key,
            Key::F1 | Key::Escape | Key::Return | Key::Space | Key::KpEnter
        ) {
            ctx.say(&self.status);
        }
    }

    fn update(&mut self, ctx: &mut GameContext, dt: f64) {
        ctx.update_music_rotation(dt);
        if self.complete {
            return;
        }
        self.remaining = (self.remaining - dt).max(0.0);
        if self.remaining > 0.0 {
            return;
        }
        self.complete = true;
        if !self.complete_text.is_empty() {
            ctx.say_with(self.complete_text.clone(), Say::queued());
        }
        if let Some(on_complete) = self.on_complete.take() {
            on_complete(ctx);
        }
    }

    fn lines(&self, _ctx: &GameContext) -> Vec<String> {
        vec![
            self.title.clone(),
            String::new(),
            self.status.clone(),
            format!("Ready in {:.1} seconds.", self.remaining),
        ]
    }
}
```

Unit tests in the same file (`states/base.rs:770-798`): `end_sentence_adds_one_mark_only`,
`spoken_char_marks_space_and_capitals`, `key_from_char_follows_ord_lower`.

## 2. `states/base/menu.rs` (full, 561 lines, verbatim)

Re-exported from `states/base.rs:38-40` as
`pub use menu::{DynamicLabel, Label, Menu, MenuAction, MenuCore, MenuItem, SimpleMenuState, DEFAULT_INTRO_HELP};`.
There is no free `build_items` function: `build_items` is the one required
method on the `Menu` trait (line 207). The macro paths are hard-wired to
`$crate::states::base::*`, `$crate::app::GameContext`,
`$crate::discord_presence::PresenceState`, and `ff_core::radio::RadioState`
(not `$crate`-relative -- line 499).

```rust
//! The accessible menu base: [`MenuCore`] + the [`Menu`] trait and its
//! rows ([`MenuItem`], [`Label`]), plus `impl_state_for_menu!` and the
//! ready-made [`SimpleMenuState`]. Split out of `states::base` for length;
//! everything here is re-exported from there.

use std::rc::Rc;

use ff_core::radio::RadioState;

use crate::app::{GameContext, Say};
use crate::controller::ControllerAction;
use crate::discord_presence::PresenceState;
use crate::states::base::{end_sentence, InputEvent, Key};

// -- menus -------------------------------------------------------------------------------

/// A menu row's label or help: a fixed string, or computed while the menu is
/// open. Callable for the same reason the Python field was: a row whose help
/// depends on state that changes while the menu is open (a waiting cloud
/// conflict, say) must not read out an answer that was true when the screen
/// was built. Read it through [`Label::text`], never off the field.
pub enum Label<S> {
    Text(String),
    Dynamic(DynamicLabel<S>),
}

/// A label computed while the menu is open.
pub type DynamicLabel<S> = Rc<dyn Fn(&S, &GameContext) -> String>;

impl<S> Label<S> {
    pub fn dynamic(f: impl Fn(&S, &GameContext) -> String + 'static) -> Self {
        Label::Dynamic(Rc::new(f))
    }

    pub fn text(&self, state: &S, ctx: &GameContext) -> String {
        match self {
            Label::Text(text) => text.clone(),
            Label::Dynamic(f) => f(state, ctx),
        }
    }
}

impl<S> Clone for Label<S> {
    fn clone(&self) -> Self {
        match self {
            Label::Text(text) => Label::Text(text.clone()),
            Label::Dynamic(f) => Label::Dynamic(Rc::clone(f)),
        }
    }
}

impl<S> From<&str> for Label<S> {
    fn from(text: &str) -> Self {
        Label::Text(text.to_string())
    }
}

impl<S> From<String> for Label<S> {
    fn from(text: String) -> Self {
        Label::Text(text)
    }
}

/// What activating a row does: the Python `action` callable, given the
/// screen and the context.
pub type MenuAction<S> = Rc<dyn Fn(&mut S, &mut GameContext)>;

/// One menu row (`MenuItem` dataclass).
pub struct MenuItem<S> {
    pub label: Label<S>,
    pub action: MenuAction<S>,
    pub help: Label<S>,
    /// The click played when this item is activated. `None` for items whose
    /// action plays its own confirmation sound, so the two do not stack.
    pub select_sound: Option<String>,
}

impl<S> Clone for MenuItem<S> {
    fn clone(&self) -> Self {
        Self {
            label: self.label.clone(),
            action: Rc::clone(&self.action),
            help: self.help.clone(),
            select_sound: self.select_sound.clone(),
        }
    }
}

impl<S: 'static> MenuItem<S> {
    /// `MenuItem(label, action)` with the default help and select sound.
    pub fn new(
        label: impl Into<Label<S>>,
        action: impl Fn(&mut S, &mut GameContext) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            action: Rc::new(action),
            help: Label::Text(String::new()),
            select_sound: Some("ui/menu_select".to_string()),
        }
    }

    /// A row that does nothing when activated (an information line).
    pub fn inert(label: impl Into<Label<S>>) -> Self {
        Self::new(label, |_, _| {})
    }

    pub fn help(mut self, help: impl Into<Label<S>>) -> Self {
        self.help = help.into();
        self
    }

    /// `select_sound=None` / another key.
    pub fn select_sound(mut self, key: Option<&str>) -> Self {
        self.select_sound = key.map(str::to_string);
        self
    }

    pub fn text(&self, state: &S, ctx: &GameContext) -> String {
        self.label.text(state, ctx)
    }

    pub fn help_text(&self, state: &S, ctx: &GameContext) -> String {
        self.help.text(state, ctx)
    }
}

/// The reusable state of a `MenuState`: the Python class attributes
/// (`title`, `intro_help`, `open_sound_key`) and instance fields (`items`,
/// `index`). Embed one in a screen struct and implement [`Menu`].
pub struct MenuCore<S> {
    pub title: String,
    pub intro_help: String,
    /// `None` plays nothing on entry.
    pub open_sound_key: Option<String>,
    pub items: Vec<MenuItem<S>>,
    pub index: usize,
}

pub const DEFAULT_INTRO_HELP: &str =
    "Enter selects, Escape goes back. Control stops the current speech.";

impl<S> MenuCore<S> {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            intro_help: DEFAULT_INTRO_HELP.to_string(),
            open_sound_key: Some("ui/menu_open".to_string()),
            items: Vec::new(),
            index: 0,
        }
    }

    pub fn with_intro_help(mut self, help: &str) -> Self {
        self.intro_help = help.to_string();
        self
    }

    pub fn with_open_sound(mut self, key: Option<&str>) -> Self {
        self.open_sound_key = key.map(str::to_string);
        self
    }

    fn clamp_index(&mut self) {
        self.index = self.index.min(self.items.len().saturating_sub(1));
    }
}

impl<S> Default for MenuCore<S> {
    fn default() -> Self {
        Self::new("Menu")
    }
}

/// A vertically navigated, fully spoken menu.
///
/// Implementors provide the two accessors and `build_items`; everything
/// else is the Python `MenuState` behaviour as provided methods, each
/// overridable. The [`State`] hooks (`enter`, `update`, `lines`, ...) are
/// here too, under the same names, so `impl_state_for_menu!` can forward a
/// screen's whole `State` impl to them -- a screen that needs a custom
/// `update` overrides `Menu::update` and the forwarding picks it up.
///
/// ```ignore
/// pub struct ConfirmQuitState { menu: MenuCore<Self> }
///
/// impl ConfirmQuitState {
///     pub fn new() -> Self { Self { menu: MenuCore::new("Quit Freight Fate?") } }
/// }
///
/// impl Menu for ConfirmQuitState {
///     fn menu(&self) -> &MenuCore<Self> { &self.menu }
///     fn menu_mut(&mut self) -> &mut MenuCore<Self> { &mut self.menu }
///     fn build_items(&mut self, _ctx: &mut GameContext) -> Vec<MenuItem<Self>> {
///         vec![
///             MenuItem::new("Yes, quit", |_s, ctx| ctx.quit()).help("Closes the game."),
///             MenuItem::new("No, go back", |s, ctx| s.go_back(ctx)),
///         ]
///     }
/// }
///
/// impl_state_for_menu!(ConfirmQuitState);
/// ```
pub trait Menu: Sized + 'static {
    fn menu(&self) -> &MenuCore<Self>;
    fn menu_mut(&mut self) -> &mut MenuCore<Self>;
    fn build_items(&mut self, ctx: &mut GameContext) -> Vec<MenuItem<Self>>;

    // -- State hooks, forwarded by impl_state_for_menu! ----------------------------

    fn paces_main_speech(&self) -> bool {
        false
    }

    fn enter(&mut self, ctx: &mut GameContext) {
        let items = self.build_items(ctx);
        let core = self.menu_mut();
        core.items = items;
        core.clamp_index();
        if let Some(key) = self.menu().open_sound_key.clone() {
            ctx.audio.play(&key);
        }
        self.announce_entry(ctx);
    }

    fn exit(&mut self, _ctx: &mut GameContext) {}

    fn update(&mut self, ctx: &mut GameContext, dt: f64) {
        ctx.update_music_rotation(dt);
    }

    fn on_controller_disconnect(&mut self, _ctx: &mut GameContext) {}

    fn presence(&self, _ctx: &GameContext) -> Option<PresenceState> {
        None
    }

    fn online_presence(&self, _ctx: &GameContext) -> Option<PresenceState> {
        None
    }

    fn captures_text_input(&self) -> bool {
        false
    }

    /// Menus opt into held D-pad left/right auto-repeat for adjusting options.
    fn wants_controller_repeat(&self) -> bool {
        true
    }

    fn ticks_covered_music(&self) -> bool {
        false
    }

    fn tick_covered_music(&mut self, _ctx: &mut GameContext, _dt: f64) {}

    fn applies_radio_settings(&self) -> bool {
        false
    }

    fn apply_radio_settings_now(&mut self, _ctx: &mut GameContext) {}

    fn radio(&self) -> Option<&RadioState> {
        None
    }

    fn handle_event(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        let Some((key, _mods, text)) = event.key_down() else {
            return;
        };
        match key {
            Key::Down => self.move_by(ctx, 1),
            Key::Up => self.move_by(ctx, -1),
            Key::Home => self.jump(ctx, 0),
            Key::End => {
                let last = self.menu().items.len().saturating_sub(1);
                self.jump(ctx, last);
            }
            Key::Return | Key::Space | Key::KpEnter => self.activate(ctx),
            Key::Escape => self.go_back(ctx),
            Key::F1 => {
                let help = self.current_help(ctx);
                ctx.say(&help);
            }
            Key::LCtrl | Key::RCtrl => ctx.stop_speech(),
            _ => {
                if let Some(ch) = text.filter(|ch| ch.is_alphanumeric()) {
                    let lower: String = ch.to_lowercase().collect();
                    self.first_letter_jump(ctx, &lower);
                }
            }
        }
    }

    fn handle_controller(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        match ctx.controller.menu_action(event) {
            Some(ControllerAction::MenuDown) => self.move_by(ctx, 1),
            Some(ControllerAction::MenuUp) => self.move_by(ctx, -1),
            Some(ControllerAction::AdjustRight) => self.adjust(ctx, 1),
            Some(ControllerAction::AdjustLeft) => self.adjust(ctx, -1),
            Some(ControllerAction::Confirm) => self.activate(ctx),
            Some(ControllerAction::Back) => self.go_back(ctx),
            Some(ControllerAction::Help) => {
                let help = self.current_help(ctx);
                ctx.say(&help);
            }
            None => {}
        }
    }

    fn lines(&self, ctx: &GameContext) -> Vec<String> {
        let core = self.menu();
        let mut out = vec![core.title.clone(), String::new()];
        for (i, item) in core.items.iter().enumerate() {
            let marker = if i == core.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out
    }

    // -- menu behaviour -------------------------------------------------------------

    fn announce_entry(&mut self, ctx: &mut GameContext) {
        let title = end_sentence(&self.menu().title);
        ctx.say(&title);
        let current = self.current_text(ctx);
        ctx.say_with(current, Say::queued().review(false));
    }

    fn refresh(&mut self, ctx: &mut GameContext, keep_index: bool) {
        let old = self.menu().index;
        let items = self.build_items(ctx);
        let core = self.menu_mut();
        core.items = items;
        core.index = if keep_index { old } else { 0 };
        core.clamp_index();
    }

    fn current_text(&self, ctx: &GameContext) -> String {
        let core = self.menu();
        if core.items.is_empty() {
            return "No options available.".to_string();
        }
        let label = end_sentence(&core.items[core.index].text(self, ctx));
        if !ctx.settings.announce_menu_position {
            return label;
        }
        format!("{label} {} of {}.", core.index + 1, core.items.len())
    }

    fn speak_current(&mut self, ctx: &mut GameContext) {
        let text = self.current_text(ctx);
        ctx.say_with(text, Say::new().review(false));
    }

    fn current_help(&self, ctx: &GameContext) -> String {
        let core = self.menu();
        if core.items.is_empty() {
            return core.intro_help.clone();
        }
        let item = &core.items[core.index];
        let help = item.help_text(self, ctx);
        if help.is_empty() {
            format!("{}.", item.text(self, ctx))
        } else {
            help
        }
    }

    fn move_by(&mut self, ctx: &mut GameContext, delta: i64) {
        let core = self.menu_mut();
        if core.items.is_empty() {
            return;
        }
        let n = core.items.len() as i64;
        core.index = ((core.index as i64 + delta).rem_euclid(n)) as usize;
        ctx.audio.play("ui/menu_move");
        self.speak_current(ctx);
    }

    fn jump(&mut self, ctx: &mut GameContext, index: usize) {
        let core = self.menu_mut();
        if core.items.is_empty() {
            return;
        }
        core.index = index.min(core.items.len() - 1);
        ctx.audio.play("ui/menu_move");
        self.speak_current(ctx);
    }

    fn activate(&mut self, ctx: &mut GameContext) {
        let core = self.menu();
        if core.items.is_empty() {
            return;
        }
        let item = core.items[core.index].clone();
        if let Some(sound) = &item.select_sound {
            ctx.audio.play(sound);
        }
        (item.action)(self, ctx);
    }

    fn go_back(&mut self, ctx: &mut GameContext) {
        ctx.audio.play("ui/menu_back");
        ctx.pop_state();
    }

    /// Change the current option (D-pad left/right). No-op unless the menu
    /// has adjustable options; the settings screens override this.
    fn adjust(&mut self, _ctx: &mut GameContext, _direction: i64) {}

    fn first_letter_jump(&mut self, ctx: &mut GameContext, ch: &str) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        let start = self.menu().index;
        for offset in 1..=n {
            let i = (start + offset) % n;
            let text = self.menu().items[i].text(self, ctx).to_lowercase();
            if text.starts_with(ch) {
                self.menu_mut().index = i;
                ctx.audio.play("ui/menu_move");
                self.speak_current(ctx);
                return;
            }
        }
    }
}

/// Write the [`State`] impl for a [`Menu`] screen, forwarding every hook to
/// the `Menu` method of the same name (see the example on [`Menu`]).
#[macro_export]
macro_rules! impl_state_for_menu {
    ($ty:ty) => {
        impl $crate::states::base::State for $ty {
            fn paces_main_speech(&self) -> bool {
                <Self as $crate::states::base::Menu>::paces_main_speech(self)
            }
            fn enter(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::base::Menu>::enter(self, ctx)
            }
            fn exit(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::base::Menu>::exit(self, ctx)
            }
            fn handle_event(
                &mut self,
                ctx: &mut $crate::app::GameContext,
                event: &$crate::states::base::InputEvent,
            ) {
                <Self as $crate::states::base::Menu>::handle_event(self, ctx, event)
            }
            fn handle_controller(
                &mut self,
                ctx: &mut $crate::app::GameContext,
                event: &$crate::states::base::InputEvent,
            ) {
                <Self as $crate::states::base::Menu>::handle_controller(self, ctx, event)
            }
            fn on_controller_disconnect(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::base::Menu>::on_controller_disconnect(self, ctx)
            }
            fn update(&mut self, ctx: &mut $crate::app::GameContext, dt: f64) {
                <Self as $crate::states::base::Menu>::update(self, ctx, dt)
            }
            fn lines(&self, ctx: &$crate::app::GameContext) -> Vec<String> {
                <Self as $crate::states::base::Menu>::lines(self, ctx)
            }
            fn presence(
                &self,
                ctx: &$crate::app::GameContext,
            ) -> Option<$crate::discord_presence::PresenceState> {
                <Self as $crate::states::base::Menu>::presence(self, ctx)
            }
            fn online_presence(
                &self,
                ctx: &$crate::app::GameContext,
            ) -> Option<$crate::discord_presence::PresenceState> {
                <Self as $crate::states::base::Menu>::online_presence(self, ctx)
            }
            fn captures_text_input(&self) -> bool {
                <Self as $crate::states::base::Menu>::captures_text_input(self)
            }
            fn wants_controller_repeat(&self) -> bool {
                <Self as $crate::states::base::Menu>::wants_controller_repeat(self)
            }
            fn ticks_covered_music(&self) -> bool {
                <Self as $crate::states::base::Menu>::ticks_covered_music(self)
            }
            fn tick_covered_music(&mut self, ctx: &mut $crate::app::GameContext, dt: f64) {
                <Self as $crate::states::base::Menu>::tick_covered_music(self, ctx, dt)
            }
            fn applies_radio_settings(&self) -> bool {
                <Self as $crate::states::base::Menu>::applies_radio_settings(self)
            }
            fn apply_radio_settings_now(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::base::Menu>::apply_radio_settings_now(self, ctx)
            }
            fn radio(&self) -> Option<&ff_core::radio::RadioState> {
                <Self as $crate::states::base::Menu>::radio(self)
            }
        }
    };
}

/// A ready-made menu for the simplest screens: a title and a fixed list of
/// rows, no state of its own (the Python pattern of a `MenuState` subclass
/// whose `build_items` returns literals). Also the example `Menu` impl.
pub struct SimpleMenuState {
    pub menu: MenuCore<SimpleMenuState>,
    rows: Vec<MenuItem<SimpleMenuState>>,
}

impl SimpleMenuState {
    pub fn new(title: &str, rows: Vec<MenuItem<SimpleMenuState>>) -> Self {
        Self {
            menu: MenuCore::new(title),
            rows,
        }
    }

    /// A screen of spoken lines: Up and Down read them one at a time, Enter
    /// repeats the line under focus, Back returns. The shape every readout
    /// takes (the driving status screens, the logbook), so a screen that
    /// used to be one long sentence can be re-read a line at a time.
    pub fn readout(title: &str, lines: Vec<String>) -> Self {
        let mut rows: Vec<MenuItem<SimpleMenuState>> = lines
            .into_iter()
            .map(|line| {
                let spoken = line.clone();
                MenuItem::new(line, move |_s: &mut SimpleMenuState, ctx| ctx.say(&spoken))
                    .help("Repeat this line.")
            })
            .collect();
        rows.push(
            MenuItem::new("Back", |s: &mut SimpleMenuState, ctx| s.go_back(ctx))
                .help("Return to the previous menu."),
        );
        let mut screen = Self::new(title, rows);
        screen.menu = screen.menu.with_intro_help(
            "Up and Down read a line at a time, Enter repeats it, Escape goes back.",
        );
        screen
    }
}

impl Menu for SimpleMenuState {
    fn menu(&self) -> &MenuCore<Self> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut GameContext) -> Vec<MenuItem<Self>> {
        self.rows.clone()
    }
}

impl_state_for_menu!(SimpleMenuState);
```

## 3. `states/text_entry.rs` (323 lines)

Consumers: `states/main_menu.rs:961-992` (`NameEntryState`, embeds
`TextEntryCore::new("New career", "Driver name")`),
`states/driving_radio_app.rs:364-381` (`RadioSearchEntryState`,
`TextEntryCore::new("Search stations", "Search")`),
`states/main_menu/settings_actions.rs:436-437` (`TextEntryState::new("Music seed", "Seed", ..)`).

Imports (`states/text_entry.rs:16-17`):

```rust
use crate::app::{GameContext, Say};
use crate::states::base::{spoken_char, InputEvent, Key, State};
```

Couplings: only `GameContext` methods `say`, `say_with`, `audio.play`,
`pop_state`, `update_music_rotation`, plus `spoken_char` from `states/base.rs`.
Earcons: `ui/menu_back` (Escape), `ui/tick` (typed char), `ui/error` (Backspace
at 0, cursor at an edge, Home/End when already there). F2 speaks the whole
text. Enter / KpEnter call `confirm`. `captures_text_input()` returns true so
the global message-review keys yield (a comma reaches the field). No trucking
concepts. The macro does NOT forward `handle_controller`, so text entry gets
`State::handle_controller`'s default (A->Return, B->Escape, Back->F1,
D-pad->arrows via `controller_key_map`).

Verbatim, types + trait + macro + ready-made state (`states/text_entry.rs:19-323`):

```rust
/// The longest string a field takes.
pub const MAX_LEN: usize = 24;

/// The field itself: what the Python class kept on `self`.
pub struct TextEntryCore {
    pub heading: String,
    pub field_label: String,
    /// The typed text (kept as chars so the cursor indexes code points the
    /// way a Python `str` does).
    pub name: Vec<char>,
    /// A gap position from 0 to `name.len()`, the way an OS text field's
    /// caret works.
    pub cursor: usize,
    pub max_len: usize,
}

impl TextEntryCore {
    pub fn new(heading: &str, field_label: &str) -> Self {
        Self {
            heading: heading.to_string(),
            field_label: field_label.to_string(),
            name: Vec::new(),
            cursor: 0,
            max_len: MAX_LEN,
        }
    }

    /// The typed text as a string.
    pub fn text(&self) -> String {
        self.name.iter().collect()
    }

    /// Replace the typed text (cursor to the end).
    pub fn set_text(&mut self, text: &str) {
        self.name = text.chars().collect();
        self.cursor = self.name.len();
    }
}

impl Default for TextEntryCore {
    fn default() -> Self {
        Self::new("Text entry", "Text")
    }
}

/// Accessible text entry: characters are echoed as you type.
///
/// A review cursor tracks where in the typed text the player currently is.
/// Left and Right step the cursor one character and speak whatever character
/// sits between the old and new position -- the standard way a screen reader
/// lets a player check what they typed without retyping it. Home and End
/// jump to either end and speak the character now there. Typing and
/// Backspace act at the cursor, not just at the end, so a player who arrows
/// back to fix a letter edits in place instead of being surprised that
/// Backspace deletes from the end regardless of where they were reviewing.
///
/// Implementors set `heading` and `field_label` on the core for the visual
/// lines, speak their own prompt in `enter` (the default speaks the shared
/// one), and act on Enter in `confirm`.
pub trait TextEntry: Sized + 'static {
    fn entry(&self) -> &TextEntryCore;
    fn entry_mut(&mut self) -> &mut TextEntryCore;
    /// Enter was pressed.
    fn confirm(&mut self, ctx: &mut GameContext);

    // -- State hooks, forwarded by impl_state_for_text_entry! ----------------------

    /// Keep typed commas; the global repeat key yields.
    fn captures_text_input(&self) -> bool {
        true
    }

    fn enter(&mut self, ctx: &mut GameContext) {
        let heading = self.entry().heading.clone();
        ctx.say(&format!(
            "{heading}. Type, then press Enter. \
             Left and right arrows review the letters you have typed, \
             Home and End jump to the start or end. Press Escape to cancel."
        ));
    }

    fn exit(&mut self, _ctx: &mut GameContext) {}

    fn update(&mut self, ctx: &mut GameContext, dt: f64) {
        ctx.update_music_rotation(dt);
    }

    fn handle_event(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        let Some((key, _mods, text)) = event.key_down() else {
            return;
        };
        match key {
            Key::Escape => {
                ctx.audio.play("ui/menu_back");
                ctx.pop_state();
            }
            Key::Return | Key::KpEnter => self.confirm(ctx),
            Key::Backspace => self.backspace(ctx),
            Key::Left => self.move_cursor(ctx, -1),
            Key::Right => self.move_cursor(ctx, 1),
            Key::Home => self.jump_cursor(ctx, 0, "Start"),
            Key::End => {
                let end = self.entry().name.len();
                self.jump_cursor(ctx, end, "End");
            }
            Key::F2 => {
                let field = self.entry();
                let text = if field.name.is_empty() {
                    "Empty.".to_string()
                } else {
                    field.text()
                };
                ctx.say_with(text, Say::new().review(false));
            }
            _ => {
                if let Some(ch) = text {
                    let field = self.entry();
                    if is_printable(ch) && field.name.len() < field.max_len {
                        self.insert(ctx, ch);
                    }
                }
            }
        }
    }

    fn lines(&self, _ctx: &GameContext) -> Vec<String> {
        let field = self.entry();
        let before: String = field.name[..field.cursor].iter().collect();
        let after: String = field.name[field.cursor..].iter().collect();
        vec![
            field.heading.clone(),
            String::new(),
            format!("{}: {before}|{after}", field.field_label),
            "Left and right arrows review letters, Home and End jump to the \
             ends. Enter to confirm, Escape to cancel, F2 to hear the whole text."
                .to_string(),
        ]
    }

    // -- field editing --------------------------------------------------------------

    fn insert(&mut self, ctx: &mut GameContext, ch: char) {
        let field = self.entry_mut();
        field.name.insert(field.cursor, ch);
        field.cursor += 1;
        ctx.audio.play("ui/tick");
        ctx.say_with(spoken_char(ch), Say::new().review(false));
    }

    fn backspace(&mut self, ctx: &mut GameContext) {
        let field = self.entry_mut();
        if field.cursor == 0 {
            ctx.audio.play("ui/error");
            return;
        }
        let removed = field.name.remove(field.cursor - 1);
        field.cursor -= 1;
        let rest = if field.name.is_empty() {
            "Empty.".to_string()
        } else {
            field.text()
        };
        ctx.say_with(
            format!("Deleted {}. {rest}", spoken_char(removed)),
            Say::new().review(false),
        );
    }

    /// Step the review cursor one character left or right.
    ///
    /// Both directions speak the character that sat between the old and new
    /// cursor position -- the character just moved over -- matching how a
    /// screen reader announces caret movement in a plain text field.
    fn move_cursor(&mut self, ctx: &mut GameContext, delta: i64) {
        let field = self.entry_mut();
        let new_cursor = field.cursor as i64 + delta;
        if new_cursor < 0 || new_cursor > field.name.len() as i64 {
            ctx.audio.play("ui/error");
            return;
        }
        let new_cursor = new_cursor as usize;
        let spoken_index = field.cursor.min(new_cursor);
        field.cursor = new_cursor;
        let ch = field.name[spoken_index];
        ctx.say_with(spoken_char(ch), Say::new().review(false));
    }

    fn jump_cursor(&mut self, ctx: &mut GameContext, target: usize, label: &str) {
        let field = self.entry_mut();
        if field.cursor == target {
            ctx.audio.play("ui/error");
            return;
        }
        field.cursor = target;
        if field.name.is_empty() {
            ctx.say_with(format!("{label}. Empty."), Say::new().review(false));
            return;
        }
        let edge = if target == 0 {
            field.name[0]
        } else {
            field.name[field.name.len() - 1]
        };
        ctx.say_with(
            format!("{label}. {}", spoken_char(edge)),
            Say::new().review(false),
        );
    }
}

/// `str.isprintable()` for one character, near enough: control characters
/// and the non-space separators are not typed into a name.
pub fn is_printable(ch: char) -> bool {
    if ch == ' ' {
        return true;
    }
    !(ch.is_control() || ch.is_whitespace())
}

/// Write the [`State`] impl for a [`TextEntry`] screen, forwarding the
/// hooks to the `TextEntry` methods of the same name.
#[macro_export]
macro_rules! impl_state_for_text_entry {
    ($ty:ty) => {
        impl $crate::states::base::State for $ty {
            fn enter(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::text_entry::TextEntry>::enter(self, ctx)
            }
            fn exit(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::text_entry::TextEntry>::exit(self, ctx)
            }
            fn update(&mut self, ctx: &mut $crate::app::GameContext, dt: f64) {
                <Self as $crate::states::text_entry::TextEntry>::update(self, ctx, dt)
            }
            fn handle_event(
                &mut self,
                ctx: &mut $crate::app::GameContext,
                event: &$crate::states::base::InputEvent,
            ) {
                <Self as $crate::states::text_entry::TextEntry>::handle_event(self, ctx, event)
            }
            fn lines(&self, ctx: &$crate::app::GameContext) -> Vec<String> {
                <Self as $crate::states::text_entry::TextEntry>::lines(self, ctx)
            }
            fn captures_text_input(&self) -> bool {
                <Self as $crate::states::text_entry::TextEntry>::captures_text_input(self)
            }
        }
    };
}

/// The ready-made field: a heading, a label, and a confirm closure that
/// receives the typed text.
pub struct TextEntryState {
    pub entry: TextEntryCore,
    on_confirm: OnConfirm,
}

/// What a [`TextEntryState`] does with the typed text on Enter.
pub type OnConfirm = Box<dyn FnMut(&mut GameContext, &str)>;

impl TextEntryState {
    pub fn new(
        heading: &str,
        field_label: &str,
        on_confirm: impl FnMut(&mut GameContext, &str) + 'static,
    ) -> Self {
        Self {
            entry: TextEntryCore::new(heading, field_label),
            on_confirm: Box::new(on_confirm),
        }
    }

    /// The typed text.
    pub fn name(&self) -> String {
        self.entry.text()
    }

    pub fn cursor(&self) -> usize {
        self.entry.cursor
    }
}

impl TextEntry for TextEntryState {
    fn entry(&self) -> &TextEntryCore {
        &self.entry
    }

    fn entry_mut(&mut self) -> &mut TextEntryCore {
        &mut self.entry
    }

    fn confirm(&mut self, ctx: &mut GameContext) {
        let text = self.entry.text();
        (self.on_confirm)(ctx, &text);
    }
}

impl_state_for_text_entry!(TextEntryState);

// Keep the trait object usable where a plain `State` is expected.
#[allow(dead_code)]
fn _assert_state(state: TextEntryState) -> Box<dyn State> {
    Box::new(state)
}
```

### Menu consumer example: `states/save_notice.rs` (281 lines)

Four `Menu` screens (`SaveModifiedNoticeState`, `LegacyCareerNoticeState`,
`DrivingRecordNoticeState`, `SaveMigrationNoticeState`). Pattern: override
`announce_entry` to speak a long notice followed by `self.current_text(ctx)`;
override `go_back` so Escape acknowledges. Imports (`states/save_notice.rs:4-9`):

```rust
use ff_core::models::enforcement;

use crate::app::GameContext;
use crate::impl_state_for_menu;
use crate::states::base::{Menu, MenuCore, MenuItem};
use crate::states::main_menu::{pending_notice_state, world_entry_state, NameEntryState};
```

Trucking couplings: `ff_core::models::enforcement` (line 4, used 189-190),
`ctx.profile` (30, 161, 187, 233; fields `integrity_notice_pending`,
`driving_record.notice_pending`, `migration_notice_pending`, `.save()`),
`states::main_menu::{pending_notice_state, world_entry_state, NameEntryState}`
(9, 13, 102), `ctx.replace_shared_with(next, true, true)` (14),
`ctx.replace_state(..)` (102), `log::error!` (33, 164, 236). Nothing in this
file moves to `hearken-app`; it is a usage sample. Minimal representative
screen (`states/save_notice.rs:220-281`):

```rust
pub struct SaveMigrationNoticeState {
    menu: MenuCore<Self>,
}

impl SaveMigrationNoticeState {
    pub fn new() -> Self {
        Self {
            menu: MenuCore::new("Save file updated")
                .with_intro_help("Enter continues to your career."),
        }
    }

    fn acknowledge(&mut self, ctx: &mut GameContext) {
        if let Some(p) = ctx.profile.as_mut() {
            p.migration_notice_pending = false;
            if let Err(e) = p.save() {
                log::error!("Could not save the profile: {e}");
            }
        }
        continue_to_career(ctx);
    }
}

impl Default for SaveMigrationNoticeState {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu for SaveMigrationNoticeState {
    fn menu(&self) -> &MenuCore<Self> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self> {
        &mut self.menu
    }

    fn announce_entry(&mut self, ctx: &mut GameContext) {
        let text = format!(
            "Save file updated. This career was converted from an older \
             version: every truck you own now keeps its own fuel, damage, tire \
             wear, and road grime. The truck you were driving keeps its \
             condition; your other trucks start fueled and fresh. The updated \
             save no longer opens in older versions of the game. {}",
            self.current_text(ctx)
        );
        ctx.say(&text);
    }

    fn build_items(&mut self, _ctx: &mut GameContext) -> Vec<MenuItem<Self>> {
        vec![MenuItem::new("OK", |s: &mut Self, ctx| s.acknowledge(ctx))
            .help("Continue to your career.")]
    }

    fn go_back(&mut self, ctx: &mut GameContext) {
        // Escape acknowledges too; the player must never be stuck here.
        self.acknowledge(ctx);
    }
}

impl_state_for_menu!(SaveMigrationNoticeState);
```

## 4. Key bindings (`bindings.rs` 1115 lines, `bindings/names.rs` 181 lines)

### Data model summary

- **Only driving controls are rebindable.** `Action` (`bindings.rs:39-97`) is a
  flat enum of 51 trucking actions. There is NO menu up/down/select/back/help
  action in `bindings.rs`. Menu keys are hard-coded in `Menu::handle_event`
  (`states/base/menu.rs:267-293`), and pad menu navigation goes through
  `ControllerAction` in `controller.rs` (section 5). The module doc
  (`bindings.rs:12-19`) says Escape, Enter, F1, Control, the Shift clutch,
  plus/minus, the radio dial keys, the message-review keys and every menu key
  are fixed on purpose.
- **Naming.** Each `Action` has a row in `const TABLE: &[Row]`
  (`bindings.rs:235-606`): `(Action, saved id, spoken label, default key chords, default pad chords)`.
  `Action::id()` is the snake_case id stored in settings. `Action::label()` is
  the spoken row label. Row order is screen order (`Action::all()`).
- **Keys.** `Chord { key: Key, mods: Mods }`. Matching is exact, falling back
  to the bare key when the modifiers are unclaimed (`action_for`, 810-818).
  Spoken via `Chord::spoken()` ("Alt J", "the Up arrow"), saved via
  `Chord::saved()` ("alt+j", "up"), parsed by `Chord::parse` (split on `+`).
  A default row may list several chords (`place_state` = Alt+1 and Alt+keypad 1);
  once moved, the action has exactly one chord.
- **Pad.** `PadChord { button: ControllerButton, modified: bool }`, where
  `modified` means the right bumper is held (second layer). Saved as `name` or
  `mod+name`.
- **Storage.** `KeyBindings { keys: HashMap<Action, Chord>, pad: HashMap<Action, PadChord> }`
  holds only overrides (empty = all defaults). Persisted as two strings on
  `ff_core::settings::Settings`: `key_bindings` and `pad_bindings`
  (`ff-core/src/settings.rs:649-655`), format `id=chord;id=chord`
  (`engine=z;cruise_resume=shift+o`; pad `engine=mod+a;horn=paddle_1`).
  Unreadable entries are skipped (`entries`, 964-969). An override equal to the
  default is removed rather than stored (846-850, 885-889).
- **Live table** is `GameContext.bindings: KeyBindings` (`app/context.rs:137`),
  built at `app/context.rs:253` and rebuilt by `GameContext::apply_bindings()`
  (`app/context.rs:779-783`). `GameContext::control_name(action)` and
  `control_hint(action_str)` (`app/context.rs:785-805`) choose the pad or the
  keyboard phrase by `self.controller.device() == ff_core::input_hints::CONTROLLER`.
- **Conflict handling.** `set_chord` / `set_pad_chord` return `Rebind`, checked
  in this order: `Reserved(reason)` (`reserved_key_reason` 655-700,
  `reserved_pad_reason` 703-714); `Unchanged` if it is already this action's
  chord; `Taken(other_action)` if another action has it (refused, never
  swapped); else `Done`.
- **Rebinding UI** is `states/main_menu/shortcuts.rs` (264 lines, see below):
  `ShortcutsState` is a `Menu` with a `capturing: Option<Action>` field. While
  capturing, the next KeyDown (or pad ButtonDown) becomes the new chord.
  Modifier-only keys are ignored (half a chord); Escape keeps the old binding.
  On `Rebind::Done` it calls `ctx.bindings.store(&mut ctx.settings)` then
  `save_settings(&ctx.settings)` and plays `ui/menu_select`; `Taken` and
  `Reserved` play `ui/error` and keep listening.
- **Held-key query.** `KeyBindings::pressed(&HeldKeys, Action)` (825-833)
  couples bindings to `app/held_keys.rs` (section 6).

### Generic vs trucking

- Generic, reusable as-is: `Chord` (99-174), `PadChord` (176-230), `Rebind`
  (716-725), the override-map design, the `id=chord;` storage format,
  `entries()` (964-969), `action_for` / `exact_action` / `pressed` /
  `set_chord` / `set_pad_chord` / `pad_action_for` logic, and all of
  `bindings/names.rs`.
- Trucking-specific: the whole `Action` enum and `TABLE` (every row);
  `reserved_key_reason`'s reasons (Escape = "pause menu", plus/minus = cruise
  target, PageUp/PageDown/semicolon/apostrophe = radio dial,
  comma/period/brackets = message review, Ctrl+C = copy message);
  `reserved_pad_reason` (left bumper = clutch, right bumper = second layer,
  Start = pause, Back = stop the driving voice); `hint_key_phrase` /
  `hint_pad_phrase` / `hint_action` (895-962, tied to `ff_core::input_hints`);
  the `key_bindings` / `pad_bindings` fields living on `ff_core::settings::Settings`.
- Implication for `hearken-app`: `Action` must become a game-supplied type
  (for example a trait with `all()`, `id()`, `label()`, `default_chords()`,
  `default_pad_chords()`, making `KeyBindings<A>` generic), and the reserved
  policy a game-supplied function. Menu navigation is outside this system today.

Imports (`bindings.rs:27-37`):

```rust
use std::collections::HashMap;

use ff_core::settings::Settings;

use crate::app::held_keys::HeldKeys;
use crate::controller::ControllerButton;
use crate::states::base::{Key, Mods};

mod names;

pub use names::{key_saved_name, key_spoken_name, pad_button_short_name, parse_key_name};
```

### Type definitions and API, verbatim (`bindings.rs:39-230`)

```rust
/// One discrete driving control a player can move to another key or button.
///
/// The order here is the order of the rows on the shortcuts screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Accelerate,
    Brake,
    EmergencyBrake,
    SteerLeft,
    SteerRight,
    Straighten,
    Engine,
    ParkingBrake,
    Horn,
    Cruise,
    CruiseResume,
    /// Pad only: the keyboard's plus and minus keys stay fixed.
    CruiseUp,
    CruiseDown,
    TakeExit,
    Rest,
    Status,
    Speed,
    SpeedLimit,
    SafeSpeed,
    Fuel,
    Clock,
    Route,
    Weather,
    Lane,
    LaneLocator,
    Grade,
    Upcoming,
    LastAnnouncement,
    Cb,
    HosWheel,
    HosBreak,
    HosDrive,
    PlaceState,
    PlaceRoad,
    PlaceTown,
    PlaceDirection,
    EngineBrake,
    AutoJake,
    JakeStage1,
    JakeStage2,
    JakeStage3,
    /// Pad only: one button walks the stages the keyboard has three keys for.
    CycleJake,
    ShiftUp,
    ShiftDown,
    Neutral,
    Reverse,
    TransmissionMode,
    Radio,
    RadioFavorite,
    RadioStatus,
    RadioNowPlaying,
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

    /// As a hint names it: "the A button", "D-pad down", "right bumper plus
    /// Y".
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
```

`Row` type and the first two TABLE rows (`bindings.rs:232-259`). The rest of the
table (260-606) has the same shape, one row per `Action`:

```rust
/// `(action, saved id, spoken label, default keyboard chords, default pad
/// chords)`. A control with more than one default chord keeps them all
/// until the player moves it, and then has only the chord they chose.
type Row = (
    Action,
    &'static str,
    &'static str,
    &'static [Chord],
    &'static [PadChord],
);

use ControllerButton as Pad;

const TABLE: &[Row] = &[
    (
        Action::Accelerate,
        "accelerate",
        "Accelerate",
        &[Chord::plain(Key::Up)],
        &[],
    ),
    (
        Action::Brake,
        "brake",
        "Brake",
        &[Chord::plain(Key::Down)],
        &[],
    ),
    // ... one row per Action through line 606 ...
];
```

Full default table, condensed from `TABLE` (`mod+` = right bumper held):

| id | label | keys | pad |
|---|---|---|---|
| accelerate | Accelerate | Up | |
| brake | Brake | Down | |
| emergency_brake | Emergency brake | B | |
| steer_left | Steer left | Left | |
| steer_right | Steer right | Right | |
| straighten | Straighten up | Slash | |
| engine | Engine on or off | E | mod+A |
| parking_brake | Parking brake | P | mod+Y |
| horn | Horn | H | LeftStick |
| cruise | Automatic speed control | K | Y |
| cruise_resume | Resume the last cruise speed | Shift+K | |
| cruise_up | Cruise target up | | mod+DPadRight |
| cruise_down | Cruise target down | | mod+DPadLeft |
| take_exit | Signal for the exit or a pull-over | X | DPadDown |
| rest | Rest stop | T | mod+DPadDown |
| status | Status menu | Tab | mod+Start |
| speed | Speed | Space | B |
| speed_limit | Posted speed limit | S | mod+X |
| safe_speed | Safe speed | D | |
| fuel | Fuel | F | mod+B |
| clock | Clock | C | DPadRight |
| route | Route and location | R | DPadUp, mod+DPadUp |
| weather | Weather | V | DPadLeft |
| lane | Lane position | L | |
| lane_locator | Lane locator | I | |
| grade | Grade | G | |
| upcoming | Road ahead | U | |
| last_announcement | Repeat the last announcement | A | |
| cb | Repeat the last CB chatter | Alt+C | |
| hos_wheel | Time at the wheel | Alt+A | |
| hos_break | When the break is due | Alt+S | |
| hos_drive | What ends this shift | Alt+D | |
| place_state | The state | Alt+Num1, Alt+Kp1 | |
| place_road | The road | Alt+Num2, Alt+Kp2 | |
| place_town | The town | Alt+Num3, Alt+Kp3 | |
| place_direction | The direction | Alt+Num4, Alt+Kp4 | |
| engine_brake | Engine brake | J | RightStick |
| auto_jake | Automatic engine brake on or off | Alt+J | |
| jake_stage_1 | Engine brake stage 1 | Num1 | |
| jake_stage_2 | Engine brake stage 2 | Num2 | |
| jake_stage_3 | Engine brake stage 3 | Num3 | |
| cycle_jake | Next engine brake stage | | mod+RightStick |
| shift_up | Shift up | W | A |
| shift_down | Shift down | Q | X |
| neutral | Neutral | N | |
| reverse | Reverse | Backspace | |
| transmission_mode | Automatic or manual shifting | Alt+T | |
| radio | Radio on or off | M | |
| radio_favorite | Save the station as a favorite | O | |
| radio_status | Radio status | Y | |
| radio_now_playing | What the radio is playing | Shift+Y | |
### Action impl, reserved-key policy, Rebind, KeyBindings, verbatim (`bindings.rs:608-969`)

```rust
fn row(action: Action) -> &'static Row {
    TABLE
        .iter()
        .find(|(a, ..)| *a == action)
        .expect("every Action has a row in TABLE")
}

impl Action {
    /// Every action, in screen order.
    pub fn all() -> impl Iterator<Item = Action> {
        TABLE.iter().map(|(a, ..)| *a)
    }

    /// The id the settings file stores.
    pub fn id(self) -> &'static str {
        row(self).1
    }

    pub fn from_id(id: &str) -> Option<Action> {
        TABLE.iter().find(|(_, i, ..)| *i == id).map(|(a, ..)| *a)
    }

    /// The row label: what the control does.
    pub fn label(self) -> &'static str {
        row(self).2
    }

    pub fn default_chords(self) -> &'static [Chord] {
        row(self).3
    }

    pub fn default_pad_chords(self) -> &'static [PadChord] {
        row(self).4
    }

    /// Whether the keyboard has this control at all.
    pub fn on_keyboard(self) -> bool {
        !self.default_chords().is_empty()
    }

    /// Whether the pad has this control at all.
    pub fn on_pad(self) -> bool {
        !self.default_pad_chords().is_empty()
    }
}

/// Why a key cannot be chosen, as the screen says it.
pub fn reserved_key_reason(chord: &Chord) -> Option<&'static str> {
    let bare = chord.mods == Mods::NONE;
    let fixed = match chord.key {
        Key::Escape => Some("Escape is the pause menu"),
        Key::Return | Key::KpEnter => Some("Enter confirms"),
        Key::F1 => Some("F1 is help"),
        Key::LCtrl | Key::RCtrl | Key::LShift | Key::RShift | Key::LAlt | Key::RAlt => {
            Some("a modifier key on its own cannot be a shortcut")
        }
        Key::Equals | Key::Plus | Key::KpPlus | Key::Minus | Key::KpMinus => {
            Some("plus and minus set the cruise target")
        }
        Key::PageUp | Key::PageDown | Key::Semicolon | Key::Quote => {
            Some("the radio dial keys are fixed")
        }
        Key::Comma | Key::Period | Key::LeftBracket | Key::RightBracket => {
            Some("the message review keys are fixed")
        }
        Key::Other(_) => Some("that key cannot be used"),
        _ => None,
    };
    if let Some(reason) = fixed {
        // Escape, F1 and the modifier keys are fixed however they are
        // pressed (the first two are answered before the table is asked);
        // the rest are only claimed bare, so Alt with a review key is free.
        let always = matches!(
            chord.key,
            Key::Escape
                | Key::F1
                | Key::LCtrl
                | Key::RCtrl
                | Key::LShift
                | Key::RShift
                | Key::LAlt
                | Key::RAlt
                | Key::Other(_)
        );
        if bare || always {
            return Some(reason);
        }
    }
    if chord.key == Key::C && chord.mods == Mods::CTRL {
        return Some("Control C copies the message in review");
    }
    None
}

/// Why a pad chord cannot be chosen, as the screen says it.
pub fn reserved_pad_reason(chord: &PadChord) -> Option<&'static str> {
    match chord.button {
        ControllerButton::LeftShoulder => Some("the left bumper is the clutch"),
        ControllerButton::RightShoulder => Some("the right bumper is the second layer"),
        ControllerButton::Guide => Some("the guide button belongs to the system"),
        ControllerButton::Start if !chord.modified => Some("Start is pause"),
        ControllerButton::Back if !chord.modified => {
            Some("Back stops the driving voice and reads help")
        }
        _ => None,
    }
}

/// The result of asking for a new binding.
#[derive(Debug, PartialEq, Eq)]
pub enum Rebind {
    Done,
    /// The chord already belongs to this action.
    Unchanged,
    Reserved(&'static str),
    /// Another action has the chord.
    Taken(Action),
}

/// The live table: defaults plus what the player moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyBindings {
    keys: HashMap<Action, Chord>,
    pad: HashMap<Action, PadChord>,
}

impl KeyBindings {
    /// The table `settings` describes; unreadable entries are ignored, so
    /// one bad line never costs the rest.
    pub fn from_settings(settings: &Settings) -> KeyBindings {
        let mut out = KeyBindings::default();
        for (id, value) in entries(&settings.key_bindings) {
            if let (Some(action), Some(chord)) = (Action::from_id(id), Chord::parse(value)) {
                if action.on_keyboard() {
                    out.keys.insert(action, chord);
                }
            }
        }
        for (id, value) in entries(&settings.pad_bindings) {
            if let (Some(action), Some(chord)) = (Action::from_id(id), PadChord::parse(value)) {
                if action.on_pad() {
                    out.pad.insert(action, chord);
                }
            }
        }
        out
    }

    /// Write the moved controls back into `settings`.
    pub fn store(&self, settings: &mut Settings) {
        let mut keys: Vec<String> = Vec::new();
        let mut pad: Vec<String> = Vec::new();
        for action in Action::all() {
            if let Some(saved) = self.keys.get(&action).and_then(Chord::saved) {
                keys.push(format!("{}={saved}", action.id()));
            }
            if let Some(chord) = self.pad.get(&action) {
                pad.push(format!("{}={}", action.id(), chord.saved()));
            }
        }
        settings.key_bindings = keys.join(";");
        settings.pad_bindings = pad.join(";");
    }

    /// True when nothing has been moved from its default.
    pub fn is_default(&self) -> bool {
        self.keys.is_empty() && self.pad.is_empty()
    }

    pub fn reset(&mut self) {
        self.keys.clear();
        self.pad.clear();
    }

    pub fn reset_keys(&mut self) {
        self.keys.clear();
    }

    pub fn reset_pad(&mut self) {
        self.pad.clear();
    }

    // -- keyboard ----------------------------------------------------------------

    /// The chords that trigger `action` today.
    pub fn chords(&self, action: Action) -> Vec<Chord> {
        match self.keys.get(&action) {
            Some(chord) => vec![*chord],
            None => action.default_chords().to_vec(),
        }
    }

    /// The control's name for a spoken prompt, first chord only.
    pub fn spoken(&self, action: Action) -> String {
        self.chords(action)
            .first()
            .map(Chord::spoken)
            .unwrap_or_else(|| action.label().to_string())
    }

    /// The action a key press means: the exact chord if one is bound, else
    /// the bare key's action.
    pub fn action_for(&self, key: Key, mods: Mods) -> Option<Action> {
        self.exact_action(&Chord { key, mods }).or_else(|| {
            if mods == Mods::NONE {
                None
            } else {
                self.exact_action(&Chord::plain(key))
            }
        })
    }

    fn exact_action(&self, chord: &Chord) -> Option<Action> {
        Action::all().find(|action| self.chords(*action).contains(chord))
    }

    /// Whether `action`'s key is down right now, modifiers included.
    pub fn pressed(&self, input: &HeldKeys, action: Action) -> bool {
        let held = input.mods();
        self.chords(action).iter().any(|chord| {
            input.is_pressed(chord.key)
                && (!chord.mods.shift || held.shift)
                && (!chord.mods.ctrl || held.ctrl)
                && (!chord.mods.alt || held.alt)
        })
    }

    /// Move `action` to `chord`.
    pub fn set_chord(&mut self, action: Action, chord: Chord) -> Rebind {
        if let Some(reason) = reserved_key_reason(&chord) {
            return Rebind::Reserved(reason);
        }
        if self.chords(action).contains(&chord) {
            return Rebind::Unchanged;
        }
        if let Some(other) = self.exact_action(&chord) {
            return Rebind::Taken(other);
        }
        if action.default_chords() == [chord] {
            self.keys.remove(&action);
        } else {
            self.keys.insert(action, chord);
        }
        Rebind::Done
    }

    // -- pad ---------------------------------------------------------------------

    pub fn pad_chords(&self, action: Action) -> Vec<PadChord> {
        match self.pad.get(&action) {
            Some(chord) => vec![*chord],
            None => action.default_pad_chords().to_vec(),
        }
    }

    pub fn pad_spoken(&self, action: Action) -> String {
        self.pad_chords(action)
            .first()
            .map(PadChord::spoken)
            .unwrap_or_else(|| action.label().to_string())
    }

    pub fn pad_action_for(&self, button: ControllerButton, modified: bool) -> Option<Action> {
        let chord = PadChord { button, modified };
        Action::all().find(|action| self.pad_chords(*action).contains(&chord))
    }

    pub fn set_pad_chord(&mut self, action: Action, chord: PadChord) -> Rebind {
        if let Some(reason) = reserved_pad_reason(&chord) {
            return Rebind::Reserved(reason);
        }
        if self.pad_chords(action).contains(&chord) {
            return Rebind::Unchanged;
        }
        if let Some(other) = self.pad_action_for(chord.button, chord.modified) {
            return Rebind::Taken(other);
        }
        if action.default_pad_chords() == [chord] {
            self.pad.remove(&action);
        } else {
            self.pad.insert(action, chord);
        }
        Rebind::Done
    }

    // -- spoken hints -------------------------------------------------------------

    /// The keyboard phrase for a `control_hint` action id, when the hint
    /// names a control the player can move; `None` leaves the fixed table's
    /// wording alone.
    pub fn hint_key_phrase(&self, hint: &str) -> Option<String> {
        Some(match hint {
            "gears" => format!(
                "{} and {}",
                self.spoken(Action::ShiftUp),
                self.spoken(Action::ShiftDown)
            ),
            "gear_first" => self.spoken(Action::ShiftUp),
            other => self.spoken(hint_action(other)?),
        })
    }

    /// The pad phrase for a `control_hint` action id, same contract. The
    /// two-button phrases keep the table's tighter wording ("the A and X
    /// buttons") until one of the pair moves.
    pub fn hint_pad_phrase(&self, hint: &str) -> Option<String> {
        let pair = |a: Action, b: Action, joiner: &str| {
            if self.pad.contains_key(&a) || self.pad.contains_key(&b) {
                Some(format!(
                    "{} {joiner} {}",
                    self.pad_spoken(a),
                    self.pad_spoken(b)
                ))
            } else {
                None
            }
        };
        let action = match hint {
            "gear_first" => Action::ShiftUp,
            "cruise_adjust" => return pair(Action::CruiseDown, Action::CruiseUp, "or"),
            "gears" => return pair(Action::ShiftUp, Action::ShiftDown, "and"),
            other => hint_action(other)?,
        };
        if !action.on_pad() {
            return None;
        }
        Some(self.pad_spoken(action))
    }
}

/// The `input_hints` action ids that name a movable control.
fn hint_action(hint: &str) -> Option<Action> {
    Some(match hint {
        "accelerate" => Action::Accelerate,
        "brake" => Action::Brake,
        "emergency_brake" => Action::EmergencyBrake,
        "reverse" => Action::Reverse,
        "neutral" => Action::Neutral,
        "engine" => Action::Engine,
        "parking_brake" => Action::ParkingBrake,
        "take_exit" => Action::TakeExit,
        "rest" => Action::Rest,
        "cruise_set" => Action::Cruise,
        "speed" => Action::Speed,
        "status_menu" => Action::Status,
        "fuel" => Action::Fuel,
        "clock" => Action::Clock,
        "route" => Action::Route,
        "weather" => Action::Weather,
        "lane" => Action::Lane,
        "horn" => Action::Horn,
        "engine_brake" => Action::EngineBrake,
        _ => return None,
    })
}

/// `id=value` pairs from a saved bindings string.
fn entries(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.split(';')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.split_once('='))
}
```

### `bindings/names.rs` (181 lines, verbatim)

`KEYS` maps every `Key` a shortcut can land on to `(saved id, spoken name)`.
`PAD` maps every `ControllerButton` to `(saved id, short spoken name)`. Fully
generic. Its only imports are `crate::controller::ControllerButton` and
`crate::states::base::Key`.

```rust
//! Key and pad button names: the id the settings file stores and the words
//! the screen reader says.

use crate::controller::ControllerButton;
use crate::states::base::Key;

/// `(key, saved id, spoken name)` for every key a shortcut can land on.
const KEYS: &[(Key, &str, &str)] = &[
    (Key::A, "a", "A"),
    (Key::B, "b", "B"),
    (Key::C, "c", "C"),
    (Key::D, "d", "D"),
    (Key::E, "e", "E"),
    (Key::F, "f", "F"),
    (Key::G, "g", "G"),
    (Key::H, "h", "H"),
    (Key::I, "i", "I"),
    (Key::J, "j", "J"),
    (Key::K, "k", "K"),
    (Key::L, "l", "L"),
    (Key::M, "m", "M"),
    (Key::N, "n", "N"),
    (Key::O, "o", "O"),
    (Key::P, "p", "P"),
    (Key::Q, "q", "Q"),
    (Key::R, "r", "R"),
    (Key::S, "s", "S"),
    (Key::T, "t", "T"),
    (Key::U, "u", "U"),
    (Key::V, "v", "V"),
    (Key::W, "w", "W"),
    (Key::X, "x", "X"),
    (Key::Y, "y", "Y"),
    (Key::Z, "z", "Z"),
    (Key::Num0, "0", "0"),
    (Key::Num1, "1", "1"),
    (Key::Num2, "2", "2"),
    (Key::Num3, "3", "3"),
    (Key::Num4, "4", "4"),
    (Key::Num5, "5", "5"),
    (Key::Num6, "6", "6"),
    (Key::Num7, "7", "7"),
    (Key::Num8, "8", "8"),
    (Key::Num9, "9", "9"),
    (Key::Kp0, "kp_0", "keypad 0"),
    (Key::Kp1, "kp_1", "keypad 1"),
    (Key::Kp2, "kp_2", "keypad 2"),
    (Key::Kp3, "kp_3", "keypad 3"),
    (Key::Kp4, "kp_4", "keypad 4"),
    (Key::Kp5, "kp_5", "keypad 5"),
    (Key::Kp6, "kp_6", "keypad 6"),
    (Key::Kp7, "kp_7", "keypad 7"),
    (Key::Kp8, "kp_8", "keypad 8"),
    (Key::Kp9, "kp_9", "keypad 9"),
    (Key::KpEnter, "kp_enter", "keypad Enter"),
    (Key::KpPlus, "kp_plus", "keypad plus"),
    (Key::KpMinus, "kp_minus", "keypad minus"),
    (Key::F1, "f1", "F1"),
    (Key::F2, "f2", "F2"),
    (Key::F3, "f3", "F3"),
    (Key::F4, "f4", "F4"),
    (Key::F5, "f5", "F5"),
    (Key::F6, "f6", "F6"),
    (Key::F7, "f7", "F7"),
    (Key::F8, "f8", "F8"),
    (Key::F9, "f9", "F9"),
    (Key::F10, "f10", "F10"),
    (Key::F11, "f11", "F11"),
    (Key::F12, "f12", "F12"),
    (Key::Return, "enter", "Enter"),
    (Key::Escape, "escape", "Escape"),
    (Key::Space, "space", "Space"),
    (Key::Tab, "tab", "Tab"),
    (Key::Backspace, "backspace", "Backspace"),
    (Key::Up, "up", "the Up arrow"),
    (Key::Down, "down", "the Down arrow"),
    (Key::Left, "left", "the Left arrow"),
    (Key::Right, "right", "the Right arrow"),
    (Key::Home, "home", "Home"),
    (Key::End, "end", "End"),
    (Key::PageUp, "page_up", "Page Up"),
    (Key::PageDown, "page_down", "Page Down"),
    (Key::Insert, "insert", "Insert"),
    (Key::Delete, "delete", "Delete"),
    (Key::Comma, "comma", "comma"),
    (Key::Period, "period", "period"),
    (Key::Slash, "slash", "slash"),
    (Key::Backslash, "backslash", "backslash"),
    (Key::Backquote, "grave", "grave accent"),
    (Key::Equals, "equals", "equals"),
    (Key::Plus, "plus", "plus"),
    (Key::Minus, "minus", "minus"),
    (Key::Semicolon, "semicolon", "semicolon"),
    (Key::Quote, "apostrophe", "apostrophe"),
    (Key::LeftBracket, "left_bracket", "left bracket"),
    (Key::RightBracket, "right_bracket", "right bracket"),
    (Key::LCtrl, "left_ctrl", "Left Control"),
    (Key::RCtrl, "right_ctrl", "Right Control"),
    (Key::LShift, "left_shift", "Left Shift"),
    (Key::RShift, "right_shift", "Right Shift"),
    (Key::LAlt, "left_alt", "Left Alt"),
    (Key::RAlt, "right_alt", "Right Alt"),
];

/// The id the settings file stores for `key`; `None` for a key the game
/// has no name for.
pub fn key_saved_name(key: Key) -> Option<&'static str> {
    KEYS.iter().find(|(k, ..)| *k == key).map(|(_, id, _)| *id)
}

/// The key as the screen reader says it.
pub fn key_spoken_name(key: Key) -> String {
    match KEYS.iter().find(|(k, ..)| *k == key) {
        Some((_, _, spoken)) => (*spoken).to_string(),
        None => "an unnamed key".to_string(),
    }
}

pub fn parse_key_name(id: &str) -> Option<Key> {
    KEYS.iter().find(|(_, i, _)| *i == id).map(|(k, ..)| *k)
}

/// `(button, saved id, short spoken name)`.
const PAD: &[(ControllerButton, &str, &str)] = &[
    (ControllerButton::A, "a", "A"),
    (ControllerButton::B, "b", "B"),
    (ControllerButton::X, "x", "X"),
    (ControllerButton::Y, "y", "Y"),
    (ControllerButton::Back, "back", "Back"),
    (ControllerButton::Guide, "guide", "the guide button"),
    (ControllerButton::Start, "start", "Start"),
    (
        ControllerButton::LeftStick,
        "left_stick",
        "the left stick click",
    ),
    (
        ControllerButton::RightStick,
        "right_stick",
        "the right stick click",
    ),
    (
        ControllerButton::LeftShoulder,
        "left_bumper",
        "the left bumper",
    ),
    (
        ControllerButton::RightShoulder,
        "right_bumper",
        "the right bumper",
    ),
    (ControllerButton::DPadUp, "dpad_up", "D-pad up"),
    (ControllerButton::DPadDown, "dpad_down", "D-pad down"),
    (ControllerButton::DPadLeft, "dpad_left", "D-pad left"),
    (ControllerButton::DPadRight, "dpad_right", "D-pad right"),
    (ControllerButton::Misc1, "misc", "the extra button"),
    (ControllerButton::Paddle1, "paddle_1", "paddle 1"),
    (ControllerButton::Paddle2, "paddle_2", "paddle 2"),
    (ControllerButton::Paddle3, "paddle_3", "paddle 3"),
    (ControllerButton::Paddle4, "paddle_4", "paddle 4"),
    (ControllerButton::Touchpad, "touchpad", "the touchpad"),
];

pub(super) fn pad_button_saved_name(button: ControllerButton) -> &'static str {
    PAD.iter()
        .find(|(b, ..)| *b == button)
        .map(|(_, id, _)| *id)
        .unwrap_or("unknown")
}

/// The button as a hint names it after "right bumper plus".
pub fn pad_button_short_name(button: ControllerButton) -> &'static str {
    PAD.iter()
        .find(|(b, ..)| *b == button)
        .map(|(_, _, spoken)| *spoken)
        .unwrap_or("an unnamed button")
}

pub(super) fn parse_pad_button_name(id: &str) -> Option<ControllerButton> {
    PAD.iter().find(|(_, i, _)| *i == id).map(|(b, ..)| *b)
}
```

### Rebinding screen: `states/main_menu/shortcuts.rs` (264 lines)

The task named a top-level `shortcuts.rs`; the only file of that name is
`states/main_menu/shortcuts.rs`. Imports (`12-18`):

```rust
use crate::app::GameContext;
use crate::bindings::{Action, Chord, PadChord, Rebind};
use crate::controller::{ControllerAction, ControllerButton};
use crate::impl_state_for_menu;
use crate::states::base::{InputEvent, Key, Label, Menu, MenuCore, MenuItem, Mods};

use super::settings::{base_handle_event, save_settings};
```

Couplings: `ctx.bindings` (65-66, 135, 150, 156-159); `ctx.settings` plus FF's
`save_settings` (`states/main_menu/settings.rs:109-113`, wraps
`Settings::save()` with `log::warn!`); `ctx.controller.modifier` (148, the
right-bumper layer flag); `ctx.controller.menu_action` (234); and
`base_handle_event` from `states/main_menu/settings.rs:249-274`, which is a
verbatim copy of the default body of `Menu::handle_event`. The copy exists
because a Rust override cannot call the trait default. `hearken-app` should
export that body as a free `pub fn` (e.g. `menu_handle_event<S: Menu>(s, ctx, event)`)
so overriding screens can delegate to it.

Capture logic, verbatim (`states/main_menu/shortcuts.rs:70-166` and `218-251`):

```rust
    fn begin_capture(&mut self, ctx: &mut GameContext, action: Action) {
        self.capturing = Some(action);
        let current = self.current_name(ctx, action);
        let prompt = match self.device {
            ShortcutDevice::Keyboard => format!(
                "Press the new key for {}. Escape keeps {current}.",
                action.label()
            ),
            ShortcutDevice::Controller => format!(
                "Press the new button for {}, with the right bumper held for the \
                 second layer. Press {current} again to keep it.",
                action.label()
            ),
        };
        ctx.say(&prompt);
    }

    fn finish(&mut self, ctx: &mut GameContext, action: Action, result: Rebind, chosen: &str) {
        let current = self.current_name(ctx, action);
        match result {
            Rebind::Done => {
                self.capturing = None;
                ctx.bindings.store(&mut ctx.settings);
                save_settings(&ctx.settings);
                ctx.audio.play("ui/menu_select");
                let now = self.current_name(ctx, action);
                self.refresh(ctx, true);
                ctx.say(&format!("{} is now {now}.", action.label()));
            }
            Rebind::Unchanged => {
                self.capturing = None;
                ctx.say(&format!("Kept {current}."));
            }
            Rebind::Taken(other) => {
                ctx.audio.play("ui/error");
                ctx.say(&format!(
                    "{} is already {}. Press another, or keep {current} with Escape.",
                    sentence_start(chosen),
                    other.label()
                ));
            }
            Rebind::Reserved(reason) => {
                ctx.audio.play("ui/error");
                ctx.say(&format!(
                    "{}. Press another, or keep {current} with Escape.",
                    sentence_start(reason)
                ));
            }
        }
    }

    fn capture_key(&mut self, ctx: &mut GameContext, action: Action, key: Key, mods: Mods) {
        if matches!(
            key,
            Key::LCtrl | Key::RCtrl | Key::LShift | Key::RShift | Key::LAlt | Key::RAlt
        ) {
            return; // half a chord: wait for the key that goes with it
        }
        if key == Key::Escape {
            self.capturing = None;
            let current = self.current_name(ctx, action);
            ctx.say(&format!("Kept {current}."));
            return;
        }
        let chord = Chord { key, mods };
        let result = ctx.bindings.set_chord(action, chord);
        self.finish(ctx, action, result, &chord.spoken());
    }

    fn capture_button(&mut self, ctx: &mut GameContext, action: Action, button: ControllerButton) {
        if matches!(
            button,
            ControllerButton::LeftShoulder | ControllerButton::RightShoulder
        ) {
            return; // the layer keys, not a choice
        }
        let chord = PadChord {
            button,
            modified: ctx.controller.modifier,
        };
        let result = ctx.bindings.set_pad_chord(action, chord);
        self.finish(ctx, action, result, &chord.spoken());
    }

    fn reset_all(&mut self, ctx: &mut GameContext) {
        match self.device {
            ShortcutDevice::Keyboard => ctx.bindings.reset_keys(),
            ShortcutDevice::Controller => ctx.bindings.reset_pad(),
        }
        ctx.bindings.store(&mut ctx.settings);
        save_settings(&ctx.settings);
        self.refresh(ctx, true);
        ctx.say(match self.device {
            ShortcutDevice::Keyboard => "Every keyboard shortcut is back to its default.",
            ShortcutDevice::Controller => "Every controller button is back to its default.",
        });
    }
    // ... (lines 167-217: Menu impl header and build_items) ...
    fn handle_event(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        let Some(action) = self.capturing else {
            return base_handle_event(self, ctx, event);
        };
        if let Some((key, mods, _)) = event.key_down() {
            self.capture_key(ctx, action, key, mods);
        }
    }

    fn handle_controller(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        if let Some(action) = self.capturing {
            if let InputEvent::ControllerButtonDown { button, .. } = event {
                self.capture_button(ctx, action, *button);
            }
            return;
        }
        match ctx.controller.menu_action(event) {
            Some(ControllerAction::MenuDown) => self.move_by(ctx, 1),
            Some(ControllerAction::MenuUp) => self.move_by(ctx, -1),
            Some(ControllerAction::Confirm) => self.activate(ctx),
            Some(ControllerAction::Back) => self.go_back(ctx),
            Some(ControllerAction::Help) => {
                let help = self.current_help(ctx);
                ctx.say(&help);
            }
            _ => {}
        }
    }

    fn go_back(&mut self, ctx: &mut GameContext) {
        save_settings(&ctx.settings);
        ctx.audio.play("ui/menu_back");
        ctx.pop_state();
    }
```

## 5. Controller (`controller.rs` 1001 lines; `controller/sdl.rs` 161, `controller/fakes.rs` 132, `controller/diagnostics.rs` 280)

### Summary

- **Library:** SDL2 GameController API via the `sdl2` crate (workspace pin
  `sdl2 = "0.38"`, `Freight-fate/Cargo.toml:51`; the game crate enables
  `raw-window-handle`, and on macOS/Linux `bundled` + `static-link`,
  `crates/freight-fate/Cargo.toml:68-95`). SDL remaps any recognized pad onto
  the Xbox layout.
- **Seam:** `ControllerManager` never touches SDL. The device is behind trait
  `PadDevice`, the subsystem behind trait `PadSubsystem`, created lazily by a
  `PadSubsystemFactory` (`Box<dyn FnMut() -> Option<Box<dyn PadSubsystem>>>`).
  `controller/sdl.rs` implements both over `sdl2::controller` and provides
  `sdl_factory(sdl: sdl2::Sdl)`. `controller/fakes.rs` implements both for tests
  (`FakePad`, `FakeSubsystem`, `fake_factory`). Tests use
  `ControllerManager::detached(enabled, haptics)` plus `bind_device(Box::new(FakePad::new(id)), "test pad")`.
- **Event flow** (`app.rs:666-681`, `app.rs:740-775`, `app.rs:781-806`):
  SDL events become `InputEvent::Controller*` in `app/sdl_shell.rs::translate_events`
  (261-345). `App::handle_event` sends every controller event to
  `App::dispatch_controller`, which calls `ctx.controller.process_event(event)`.
  Only an accepted button down/up from the bound pad returns true; then, if
  `ctx.controller.active()`, the top state's `State::handle_controller(ctx, event)`
  runs. Axis events only update cached analog state. Every frame
  `App::tick` calls `ctx.controller.tick(dt)`, which returns synthetic
  `ControllerButtonDown` repeats for a held D-pad left/right; these go to
  `handle_controller` only when `state.wants_controller_repeat()`. Then
  `ctx.controller.take_disconnect()` makes the app say "Controller
  disconnected. You can keep playing with the keyboard, or reconnect your
  controller." and call `state.on_controller_disconnect(ctx)`.
  `dispatch_to_state` calls `ctx.controller.note_keyboard()` on every key down.
- **Mapping to menu actions:** `ControllerAction { MenuUp, MenuDown, AdjustLeft, AdjustRight, Confirm, Back, Help }`
  via `menu_action_for(button)` (139-150): D-pad up/down -> MenuUp/MenuDown,
  D-pad left/right -> AdjustLeft/AdjustRight, A -> Confirm, B -> Back,
  Back -> Help. `ControllerManager::menu_action(&InputEvent)` (799-804) applies
  it to a `ControllerButtonDown`. `Menu::handle_controller` consumes it
  (`states/base/menu.rs:295-309`).
- **Mapping to keys for plain states:** `controller_key_map` in
  `states/base.rs:540-552` (A->Return, B->Escape, Back->F1, D-pad->arrows),
  used by the default `State::handle_controller` (`states/base.rs:449-456`).
- **Driving actions on the pad** go through `KeyBindings::pad_action_for(button, ctx.controller.modifier)`
  (section 4), not through the manager.
- **Device tracking:** `active_device: &'static str` is `ff_core::input_hints::KEYBOARD`
  or `CONTROLLER`; set to CONTROLLER on button down or a non-zero steer/trigger
  axis, back to KEYBOARD by `note_keyboard()`. `device()` (629-635) reports
  CONTROLLER only while a pad is active and was used last.
- **Robustness:** one bound pad; the opened instance id is provisional
  (`id_pending`) until the first real event; foreign-id events are dropped and
  logged once; a duplicate ButtonDown without a ButtonUp is ignored
  (`buttons_down`); `ControllerAdded` re-opens only if nothing is bound or the
  bound pad has detached; the subsystem is never cycled (would double every
  event). The subsystem comes up lazily on first enable and is torn down on
  disable. `NO_CONTROLLER_ENV = "FREIGHT_FATE_NO_CONTROLLER"` + `pads_allowed()`
  (267-272) keep pads out of agent sessions.

### Generic vs trucking in `controller.rs`

- Generic: `ControllerButton`, `ControllerAxis`, `ControllerAction`,
  `button_label`, `menu_action_for`, `deadzone`, `smooth`, `RepeatTimer` and the
  `REPEAT_*` constants, `PadDevice`/`PadSubsystem`/`PadSubsystemFactory`,
  `mapping_keys`, `ESSENTIAL_MAPPINGS` + `log_capabilities`, the whole
  lifecycle/hot-plug/id-latching/duplicate-drop logic in `process_event`,
  `active_device`/`note_keyboard`/`device()`, `modifier` (right bumper layer),
  `controller/sdl.rs`, `controller/fakes.rs`, `controller/diagnostics.rs`
  (except its window title "Freight Fate - Controller Diagnostics", line 126).
- Trucking-specific: analog pedal/wheel state (`steer_target`,
  `throttle_target`, `brake_target`, `clutch`, `throttle`, `brake`,
  fields 339-347) and `on_axis` (755-778: LeftX = steering, TriggerRight =
  throttle, TriggerLeft = brake); `on_button_down/up` LeftShoulder = clutch
  (780-796); reads `steering()`, `throttle()`, `brake()`, `throttle_target()`,
  `brake_target()`, `clutch()` (842-891); `SMOOTH_RATE` smoothing in `tick`
  (821-822); `hint(action)` (638-640) and the `ff_core::input_hints` import (37);
  `NO_CONTROLLER_ENV` name. Haptics: `rumble: RumbleEngine<DeviceRumbleSink>`
  (338) with `ff_core::rumble::{RumbleEngine, RumbleSink}` (38); the engine's
  effects (`hazard`, `rumble_strip`, `hard_brake`, `impact`, `joint`) are
  trucking, but `RumbleSink` + `DeviceRumbleSink` + `to_motor` are generic.
- Suggested cut: keep a generic `ControllerManager` that tracks buttons,
  modifier, repeat, device and hot-plug, and exposes raw normalized axis values
  (with deadzone) instead of named pedals. Freight Fate layers
  steering/throttle/brake/clutch and rumble on top.

Imports and module layout (`controller.rs:33-67`):

```rust
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use ff_core::input_hints::{control_hint, CONTROLLER, KEYBOARD};
use ff_core::rumble::{RumbleEngine, RumbleSink};

use crate::states::base::InputEvent;

pub mod diagnostics;
pub mod fakes;
pub mod sdl;

// Deadzones from the design spec: 10% for the sticks, 4% for the triggers.
pub const STICK_DEADZONE: f64 = 0.10;
pub const TRIGGER_DEADZONE: f64 = 0.04;
pub const AXIS_MAX: f64 = 32767.0;

// Held D-pad left/right auto-repeat for adjusting menu options.
pub const REPEAT_DELAY_S: f64 = 0.30; // wait before the first repeat
pub const REPEAT_FAST_S: f64 = 0.03; // fastest repeat once fully accelerated
pub const REPEAT_RAMP_S: f64 = 1.50; // time held over which it accelerates

// Minimal analog smoothing: triggers and clutch reach their target quickly
// but not instantly, matching how the keyboard ramps feel today.
pub const SMOOTH_RATE: f64 = 18.0;

// Controls SDL surfaces on a fully mapped pad; a reconnect that drops these
// is the signature of the "dead triggers/shoulders" fault, so we check them.
pub const ESSENTIAL_MAPPINGS: [&str; 4] = [
    "lefttrigger",
    "righttrigger",
    "leftshoulder",
    "rightshoulder",
];
```

### Types, free functions, RepeatTimer, device seam (`controller.rs:69-314`)

```rust
/// `SDL_GameControllerButton`, the buttons any recognized pad is mapped onto.
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
}

/// `SDL_GameControllerAxis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControllerAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    TriggerLeft,
    TriggerRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControllerAction {
    MenuUp,
    MenuDown,
    AdjustLeft,
    AdjustRight,
    Confirm,
    Back,
    Help,
}

/// `_BUTTON_LABELS`: the spoken name of a button, "a button" for the rest.
pub fn button_label(button: ControllerButton) -> &'static str {
    match button {
        ControllerButton::A => "A button",
        ControllerButton::B => "B button",
        ControllerButton::X => "X button",
        ControllerButton::Y => "Y button",
        ControllerButton::LeftShoulder => "left bumper",
        ControllerButton::RightShoulder => "right bumper",
        ControllerButton::LeftStick => "left stick click",
        ControllerButton::RightStick => "right stick click",
        ControllerButton::DPadUp => "D-pad up",
        ControllerButton::DPadDown => "D-pad down",
        ControllerButton::DPadLeft => "D-pad left",
        ControllerButton::DPadRight => "D-pad right",
        ControllerButton::Back => "Back button",
        ControllerButton::Start => "Start button",
        _ => "a button",
    }
}

/// `_MENU_ACTIONS`: the semantic menu action of a button.
pub fn menu_action_for(button: ControllerButton) -> Option<ControllerAction> {
    match button {
        ControllerButton::DPadUp => Some(ControllerAction::MenuUp),
        ControllerButton::DPadDown => Some(ControllerAction::MenuDown),
        ControllerButton::DPadLeft => Some(ControllerAction::AdjustLeft),
        ControllerButton::DPadRight => Some(ControllerAction::AdjustRight),
        ControllerButton::A => Some(ControllerAction::Confirm),
        ControllerButton::B => Some(ControllerAction::Back),
        ControllerButton::Back => Some(ControllerAction::Help),
        _ => None,
    }
}

/// Rescale a normalized axis so it is 0 inside the deadzone and reaches
/// full range at the edge, avoiding a sudden jump just past the threshold.
pub fn deadzone(value: f64, dead: f64) -> f64 {
    let magnitude = value.abs();
    if magnitude <= dead {
        return 0.0;
    }
    let scaled = (magnitude - dead) / (1.0 - dead);
    if value >= 0.0 {
        scaled
    } else {
        -scaled
    }
}

/// `ControllerManager._smooth`.
pub fn smooth(current: f64, target: f64, dt: f64) -> f64 {
    current + (target - current) * (SMOOTH_RATE * dt).min(1.0)
}

/// The held D-pad left/right auto-repeat clock, on its own so the timing is
/// testable without a manager.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RepeatTimer {
    button: Option<ControllerButton>,
    countdown: f64,
    held: f64,
}

impl RepeatTimer {
    pub fn button(&self) -> Option<ControllerButton> {
        self.button
    }

    /// A D-pad left/right press starts the clock; other buttons are ignored.
    pub fn press(&mut self, button: ControllerButton) {
        if matches!(
            button,
            ControllerButton::DPadLeft | ControllerButton::DPadRight
        ) {
            self.button = Some(button);
            self.countdown = REPEAT_DELAY_S;
            self.held = 0.0;
        }
    }

    pub fn release(&mut self, button: ControllerButton) {
        if self.button == Some(button) {
            self.button = None;
        }
    }

    pub fn clear(&mut self) {
        self.button = None;
    }

    /// Advance `dt`; how many repeats fire this frame (the interval shrinks
    /// from `REPEAT_DELAY_S` to `REPEAT_FAST_S` over `REPEAT_RAMP_S` held).
    pub fn tick(&mut self, dt: f64) -> u32 {
        if self.button.is_none() {
            return 0;
        }
        self.held += dt;
        self.countdown -= dt;
        let mut count = 0;
        while self.countdown <= 0.0 {
            count += 1;
            let fraction = (self.held / REPEAT_RAMP_S).min(1.0);
            let interval = REPEAT_DELAY_S - (REPEAT_DELAY_S - REPEAT_FAST_S) * fraction;
            self.countdown += interval;
        }
        count
    }
}

// -- the device seam -------------------------------------------------------------

/// One opened game controller (`pygame._sdl2.controller.Controller`).
pub trait PadDevice {
    /// `Controller.id` right after open -- possibly stale after a hot-plug.
    fn instance_id(&self) -> u32;
    /// Whether the pad is still physically present. The SDL implementation
    /// asks the device; a stub answers `true` so a spurious ADDED event does
    /// not drop a working binding.
    fn attached(&self) -> bool {
        true
    }
    /// Drive both motors (0..65535) for `duration_ms`.
    fn rumble(&mut self, low: u16, high: u16, duration_ms: u32);
    fn stop_rumble(&mut self);
    /// The SDL mapping string, when the device can report it.
    fn mapping(&self) -> Option<String> {
        None
    }
}

/// The controller subsystem (`pygame._sdl2.controller` module).
pub trait PadSubsystem {
    fn count(&self) -> u32;
    fn is_controller(&self, index: u32) -> bool;
    fn name_for_index(&self, index: u32) -> Option<String>;
    fn open(&self, index: u32) -> Option<Box<dyn PadDevice>>;
}

/// Brings the subsystem up on demand; `None` when it is unavailable.
pub type PadSubsystemFactory = Box<dyn FnMut() -> Option<Box<dyn PadSubsystem>>>;

/// The bound pad and the two switches every device call is guarded by,
/// shared between the manager and the rumble sink.
/// Set (non-empty) to keep every gamepad out of the game for this process,
/// whatever the controller setting says. The agent server sets it: an
/// agent's capabilities are a player's KEYS, and the Xbox pad resting on
/// the operator's desk spent a whole afternoon toggling the parking brake,
/// stepping the cruise target and reading the speed into an agent's drive
/// (2026-09-01) before anyone thought of it.
pub const NO_CONTROLLER_ENV: &str = "FREIGHT_FATE_NO_CONTROLLER";

/// Whether pads may drive, from the player's setting and the override.
pub fn pads_allowed(setting_enabled: bool, override_value: Option<&std::ffi::OsStr>) -> bool {
    setting_enabled && !override_value.is_some_and(|v| !v.is_empty())
}

struct PadSlot {
    device: Option<Box<dyn PadDevice>>,
    enabled: bool,
    haptics: bool,
}

impl PadSlot {
    fn active(&self) -> bool {
        self.enabled && self.device.is_some()
    }
}

/// The haptics device layer: `_device_rumble` / `_device_stop_rumble`,
/// guarded by "active and haptics enabled".
pub struct DeviceRumbleSink {
    slot: Rc<RefCell<PadSlot>>,
}

impl RumbleSink for DeviceRumbleSink {
    fn send(&mut self, low: f64, high: f64, duration_ms: i32) {
        let mut slot = self.slot.borrow_mut();
        if !(slot.active() && slot.haptics) {
            return;
        }
        if let Some(device) = slot.device.as_mut() {
            device.rumble(to_motor(low), to_motor(high), duration_ms.max(0) as u32);
        }
    }

    fn stop(&mut self) {
        let mut slot = self.slot.borrow_mut();
        if let Some(device) = slot.device.as_mut() {
            device.stop_rumble();
        }
    }
}

/// 0..1 motor level to SDL's 16-bit range.
pub fn to_motor(level: f64) -> u16 {
    (level.clamp(0.0, 1.0) * 65535.0).round() as u16
}
```

### ControllerManager (`controller.rs:316-911`, verbatim)

```rust
/// Owns the active controller and translates its events for the game.
pub struct ControllerManager {
    slot: Rc<RefCell<PadSlot>>,
    /// Which device the player last used (`active_device`).
    pub active_device: &'static str,
    instance_id: Option<u32>,
    name: String,
    disconnected: bool, // latched until the app consumes it
    // A freshly opened pad reports a device id that a hot-plug can leave
    // stale (the events then arrive under a different id). We treat the
    // opened id as provisional and adopt the id of the first real event we
    // see, so the binding always matches the live event stream.
    id_pending: bool,
    // Buttons currently held, so a duplicate press (some pads/drivers
    // deliver a button twice) fires its action only once, on the genuine
    // transition.
    buttons_down: HashSet<ControllerButton>,
    // Last "foreign" instance id we logged a dropped event for, so a stream
    // of events from a stale/other pad logs once rather than every frame.
    logged_mismatch_id: Option<u32>,
    /// Haptics live in their own SDL-free engine; we only supply the guarded
    /// device send/stop and drive it once per frame in `tick()`.
    pub rumble: RumbleEngine<DeviceRumbleSink>,
    // Raw (event) and smoothed (tick) analog targets. The clutch is a
    // digital bumper, so it stays instant like the keyboard Shift and is not
    // smoothed.
    steer_target: f64,
    throttle_target: f64,
    brake_target: f64,
    clutch: f64,
    throttle: f64,
    brake: f64,
    /// Right bumper held -> secondary bindings.
    pub modifier: bool,
    repeat: RepeatTimer,
    // Defer touching the SDL controller subsystem until support is on, so a
    // player with controllers disabled never enumerates or binds a pad. It
    // comes up lazily on the first enable (see set_enabled/init_subsystem).
    subsystem: Option<Box<dyn PadSubsystem>>,
    factory: Option<PadSubsystemFactory>,
}

impl ControllerManager {
    /// A manager over a subsystem factory (`None` = no subsystem on this
    /// machine: keyboard only, as under the dummy drivers).
    pub fn new(enabled: bool, haptics: bool, factory: Option<PadSubsystemFactory>) -> Self {
        let slot = Rc::new(RefCell::new(PadSlot {
            device: None,
            enabled,
            haptics,
        }));
        let mut manager = Self {
            slot: Rc::clone(&slot),
            active_device: KEYBOARD,
            instance_id: None,
            name: String::new(),
            disconnected: false,
            id_pending: false,
            buttons_down: HashSet::new(),
            logged_mismatch_id: None,
            rumble: RumbleEngine::new(DeviceRumbleSink { slot }),
            steer_target: 0.0,
            throttle_target: 0.0,
            brake_target: 0.0,
            clutch: 0.0,
            throttle: 0.0,
            brake: 0.0,
            modifier: false,
            repeat: RepeatTimer::default(),
            subsystem: None,
            factory,
        };
        if enabled {
            manager.init_subsystem();
        } else {
            log::debug!("Controller support off at startup; subsystem not initialized");
        }
        manager
    }

    /// A manager that never reaches a subsystem (headless runs and tests).
    pub fn detached(enabled: bool, haptics: bool) -> Self {
        Self::new(enabled, haptics, None)
    }

    // -- device lifecycle -----------------------------------------------------

    /// Initialize the controller subsystem and bind the first pad.
    ///
    /// Idempotent and headless-safe: a failure leaves the subsystem absent
    /// (keyboard only), and a repeat call while already initialized is a
    /// no-op.
    fn init_subsystem(&mut self) {
        if self.subsystem.is_some() {
            return;
        }
        let Some(factory) = self.factory.as_mut() else {
            return;
        };
        match factory() {
            Some(subsystem) => {
                self.subsystem = Some(subsystem);
                log::debug!("Controller subsystem initialized");
                self.open_first();
            }
            None => log::info!("Controller subsystem unavailable; keyboard only"),
        }
    }

    /// Silence and release the pad, then uninitialize the subsystem so no
    /// controller handles or SDL state remain while support is off.
    fn teardown_subsystem(&mut self) {
        self.rumble.reset(); // silence the pad before we drop it
        self.close_controller();
        if self.subsystem.take().is_some() {
            log::debug!("Controller subsystem torn down");
        }
    }

    /// Whether the subsystem is up (`_sdl is not None`).
    pub fn subsystem_up(&self) -> bool {
        self.subsystem.is_some()
    }

    pub fn connected(&self) -> bool {
        self.slot.borrow().device.is_some()
    }

    /// True when a controller is connected and controller input is on.
    pub fn active(&self) -> bool {
        self.slot.borrow().active()
    }

    pub fn enabled(&self) -> bool {
        self.slot.borrow().enabled
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The bound instance id, `None` when nothing is bound.
    pub fn instance_id(&self) -> Option<u32> {
        self.instance_id
    }

    /// Whether the bound id is still provisional (see `id_pending`).
    pub fn id_pending(&self) -> bool {
        self.id_pending
    }

    /// Test seam: mark the bound id latched or provisional.
    pub fn set_id_pending(&mut self, pending: bool) {
        self.id_pending = pending;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.slot.borrow_mut().enabled = enabled;
        if enabled {
            self.init_subsystem(); // bring the subsystem up on first enable
        } else {
            self.teardown_subsystem(); // close handles + uninit while off
            self.reset_analog();
            self.active_device = KEYBOARD;
        }
    }

    pub fn set_haptics_enabled(&mut self, enabled: bool) {
        self.slot.borrow_mut().haptics = enabled;
        if !enabled {
            self.rumble.reset();
        }
    }

    /// Bind `device` directly (the test seam `m._controller = object()`, and
    /// what `open_index` does once the subsystem handed a pad over).
    pub fn bind_device(&mut self, device: Box<dyn PadDevice>, name: &str) {
        self.instance_id = Some(device.instance_id());
        // Provisional: the first real event's instance id is authoritative
        // (see id_pending), which self-heals a stale id left by a hot-plug.
        self.id_pending = true;
        self.name = if name.is_empty() {
            "controller".to_string()
        } else {
            name.to_string()
        };
        log::info!(
            "Controller connected: {} (instance {:?})",
            self.name,
            self.instance_id
        );
        let mapping = device.mapping();
        self.slot.borrow_mut().device = Some(device);
        self.log_capabilities(mapping.as_deref());
    }

    /// Bind the GameController at `index`; true on success.
    fn open_index(&mut self, index: u32) -> bool {
        let (device, name) = {
            let Some(subsystem) = self.subsystem.as_ref() else {
                return false;
            };
            if !subsystem.is_controller(index) {
                return false;
            }
            let Some(device) = subsystem.open(index) else {
                log::debug!("Could not open controller at slot {index}");
                return false;
            };
            (device, subsystem.name_for_index(index).unwrap_or_default())
        };
        self.bind_device(device, &name);
        true
    }

    fn open_first(&mut self) {
        let count = match self.subsystem.as_ref() {
            Some(subsystem) => subsystem.count(),
            None => return,
        };
        for index in 0..count {
            if self.open_index(index) {
                return;
            }
        }
    }

    /// DEBUG snapshot of the bound pad's mapping, plus a WARNING if the
    /// trigger and shoulder controls are unmapped -- the fingerprint of the
    /// reconnect bug where those inputs stop responding while the rest of
    /// the pad works.
    fn log_capabilities(&self, mapping: Option<&str>) {
        let Some(mapping) = mapping.filter(|m| !m.is_empty()) else {
            return;
        };
        let keys = mapping_keys(mapping);
        log::debug!("Controller {:?} mapping keys: {:?}", self.name, keys);
        let missing: Vec<&str> = ESSENTIAL_MAPPINGS
            .iter()
            .copied()
            .filter(|name| !keys.iter().any(|k| k == name))
            .collect();
        if !missing.is_empty() {
            log::warn!(
                "Controller {:?} reconnected without {} mapped; \
                 brake/throttle and bumpers may not respond",
                self.name,
                missing.join(", ")
            );
        }
    }

    /// Release the controller before dropping our reference, so its
    /// bookkeeping is torn down cleanly across a hot-plug.
    fn close_controller(&mut self) {
        self.slot.borrow_mut().device = None;
        self.instance_id = None;
        self.id_pending = false;
        self.buttons_down.clear();
    }

    /// Whether the bound pad is still physically present.
    fn is_attached(&self) -> bool {
        self.slot
            .borrow()
            .device
            .as_ref()
            .is_some_and(|device| device.attached())
    }

    /// Drop the current pad and bind whatever is connected now.
    ///
    /// The first real event's instance id (see `id_pending`) corrects a
    /// stale id left by a hot-plug, so we never cycle the SDL subsystem:
    /// quitting and re-initializing it while the event loop is live
    /// re-registers SDL's controller event watch and makes every controller
    /// event arrive twice.
    pub fn reopen(&mut self) {
        self.close_controller();
        self.open_first();
    }

    /// Return and clear the pending-disconnect flag (app pauses on True).
    pub fn take_disconnect(&mut self) -> bool {
        if self.disconnected {
            self.disconnected = false;
            return true;
        }
        false
    }

    fn reset_analog(&mut self) {
        self.steer_target = 0.0;
        self.throttle_target = 0.0;
        self.brake_target = 0.0;
        self.clutch = 0.0;
        self.throttle = 0.0;
        self.brake = 0.0;
        self.modifier = false;
        self.repeat.clear();
        self.buttons_down.clear();
        self.rumble.reset();
    }

    // -- input notification ---------------------------------------------------

    /// Record that the keyboard was just used, so hints name keys.
    pub fn note_keyboard(&mut self) {
        self.active_device = KEYBOARD;
    }

    /// `"controller"` when a pad is active and was used last, else
    /// `"keyboard"`.
    pub fn device(&self) -> &'static str {
        if self.active() && self.active_device == CONTROLLER {
            CONTROLLER
        } else {
            KEYBOARD
        }
    }

    /// Control phrase for `action` naming whatever device is in use.
    pub fn hint(&self, action: &str) -> String {
        control_hint(action, self.device())
    }

    // -- event handling -------------------------------------------------------

    /// Update device/axis/modifier state from a `CONTROLLER*` event.
    ///
    /// Returns true only for a button press/release that belongs to the
    /// bound controller and should be forwarded to the active state. Axis,
    /// device, and rejected (foreign-id) events return false, so the app
    /// never routes a stray event -- e.g. a duplicate from a pad that
    /// enumerates twice -- into a game action.
    pub fn process_event(&mut self, event: &InputEvent) -> bool {
        let instance_id = match event {
            InputEvent::ControllerAdded { device_index } => {
                log::debug!("Controller device added (slot {device_index})");
                // Re-open when nothing is bound, or the bound pad has quietly
                // detached (a missed REMOVED event) -- otherwise a stale
                // binding would block the reconnect.
                if self.connected() && self.is_attached() {
                    log::debug!(
                        "Ignoring add; a pad is already bound (instance {:?})",
                        self.instance_id
                    );
                    return false;
                }
                // Re-open in place. The first real event's id (see
                // id_pending) corrects a stale id from the hot-plug; we
                // deliberately do not cycle the SDL subsystem, which would
                // double every controller event.
                self.reopen();
                return false;
            }
            InputEvent::ControllerRemoved { instance_id } => {
                log::debug!("Controller device removed (instance {instance_id})");
                if self.instance_id.is_some() && Some(*instance_id) == self.instance_id {
                    self.close_controller();
                    self.reset_analog();
                    self.disconnected = true;
                    self.logged_mismatch_id = None;
                    log::info!("Controller disconnected");
                }
                return false;
            }
            InputEvent::ControllerButtonDown { instance_id, .. }
            | InputEvent::ControllerButtonUp { instance_id, .. }
            | InputEvent::ControllerAxis { instance_id, .. } => *instance_id,
            _ => return false,
        };
        if !self.active() {
            return false;
        }
        if Some(instance_id) != self.instance_id {
            if self.id_pending {
                // First real event after (re)open: its id is authoritative,
                // so a stale id from the open after a hot-plug is corrected
                // here.
                log::info!(
                    "Controller bound to instance {instance_id} (opened as {:?})",
                    self.instance_id
                );
                self.instance_id = Some(instance_id);
                self.id_pending = false;
                self.logged_mismatch_id = None;
            } else {
                // A genuinely foreign id: a second pad, or a duplicate from
                // a pad that enumerates twice. Drop it (log once) so it never
                // doubles a toggle or other action.
                if Some(instance_id) != self.logged_mismatch_id {
                    self.logged_mismatch_id = Some(instance_id);
                    log::debug!(
                        "Dropping controller event from instance {instance_id} (bound to {:?})",
                        self.instance_id
                    );
                }
                return false;
            }
        } else {
            // Matched the bound id, so it is live: stop treating the id as
            // provisional (a duplicate under a different id is now a
            // duplicate).
            self.id_pending = false;
        }
        match event {
            InputEvent::ControllerAxis { axis, value, .. } => {
                self.on_axis(*axis, *value);
                false
            }
            InputEvent::ControllerButtonDown { button, .. } => {
                if self.buttons_down.contains(button) {
                    // Duplicate press with no intervening release: ignore it
                    // so a pad that delivers a button twice cannot fire the
                    // action twice.
                    log::debug!(
                        "Ignoring duplicate button-down {button:?} (instance {instance_id})"
                    );
                    return false;
                }
                self.buttons_down.insert(*button);
                self.active_device = CONTROLLER;
                log::debug!("Button down {button:?} (instance {instance_id})");
                self.on_button_down(*button);
                true
            }
            InputEvent::ControllerButtonUp { button, .. } => {
                if !self.buttons_down.remove(button) {
                    return false; // stray/duplicate release
                }
                log::debug!("Button up {button:?} (instance {instance_id})");
                self.on_button_up(*button);
                true
            }
            _ => false,
        }
    }

    fn on_axis(&mut self, axis: ControllerAxis, raw: i16) {
        let value = raw as f64 / AXIS_MAX;
        match axis {
            ControllerAxis::LeftX => {
                self.steer_target = deadzone(value, STICK_DEADZONE);
                if self.steer_target != 0.0 {
                    self.active_device = CONTROLLER;
                }
            }
            ControllerAxis::TriggerRight => {
                self.throttle_target = deadzone(value.max(0.0), TRIGGER_DEADZONE);
                if self.throttle_target != 0.0 {
                    self.active_device = CONTROLLER;
                }
            }
            ControllerAxis::TriggerLeft => {
                self.brake_target = deadzone(value.max(0.0), TRIGGER_DEADZONE);
                if self.brake_target != 0.0 {
                    self.active_device = CONTROLLER;
                }
            }
            _ => {}
        }
    }

    fn on_button_down(&mut self, button: ControllerButton) {
        match button {
            ControllerButton::RightShoulder => self.modifier = true,
            ControllerButton::LeftShoulder => self.clutch = 1.0, // instant, like holding Shift
            _ => {}
        }
        self.repeat.press(button);
    }

    fn on_button_up(&mut self, button: ControllerButton) {
        match button {
            ControllerButton::RightShoulder => self.modifier = false,
            ControllerButton::LeftShoulder => self.clutch = 0.0,
            _ => {}
        }
        self.repeat.release(button);
    }

    /// The semantic menu action for a `ControllerButtonDown`, or None.
    pub fn menu_action(&self, event: &InputEvent) -> Option<ControllerAction> {
        match event {
            InputEvent::ControllerButtonDown { button, .. } => menu_action_for(*button),
            _ => None,
        }
    }

    pub fn button_label(&self, button: ControllerButton) -> &'static str {
        button_label(button)
    }

    // -- per-frame update -----------------------------------------------------

    /// Smooth analog axes and return synthetic D-pad repeat events.
    pub fn tick(&mut self, dt: f64) -> Vec<InputEvent> {
        // Always drive the rumble engine so queued effects decay and expire
        // even when no controller is bound; the device send is guarded
        // either way.
        self.rumble.tick(dt);
        if !self.active() {
            return Vec::new();
        }
        self.throttle = smooth(self.throttle, self.throttle_target, dt);
        self.brake = smooth(self.brake, self.brake_target, dt);
        self.repeats(dt)
    }

    fn repeats(&mut self, dt: f64) -> Vec<InputEvent> {
        let Some(button) = self.repeat.button() else {
            return Vec::new();
        };
        let count = self.repeat.tick(dt);
        let instance_id = self.instance_id.unwrap_or(0);
        (0..count)
            .map(|_| InputEvent::ControllerButtonDown {
                button,
                instance_id,
            })
            .collect()
    }

    // -- analog reads for the driving loop ------------------------------------

    pub fn steering(&self) -> f64 {
        if self.active() {
            self.steer_target
        } else {
            0.0
        }
    }

    pub fn throttle(&self) -> f64 {
        if self.active() {
            self.throttle
        } else {
            0.0
        }
    }

    pub fn brake(&self) -> f64 {
        if self.active() {
            self.brake
        } else {
            0.0
        }
    }

    /// Instantaneous trigger position, before smoothing. Press/release
    /// detection (e.g. the reverse shift gesture) must read this so a quick
    /// tap registers -- the smoothed reads above lag behind the trigger.
    pub fn throttle_target(&self) -> f64 {
        if self.active() {
            self.throttle_target
        } else {
            0.0
        }
    }

    pub fn brake_target(&self) -> f64 {
        if self.active() {
            self.brake_target
        } else {
            0.0
        }
    }

    pub fn clutch(&self) -> f64 {
        if self.active() {
            self.clutch
        } else {
            0.0
        }
    }

    pub fn shutdown(&mut self) {
        self.teardown_subsystem();
        // The production factory captures an SDL handle so controller support
        // can be enabled later. Final shutdown has no later: release that
        // handle before the window begins its exit path.
        self.factory = None;
    }
}

/// The field names of an SDL mapping string (`"a:b0,b:b1,..."`), without
/// the leading GUID and name fields.
pub fn mapping_keys(mapping: &str) -> Vec<String> {
    mapping
        .split(',')
        .filter_map(|field| field.split_once(':'))
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, _)| key.trim().to_string())
        .collect()
}
```

### `controller/sdl.rs` (verbatim, 161 lines)

```rust
//! The SDL side of the controller seam: [`PadDevice`] / [`PadSubsystem`]
//! over `sdl2::controller`, the factory the app hands the manager, and the
//! conversions between SDL's enums and the game's.

use sdl2::controller::{Axis, Button, GameController};
use sdl2::GameControllerSubsystem;

use super::{ControllerAxis, ControllerButton, PadDevice, PadSubsystem, PadSubsystemFactory};

/// An opened `GameController`. Dropping it closes the device, which is what
/// `Controller.quit()` did.
pub struct SdlPad {
    controller: GameController,
}

impl PadDevice for SdlPad {
    fn instance_id(&self) -> u32 {
        self.controller.instance_id()
    }

    fn attached(&self) -> bool {
        self.controller.attached()
    }

    fn rumble(&mut self, low: u16, high: u16, duration_ms: u32) {
        // Driver dependent: a pad without motors answers with an error,
        // which is not worth a log line per frame.
        let _ = self.controller.set_rumble(low, high, duration_ms);
    }

    fn stop_rumble(&mut self) {
        let _ = self.controller.set_rumble(0, 0, 0);
    }

    fn mapping(&self) -> Option<String> {
        let mapping = self.controller.mapping();
        (!mapping.is_empty()).then_some(mapping)
    }
}

/// The game controller subsystem. Dropping it quits the subsystem
/// (`sdl_controller.quit()`).
pub struct SdlPadSubsystem {
    subsystem: GameControllerSubsystem,
}

impl SdlPadSubsystem {
    pub fn new(subsystem: GameControllerSubsystem) -> Self {
        Self { subsystem }
    }
}

impl PadSubsystem for SdlPadSubsystem {
    fn count(&self) -> u32 {
        self.subsystem.num_joysticks().unwrap_or(0)
    }

    fn is_controller(&self, index: u32) -> bool {
        self.subsystem.is_game_controller(index)
    }

    fn name_for_index(&self, index: u32) -> Option<String> {
        self.subsystem.name_for_index(index).ok()
    }

    fn open(&self, index: u32) -> Option<Box<dyn PadDevice>> {
        match self.subsystem.open(index) {
            Ok(controller) => Some(Box::new(SdlPad { controller })),
            Err(e) => {
                log::debug!("Could not open controller at slot {index}: {e}");
                None
            }
        }
    }
}

/// A factory that brings the SDL game controller subsystem up on demand.
/// Every failure (no SDL, dummy drivers) is "no subsystem": keyboard only.
pub fn sdl_factory(sdl: sdl2::Sdl) -> PadSubsystemFactory {
    Box::new(move || match sdl.game_controller() {
        Ok(subsystem) => Some(Box::new(SdlPadSubsystem::new(subsystem)) as Box<dyn PadSubsystem>),
        Err(e) => {
            log::info!("Controller subsystem unavailable; keyboard only: {e}");
            None
        }
    })
}

pub fn button_from_sdl(button: Button) -> ControllerButton {
    match button {
        Button::A => ControllerButton::A,
        Button::B => ControllerButton::B,
        Button::X => ControllerButton::X,
        Button::Y => ControllerButton::Y,
        Button::Back => ControllerButton::Back,
        Button::Guide => ControllerButton::Guide,
        Button::Start => ControllerButton::Start,
        Button::LeftStick => ControllerButton::LeftStick,
        Button::RightStick => ControllerButton::RightStick,
        Button::LeftShoulder => ControllerButton::LeftShoulder,
        Button::RightShoulder => ControllerButton::RightShoulder,
        Button::DPadUp => ControllerButton::DPadUp,
        Button::DPadDown => ControllerButton::DPadDown,
        Button::DPadLeft => ControllerButton::DPadLeft,
        Button::DPadRight => ControllerButton::DPadRight,
        Button::Misc1 => ControllerButton::Misc1,
        Button::Paddle1 => ControllerButton::Paddle1,
        Button::Paddle2 => ControllerButton::Paddle2,
        Button::Paddle3 => ControllerButton::Paddle3,
        Button::Paddle4 => ControllerButton::Paddle4,
        Button::Touchpad => ControllerButton::Touchpad,
    }
}

pub fn button_to_sdl(button: ControllerButton) -> Button {
    match button {
        ControllerButton::A => Button::A,
        ControllerButton::B => Button::B,
        ControllerButton::X => Button::X,
        ControllerButton::Y => Button::Y,
        ControllerButton::Back => Button::Back,
        ControllerButton::Guide => Button::Guide,
        ControllerButton::Start => Button::Start,
        ControllerButton::LeftStick => Button::LeftStick,
        ControllerButton::RightStick => Button::RightStick,
        ControllerButton::LeftShoulder => Button::LeftShoulder,
        ControllerButton::RightShoulder => Button::RightShoulder,
        ControllerButton::DPadUp => Button::DPadUp,
        ControllerButton::DPadDown => Button::DPadDown,
        ControllerButton::DPadLeft => Button::DPadLeft,
        ControllerButton::DPadRight => Button::DPadRight,
        ControllerButton::Misc1 => Button::Misc1,
        ControllerButton::Paddle1 => Button::Paddle1,
        ControllerButton::Paddle2 => Button::Paddle2,
        ControllerButton::Paddle3 => Button::Paddle3,
        ControllerButton::Paddle4 => Button::Paddle4,
        ControllerButton::Touchpad => Button::Touchpad,
    }
}

pub fn axis_from_sdl(axis: Axis) -> ControllerAxis {
    match axis {
        Axis::LeftX => ControllerAxis::LeftX,
        Axis::LeftY => ControllerAxis::LeftY,
        Axis::RightX => ControllerAxis::RightX,
        Axis::RightY => ControllerAxis::RightY,
        Axis::TriggerLeft => ControllerAxis::TriggerLeft,
        Axis::TriggerRight => ControllerAxis::TriggerRight,
    }
}

pub fn axis_to_sdl(axis: ControllerAxis) -> Axis {
    match axis {
        ControllerAxis::LeftX => Axis::LeftX,
        ControllerAxis::LeftY => Axis::LeftY,
        ControllerAxis::RightX => Axis::RightX,
        ControllerAxis::RightY => Axis::RightY,
        ControllerAxis::TriggerLeft => Axis::TriggerLeft,
        ControllerAxis::TriggerRight => Axis::TriggerRight,
    }
}
```

### `controller/fakes.rs` (verbatim, 132 lines)

```rust
//! Test doubles for the controller seam: a pad that records what was asked
//! of it, a subsystem that hands out such pads and counts how often it was
//! brought up or torn down (the Python tests' `object()` controller and
//! `FakeSDL`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{PadDevice, PadSubsystem, PadSubsystemFactory};

/// What a [`FakePad`] was told to do.
#[derive(Debug, Default)]
pub struct FakePadLog {
    pub rumbles: Vec<(u16, u16, u32)>,
    pub stops: u32,
}

/// A bound pad with a fixed instance id.
pub struct FakePad {
    pub instance_id: u32,
    pub attached: Rc<Cell<bool>>,
    pub mapping: Option<String>,
    pub log: Rc<RefCell<FakePadLog>>,
}

impl FakePad {
    pub fn new(instance_id: u32) -> Self {
        Self {
            instance_id,
            attached: Rc::new(Cell::new(true)),
            mapping: None,
            log: Rc::new(RefCell::new(FakePadLog::default())),
        }
    }

    pub fn with_mapping(mut self, mapping: &str) -> Self {
        self.mapping = Some(mapping.to_string());
        self
    }

    /// A handle the test keeps to flip `attached` or read the rumble log.
    pub fn handle(&self) -> FakePadHandle {
        FakePadHandle {
            attached: Rc::clone(&self.attached),
            log: Rc::clone(&self.log),
        }
    }
}

/// The test's view of a pad the manager now owns.
#[derive(Clone)]
pub struct FakePadHandle {
    pub attached: Rc<Cell<bool>>,
    pub log: Rc<RefCell<FakePadLog>>,
}

impl PadDevice for FakePad {
    fn instance_id(&self) -> u32 {
        self.instance_id
    }

    fn attached(&self) -> bool {
        self.attached.get()
    }

    fn rumble(&mut self, low: u16, high: u16, duration_ms: u32) {
        self.log.borrow_mut().rumbles.push((low, high, duration_ms));
    }

    fn stop_rumble(&mut self) {
        self.log.borrow_mut().stops += 1;
    }

    fn mapping(&self) -> Option<String> {
        self.mapping.clone()
    }
}

/// What a [`FakeSubsystem`] and its factory saw.
#[derive(Debug, Default)]
pub struct FakeSdlLog {
    /// How many times the factory brought a subsystem up (`init`).
    pub inits: u32,
    /// How many subsystems were dropped (`quit`).
    pub quits: u32,
    /// Every `open(index)` call.
    pub opens: Vec<u32>,
}

/// A subsystem with `pads` recognized controllers (name, instance id).
pub struct FakeSubsystem {
    pub pads: Vec<(String, u32)>,
    pub log: Rc<RefCell<FakeSdlLog>>,
}

impl PadSubsystem for FakeSubsystem {
    fn count(&self) -> u32 {
        self.pads.len() as u32
    }

    fn is_controller(&self, index: u32) -> bool {
        (index as usize) < self.pads.len()
    }

    fn name_for_index(&self, index: u32) -> Option<String> {
        self.pads.get(index as usize).map(|(name, _)| name.clone())
    }

    fn open(&self, index: u32) -> Option<Box<dyn PadDevice>> {
        self.log.borrow_mut().opens.push(index);
        let (_, instance_id) = self.pads.get(index as usize)?;
        Some(Box::new(FakePad::new(*instance_id)))
    }
}

impl Drop for FakeSubsystem {
    fn drop(&mut self) {
        self.log.borrow_mut().quits += 1;
    }
}

/// A factory handing out [`FakeSubsystem`]s with the given pads, recording
/// into the shared log.
pub fn fake_factory(pads: Vec<(String, u32)>, log: Rc<RefCell<FakeSdlLog>>) -> PadSubsystemFactory {
    Box::new(move || {
        log.borrow_mut().inits += 1;
        Some(Box::new(FakeSubsystem {
            pads: pads.clone(),
            log: Rc::clone(&log),
        }))
    })
}
```

### `controller/diagnostics.rs` (280 lines)

`freightfate --controller-diagnostics` (dispatched at `app.rs:1087-1089`, before
the single-instance guard): opens an SDL window, logs GameController and raw
joystick events side by side to stdout and `controller-diagnostics.log`
(`LOG_FILENAME`, line 45). Uses `sdl2::event::Event`, `sdl2::joystick::HatState`,
`chrono` for timestamps, and `super::AXIS_MAX`. Public API:

```rust
pub const LOG_FILENAME: &str = "controller-diagnostics.log";
pub fn prepare_environment() // sets SDL_JOYSTICK_HIDAPI_PS4_RUMBLE / _PS5_RUMBLE = "1" if unset (83-92)
pub fn run_controller_diagnostics() -> i32 // (95-280)
```

Only FF-specific text: window title `"Freight Fate - Controller Diagnostics"` (126).

### SDL keyboard/controller -> InputEvent translation (`app/sdl_shell.rs`, not in the task list but required)

`app/sdl_shell.rs` imports (12-23) include `crate::controller::sdl::{axis_from_sdl, button_from_sdl}`
and `crate::states::base::{InputEvent, Key, Mods}`. It pairs each SDL `KeyDown`
with the `TextInput` that follows it in the same batch (this is how
`InputEvent::KeyDown.text` is filled, which menus' first-letter jump and text
entry depend on). `mods_from` (347-353) and `key_from_keycode` (355-454, one arm
per `Key` variant, `other => Key::Other(other.into_i32())`) are generic.

```rust
/// SDL events to game events, pairing each `KeyDown` with the `TextInput`
/// that follows it.
pub fn translate_events(raw: Vec<Event>) -> Vec<InputEvent> {
    let mut out = Vec::with_capacity(raw.len());
    let mut pending_text: Option<usize> = None; // index in `out` of the last KeyDown
    for event in raw {
        match event {
            Event::KeyDown {
                keycode,
                keymod,
                repeat,
                ..
            } => {
                let key = keycode.map(key_from_keycode).unwrap_or(Key::Other(0));
                out.push(InputEvent::KeyDown {
                    key,
                    mods: mods_from(keymod),
                    text: None,
                    repeat,
                });
                pending_text = Some(out.len() - 1);
                continue;
            }
            Event::TextInput { text, .. } => {
                if let Some(index) = pending_text.take() {
                    if let Some(InputEvent::KeyDown { text: slot, .. }) = out.get_mut(index) {
                        *slot = text.chars().next();
                    }
                }
                continue;
            }
            Event::KeyUp {
                keycode, keymod, ..
            } => {
                let key = keycode.map(key_from_keycode).unwrap_or(Key::Other(0));
                out.push(InputEvent::KeyUp {
                    key,
                    mods: mods_from(keymod),
                });
            }
            Event::ControllerButtonDown { which, button, .. } => {
                out.push(InputEvent::ControllerButtonDown {
                    button: button_from_sdl(button),
                    instance_id: which,
                });
            }
            Event::ControllerButtonUp { which, button, .. } => {
                out.push(InputEvent::ControllerButtonUp {
                    button: button_from_sdl(button),
                    instance_id: which,
                });
            }
            Event::ControllerAxisMotion {
                which, axis, value, ..
            } => {
                out.push(InputEvent::ControllerAxis {
                    axis: axis_from_sdl(axis),
                    value,
                    instance_id: which,
                });
            }
            Event::ControllerDeviceAdded { which, .. } => {
                out.push(InputEvent::ControllerAdded {
                    device_index: which,
                });
            }
            Event::ControllerDeviceRemoved { which, .. } => {
                out.push(InputEvent::ControllerRemoved { instance_id: which });
            }
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } => out.push(InputEvent::WindowFocusGained),
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => out.push(InputEvent::WindowFocusLost),
            Event::Quit { .. } => out.push(InputEvent::Quit),
            _ => {}
        }
        pending_text = None;
    }
    out
}

/// `event.mod & KMOD_*`.
pub fn mods_from(keymod: Mod) -> Mods {
    Mods {
        shift: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        ctrl: keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        alt: keymod.intersects(Mod::LALTMOD | Mod::RALTMOD),
    }
}
```

## 6. Held keys (`app/held_keys.rs`, 656 lines)

### Purpose

`HeldKeys` is the `pygame.key.get_pressed()` equivalent: the set of keys the
app has seen go down and not up, plus current `Mods`, for code that polls
("is Up held right now?") rather than reacting to events. On top of that it
has a pulse layer for JAWS, which swallows arrow keys and re-sends each as a
zero-length synthetic press+release pair (plus auto-repeat pairs about 250 ms
apart). A press starts a pulse (the OS repeat delay + `FRESH_GRACE_MS`); a
repeat extends it by a learned spacing + `REPEAT_GRACE_MS`. A release in the
same frame as its press (and in a frame of at most `SYNTHETIC_FRAME_MAX_MS`)
is treated as synthetic and does not end the pulse. The pulse layer is inert
until `begin_frame` has been called at least once, so tests that feed keys
directly keep plain semantics. On Windows the OS repeat delay/interval comes
from `SystemParametersInfoW(SPI_GETKEYBOARDDELAY / SPI_GETKEYBOARDSPEED)` via
`windows-sys` (feature `Win32_UI_WindowsAndMessaging`, already in the workspace
`windows-sys` feature list at `Freight-fate/Cargo.toml:56`).

### Wiring in Freight Fate

- Field `GameContext.input: HeldKeys` (`app/context.rs:168`), built with
  `HeldKeys::default()` (`app/context.rs:269`).
- `App::handle_event` (`app.rs:740-775`): `KeyDown` -> `ctx.input.press(key, mods)`;
  `KeyUp` -> `ctx.input.release(key, mods)`; `WindowFocusLost` -> `ctx.input.clear()`;
  `WindowFocusGained` -> `ctx.input.refresh_repeat_timing()` + `ctx.input.clear_pulses()`.
- `App` frame loop calls `self.ctx.input.begin_frame(dt)` (`app.rs:906`).
- Stack changes call `self.input.clear_pulses()` (`app/context.rs:529` in
  `push_shared_with`, `app/context.rs:545` in `take_top`), so a new screen
  never inherits the previous screen's hold.
- Readers: `KeyBindings::pressed(&HeldKeys, Action)` (`bindings.rs:825-833`),
  used by the driving state to poll pedals/steering.

### Couplings

Only `use crate::states::base::{Key, Mods};` (line 57) and `windows_sys`
(108-110, `cfg(windows)`). No `GameContext`, no trucking. The module doc
mentions pedals/driving but the code is generic.

### Public API, verbatim (`app/held_keys.rs:55-361`)

```rust
use std::collections::{HashMap, HashSet};

use crate::states::base::{Key, Mods};

/// Fallbacks when the operating system will not say: the Windows defaults
/// (delay setting 1 of 0..3 is 500 ms; rate setting 31 of 0..31 is about 30
/// repeats per second).
pub const DEFAULT_REPEAT_DELAY_MS: u64 = 500;
pub const DEFAULT_REPEAT_INTERVAL_MS: u64 = 33;

/// Grace on top of the measured timing. The fresh-press pulse has to outlast
/// the repeat delay plus the screen reader's script latency and a frame of
/// batching; the per-repeat pulse has to outlast one spacing plus jitter
/// (about 30 ms measured) and a frame, and it is how long after the finger
/// lifts the key still reads held, so it stays short.
pub const FRESH_GRACE_MS: u64 = 150;
pub const REPEAT_GRACE_MS: u64 = 100;

/// A press and release in one frame is synthetic only when the frame was a
/// normal one. After a long hitch a whole real tap can land in one batch,
/// and then the honest answer is "not held", never a half-second hold. A
/// re-injected pair dropped this way costs nothing: the next repeat pair
/// re-establishes the hold a frame later.
pub const SYNTHETIC_FRAME_MAX_MS: u64 = 40;

/// A press this soon after its pulse train began cannot be the operating
/// system's first auto-repeat (which never fires early), so it is a new tap
/// of the same key and earns a fresh full pulse.
pub const REPEAT_EARLY_TOLERANCE_MS: u64 = 60;

/// The learned repeat spacing is the largest of this many recent spacings,
/// so one slow script run widens the window at once and a run of quick ones
/// narrows it again only once it has aged out.
pub const LEARNED_SPACINGS: usize = 8;

/// Windows keyboard delay setting (0..3) to milliseconds before the first
/// auto-repeat: 250 ms per step, so 0 is 250 ms and 3 is a second.
pub fn repeat_delay_ms(setting: u32) -> u64 {
    (u64::from(setting.min(3)) + 1) * 250
}

/// Windows keyboard speed setting (0..31) to milliseconds between
/// auto-repeats: 0 is about 2.5 repeats per second, 31 about 30.
pub fn repeat_interval_ms(setting: u32) -> u64 {
    let rate = 2.5 + 27.5 * f64::from(setting.min(31)) / 31.0;
    (1000.0 / rate).round() as u64
}

/// `(delay, interval)` in ms of the keyboard auto-repeat, from Windows when
/// it answers, else the defaults. Only Windows has JAWS, and only Windows
/// exposes the setting this cheaply, so nothing else is asked.
#[cfg(windows)]
pub fn os_repeat_timing() -> (u64, u64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED,
    };
    let mut delay: u32 = 0;
    let mut speed: u32 = 0;
    // SAFETY: each call writes exactly one u32 into the local it is handed
    // and touches nothing else; the locals outlive the calls.
    let got_delay = unsafe {
        SystemParametersInfoW(SPI_GETKEYBOARDDELAY, 0, (&mut delay as *mut u32).cast(), 0)
    } != 0;
    let got_speed = unsafe {
        SystemParametersInfoW(SPI_GETKEYBOARDSPEED, 0, (&mut speed as *mut u32).cast(), 0)
    } != 0;
    (
        if got_delay {
            repeat_delay_ms(delay)
        } else {
            DEFAULT_REPEAT_DELAY_MS
        },
        if got_speed {
            repeat_interval_ms(speed)
        } else {
            DEFAULT_REPEAT_INTERVAL_MS
        },
    )
}

#[cfg(not(windows))]
pub fn os_repeat_timing() -> (u64, u64) {
    (DEFAULT_REPEAT_DELAY_MS, DEFAULT_REPEAT_INTERVAL_MS)
}

/// `pygame.key.get_pressed()` / `get_mods()`: the keys currently held, as
/// the app tracks them from the key events it dispatched, plus the pulse
/// layer that reads a screen reader's re-injected press-and-release train as
/// the hold it stands for (see the module docs).
#[derive(Debug)]
pub struct HeldKeys {
    held: HashSet<Key>,
    mods: Mods,
    delay_ms: u64,
    interval_ms: u64,
    /// Frames the app has clocked. Zero means no frame loop is running, and
    /// then the pulse layer stays out of the way entirely.
    frame: u64,
    now_ms: u64,
    frame_span_ms: u64,
    pulse_until: HashMap<Key, u64>,
    train_start: HashMap<Key, u64>,
    train_repeats: HashMap<Key, u32>,
    pressed_at: HashMap<Key, u64>,
    /// The frame of the last press that came from the released state: the
    /// only kind of press a synthetic pair can begin with.
    pair_frame: HashMap<Key, u64>,
    last_pair_synthetic: HashSet<Key>,
    spacings: Vec<u64>,
}

impl Default for HeldKeys {
    fn default() -> Self {
        let (delay, interval) = os_repeat_timing();
        Self::with_timing(delay, interval)
    }
}

impl HeldKeys {
    /// A tracker with a known repeat delay and interval (tests; the default
    /// asks the operating system).
    pub fn with_timing(repeat_delay_ms: u64, repeat_interval_ms: u64) -> Self {
        Self {
            held: HashSet::new(),
            mods: Mods::NONE,
            delay_ms: repeat_delay_ms,
            interval_ms: repeat_interval_ms,
            frame: 0,
            now_ms: 0,
            frame_span_ms: 0,
            pulse_until: HashMap::new(),
            train_start: HashMap::new(),
            train_repeats: HashMap::new(),
            pressed_at: HashMap::new(),
            pair_frame: HashMap::new(),
            last_pair_synthetic: HashSet::new(),
            spacings: Vec::new(),
        }
    }

    // -- reading ------------------------------------------------------------------

    /// Physically down, or held by a live pulse from a re-injected train.
    pub fn is_pressed(&self, key: Key) -> bool {
        self.held.contains(&key) || self.pulsed(key)
    }

    /// Held only by a pulse, not by SDL's own key state.
    pub fn pulsed(&self, key: Key) -> bool {
        self.pulse_until
            .get(&key)
            .is_some_and(|&until| until > self.now_ms)
    }

    pub fn mods(&self) -> Mods {
        self.mods
    }

    // -- timing -------------------------------------------------------------------

    pub fn repeat_delay_ms(&self) -> u64 {
        self.delay_ms
    }

    pub fn repeat_interval_ms(&self) -> u64 {
        self.interval_ms
    }

    /// The spacing re-injected repeats are actually arriving at, once seen.
    pub fn learned_spacing_ms(&self) -> Option<u64> {
        self.spacings.iter().copied().max()
    }

    /// How long a lone press reads held: to the first auto-repeat, plus grace.
    pub fn fresh_pulse_ms(&self) -> u64 {
        self.delay_ms + FRESH_GRACE_MS
    }

    /// How long each repeat extends the hold: one spacing, plus grace.
    ///
    /// The spacing is the learned one when there is one (never shorter than
    /// the operating system's own rate); before anything is learned it is
    /// the fresh pulse, so the first hold of a session cannot stutter.
    pub fn repeat_pulse_ms(&self) -> u64 {
        match self.learned_spacing_ms() {
            Some(learned) => learned.max(self.interval_ms) + REPEAT_GRACE_MS,
            None => self.fresh_pulse_ms(),
        }
    }

    /// Re-read the operating system's repeat timing (on window focus, so a
    /// player who changed the keyboard settings gets them without a restart).
    pub fn refresh_repeat_timing(&mut self) {
        let (delay, interval) = os_repeat_timing();
        self.delay_ms = delay;
        self.interval_ms = interval;
    }

    // -- feeding ------------------------------------------------------------------

    /// Once per frame from the app's loop, before that frame's events, with
    /// the frame's real duration. This is the clock the pulses run on; until
    /// it has ticked, presses and releases keep their plain meaning.
    pub fn begin_frame(&mut self, dt_seconds: f64) {
        let span = (dt_seconds.max(0.0) * 1000.0).round() as u64;
        self.frame += 1;
        self.frame_span_ms = span;
        self.now_ms += span;
    }

    pub fn press(&mut self, key: Key, mods: Mods) {
        let was_held = !self.held.insert(key);
        self.mods = mods;
        if self.frame > 0 {
            self.pulse_press(key, was_held);
        }
    }

    pub fn release(&mut self, key: Key, mods: Mods) {
        self.held.remove(&key);
        self.mods = mods;
        if self.frame > 0 {
            self.pulse_release(key);
        }
    }

    /// Forget everything held, physical and pulsed.
    pub fn clear(&mut self) {
        self.held.clear();
        self.mods = Mods::NONE;
        self.clear_pulses();
    }

    /// Forget the pulses only (the app calls this when the state stack
    /// changes, so a screen never inherits the last screen's held keys); what
    /// the tracker has learned about the repeat spacing survives.
    pub fn clear_pulses(&mut self) {
        self.pulse_until.clear();
        self.train_start.clear();
        self.train_repeats.clear();
    }

    fn pulse_press(&mut self, key: Key, was_held: bool) {
        let now = self.now_ms;
        let until = self.pulse_until.get(&key).copied().unwrap_or(0);
        let repeat_after = self.delay_ms.saturating_sub(REPEAT_EARLY_TOLERANCE_MS);
        let repeating = until > now
            && self
                .train_start
                .get(&key)
                .is_some_and(|&start| now.saturating_sub(start) >= repeat_after);
        let window = if repeating {
            let repeats = self.train_repeats.entry(key).or_insert(0);
            *repeats += 1;
            let repeats = *repeats;
            // The first repeat sits at the delay, not the rate; from the
            // second on, the gap to the previous pair is the real spacing --
            // but only synthetic pairs teach it, a finger's rhythm never does.
            let previous = self.pressed_at.get(&key).copied();
            if repeats >= 2 && self.last_pair_synthetic.contains(&key) {
                if let Some(previous) = previous {
                    let spacing = now.saturating_sub(previous);
                    if spacing < repeat_after {
                        self.learn_spacing(spacing);
                    }
                }
            }
            self.repeat_pulse_ms()
        } else {
            self.train_start.insert(key, now);
            self.train_repeats.insert(key, 0);
            self.fresh_pulse_ms()
        };
        self.pulse_until.insert(key, until.max(now + window));
        self.pressed_at.insert(key, now);
        self.last_pair_synthetic.remove(&key);
        // Only a press from the released state can begin a synthetic pair:
        // SDL's own auto-repeat arrives as more presses of a key that is
        // already down, and a finger lifting in the same frame as one of
        // those is a real release.
        if was_held {
            self.pair_frame.remove(&key);
        } else {
            self.pair_frame.insert(key, self.frame);
        }
    }

    fn pulse_release(&mut self, key: Key) {
        let synthetic = self.pair_frame.get(&key) == Some(&self.frame)
            && self.frame_span_ms <= SYNTHETIC_FRAME_MAX_MS;
        if synthetic {
            self.last_pair_synthetic.insert(key);
            return;
        }
        self.pulse_until.remove(&key);
        self.train_start.remove(&key);
        self.train_repeats.remove(&key);
    }

    fn learn_spacing(&mut self, spacing_ms: u64) {
        self.spacings.push(spacing_ms);
        if self.spacings.len() > LEARNED_SPACINGS {
            let excess = self.spacings.len() - LEARNED_SPACINGS;
            self.spacings.drain(..excess);
        }
    }
}
```

Unit tests: `app/held_keys.rs:363-656` (12 tests; a `Sim` fake-clock harness,
e.g. `the_owners_jaws_train_is_one_continuous_hold_from_the_first_pair`,
`without_a_frame_clock_the_pulse_layer_is_inert`). Integration:
`tests/it/app_held_keys.rs` (143 lines; driving-coupled, uses `DrivingState`).

## 7. `single_instance.rs` and `app/logging.rs` (verbatim)

### `single_instance.rs` (135 lines)

Named Windows mutex, fails open. FF-specific values: `SINGLE_INSTANCE_MUTEX_NAME = "Local\\FreightFate.SingleInstance"`
(line 8) and the doc wording. `MutexApi` trait makes it testable
(`tests/it/single_instance.rs` supplies `FakeKernel32`). Needs `windows-sys`
features `Win32_Foundation` + `Win32_System_Threading` (already in the workspace
list, `Freight-fate/Cargo.toml:56`) and `log`. For `hearken-app`, the mutex
name must become a parameter (e.g. `SingleInstanceGuard::new(name)` or
`for_app("Hearken")`) instead of the constant used by `new()` at line 79.

Callers: `app.rs:1090-1094` (`main_with`: `let mut guard = SingleInstanceGuard::new(); if !guard.acquire() { log::warn!("Freight Fate is already running."); return 0; }` then `guard.release()` after `run_game`),
`main.rs:584`, `agent_server.rs:969`.

```rust
//! Port of `freight_fate/single_instance.py` — single-instance launch guard
//! for the game process.
//!
//! One named mutex per Windows session: the second launch sees
//! `ERROR_ALREADY_EXISTS` and declines to start. Everything fails open --
//! a mutex the OS will not create must never keep a player out of the game.

pub const SINGLE_INSTANCE_MUTEX_NAME: &str = "Local\\FreightFate.SingleInstance";
pub const ERROR_ALREADY_EXISTS: u32 = 183;

/// The two kernel32 calls the guard makes, behind a trait so the test
/// suite's `_FakeKernel32` has a Rust shape. Handles are plain integers
/// (`ctypes.c_void_p`): the Python module documents the handle-width trap
/// of leaving them as C ints.
pub trait MutexApi {
    /// `CreateMutexW(None, False, name)`: the handle (0 when the call
    /// failed) and the thread's last error right after the call.
    fn create_mutex(&self, name: &str) -> (usize, u32);
    fn close_handle(&self, handle: usize);
}

/// The real kernel32 on Windows; never constructed elsewhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinMutexApi;

#[cfg(windows)]
impl MutexApi for WinMutexApi {
    fn create_mutex(&self, name: &str) -> (usize, u32) {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the name is a valid NUL-terminated UTF-16 buffer that
        // outlives the call; a null security descriptor and no initial
        // ownership are the documented defaults.
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
            let err = GetLastError();
            (handle as usize, err)
        }
    }

    fn close_handle(&self, handle: usize) {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        // SAFETY: the handle came from CreateMutexW above and is closed once.
        unsafe {
            CloseHandle(handle as HANDLE);
        }
    }
}

#[cfg(not(windows))]
impl MutexApi for WinMutexApi {
    fn create_mutex(&self, _name: &str) -> (usize, u32) {
        (0, 0)
    }

    fn close_handle(&self, _handle: usize) {}
}

/// Coordinates one running Freight Fate instance per Windows session.
pub struct SingleInstanceGuard<A: MutexApi = WinMutexApi> {
    pub mutex_name: String,
    api: A,
    windows: bool,
    mutex_handle: Option<usize>,
    acquired: bool,
}

impl Default for SingleInstanceGuard<WinMutexApi> {
    fn default() -> Self {
        Self::new()
    }
}

impl SingleInstanceGuard<WinMutexApi> {
    /// The guard the game uses: the real kernel32 on Windows, a no-op
    /// elsewhere.
    pub fn new() -> Self {
        Self::with_api(SINGLE_INSTANCE_MUTEX_NAME, WinMutexApi, cfg!(windows))
    }
}

impl<A: MutexApi> SingleInstanceGuard<A> {
    /// A guard over an injected API. `windows` is `sys.platform == "win32"`:
    /// off it, `acquire` always succeeds without touching the API.
    pub fn with_api(mutex_name: &str, api: A, windows: bool) -> Self {
        Self {
            mutex_name: mutex_name.to_string(),
            api,
            windows,
            mutex_handle: None,
            acquired: false,
        }
    }

    pub fn acquire(&mut self) -> bool {
        if !self.windows {
            self.acquired = true;
            return true;
        }
        let (handle, last_error) = self.api.create_mutex(&self.mutex_name);
        if handle == 0 {
            log::warn!("CreateMutexW failed; allowing startup to continue");
            return true;
        }
        if last_error == ERROR_ALREADY_EXISTS {
            self.api.close_handle(handle);
            self.mutex_handle = None;
            self.acquired = false;
            return false;
        }
        self.mutex_handle = Some(handle);
        self.acquired = true;
        true
    }

    pub fn release(&mut self) {
        let Some(handle) = self.mutex_handle.take() else {
            self.acquired = false;
            return;
        };
        self.api.close_handle(handle);
        self.acquired = false;
    }

    pub fn acquired(&self) -> bool {
        self.acquired
    }
}

impl<A: MutexApi> Drop for SingleInstanceGuard<A> {
    fn drop(&mut self) {
        self.release();
    }
}
```

### `app/logging.rs` (199 lines)

Uses `log` + `env_logger` (workspace `log = "0.4"`, `env_logger = "0.11"`,
`Freight-fate/Cargo.toml:20-21`) and `chrono` for the timestamp format
`%Y-%m-%d %H:%M:%S,%3f` in a Python-logging layout
(`"{time} {LEVEL} {target}: {message}"`). Re-exported as
`pub use logging::{active_log_path, configure_logging};` (`app.rs:49`) and
called first thing in `main_with` (`app.rs:1085`), also `main.rs:457, 672`
and `agent_server.rs:990`.

Couplings to cut or parameterize:

- `crate::updater::is_frozen()` (line 53): "packaged build?" check
  (`updater.rs:188-203`: the exe stem equals `APP_NAME` and a
  `build_info.json`, `freight_fate` or `_internal` sits next to it).
- Env var names `FREIGHT_FATE_LOG_FILE` (54) and `FREIGHT_FATE_LOG` (62).
- `ff_core::settings::game_root()` (72) and `ff_core::models::profile::data_dir()`
  (73) choose the log folder: `<game_root>/logs/game.log`, or
  `<data_dir>/logs/game.log` on macOS (`packaged_log_path`, 148-151).
- Test `rotation_keeps_the_previous_log` needs `tempfile` (dev-dependency).

**Transcript target:** there is no transcript-specific code in
`logging.rs`. Transcript lines are ordinary `log::info!` records with target
`"freight_fate.transcript"`, so they land in the same file interleaved with
everything else, formatted as `... INFO freight_fate.transcript: <text>`. The
constant and macro live in `app/speech_delivery.rs:24-33` and are re-exported
at `app.rs:50` (`pub use speech_delivery::{IntoSpoken, Say, SayEvent, Spoken, TRANSCRIPT_TARGET};`):

```rust
/// The logger every spoken line lands in too, so a logged playtest reads as
/// a transcript of what the player heard -- the most faithful record for an
/// audio-first game. Same name as the Python logger so the lines grep alike.
pub const TRANSCRIPT_TARGET: &str = "freight_fate.transcript";

macro_rules! transcript {
    ($($arg:tt)*) => {
        log::info!(target: TRANSCRIPT_TARGET, $($arg)*)
    };
}
```

It is invoked in `GameContext::say_with` for every spoken main-channel line
(`transcript!("{}", text);` at `app/speech_delivery.rs:328`) plus pacer/ladder
lines tagged `[ladder]`, `[pacer]`, `[event]` (315, 432, 467, 635, 647, 677, 683, 697).
For `hearken-app`, the target string should be a parameter or
`concat!(<app>, ".transcript")`.

```rust
//! Session logging (`_configure_logging` / `active_log_path` in
//! `freight_fate/app.py`): console logging from source, a fresh log file in
//! the packaged game, the previous run kept as `game.prev.log`, and the
//! `freight_fate.transcript` lines interleaved into the same file.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::LevelFilter;

/// Where this session's log actually ended up, or None when nothing is
/// being written to disk (a source checkout with no explicit log file, or a
/// folder the game could not write to). Recorded by `configure_logging`
/// rather than derived again later, so the settings screen reports the real
/// file instead of the one the game meant to open.
static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);
static CONFIGURED: OnceLock<()> = OnceLock::new();

/// The log file this session is writing, or None when there is none.
pub fn active_log_path() -> Option<PathBuf> {
    LOG_FILE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `FREIGHT_FATE_LOG` level names (Python logging names) to a filter.
pub fn parse_level(name: &str) -> LevelFilter {
    match name.trim().to_ascii_uppercase().as_str() {
        "DEBUG" | "TRACE" => LevelFilter::Debug,
        "INFO" => LevelFilter::Info,
        "WARNING" | "WARN" => LevelFilter::Warn,
        "ERROR" | "CRITICAL" | "FATAL" => LevelFilter::Error,
        "NOTSET" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    }
}

/// Console logging from source; a fresh log file in the packaged game.
///
/// The windowed build has no console, so without a file every warning --
/// update failures especially -- vanishes. Portable builds keep the log in
/// the game folder; macOS keeps it with the player's data under Application
/// Support because an installed app bundle is read-only. An explicit
/// `FREIGHT_FATE_LOG_FILE` (set for
/// playtests/observation) forces file output and an INFO default even from a
/// source checkout, so a session can be reviewed after the fact without
/// streaming to a console. Safe to call more than once; only the first call
/// configures anything.
pub fn configure_logging() {
    if CONFIGURED.set(()).is_err() {
        return;
    }
    let packaged = crate::updater::is_frozen();
    let explicit = std::env::var("FREIGHT_FATE_LOG_FILE")
        .ok()
        .filter(|s| !s.is_empty());
    let default_level = if packaged || explicit.is_some() {
        "INFO"
    } else {
        "WARNING"
    };
    let level = std::env::var("FREIGHT_FATE_LOG")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_level.to_string());
    let level = parse_level(&level);

    let log_path: Option<PathBuf> = match explicit {
        Some(path) => Some(PathBuf::from(path)),
        None if packaged => Some(packaged_log_path(
            cfg!(target_os = "macos"),
            &ff_core::settings::game_root(),
            &ff_core::models::profile::data_dir(),
        )),
        None => None,
    };
    let file = log_path.as_deref().and_then(open_log_file);
    if file.is_some() {
        *LOG_FILE.lock().unwrap_or_else(|e| e.into_inner()) = log_path;
    }

    let mut builder = env_logger::Builder::new();
    builder.filter_level(level);
    // "%(asctime)s %(levelname)s %(name)s: %(message)s"
    builder.format(|buf, record| {
        writeln!(
            buf,
            "{} {} {}: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"),
            python_level_name(record.level()),
            record.target(),
            record.args()
        )
    });
    match file {
        Some(file) => {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
        None => {
            builder.target(env_logger::Target::Stderr);
        }
    }
    let _ = builder.try_init();

    // Python's faulthandler wrote native crash tracebacks into the log as
    // the process died; the nearest Rust equivalent is the panic hook, so a
    // panic's message and location reach the file before the default hook
    // prints it.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("Fatal error: {info}");
        default_hook(info);
    }));
}

fn python_level_name(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "ERROR",
        log::Level::Warn => "WARNING",
        log::Level::Info => "INFO",
        log::Level::Debug => "DEBUG",
        log::Level::Trace => "DEBUG",
    }
}

/// Create the log file, keeping the previous run's as `<stem>.prev<ext>`:
/// after a crash the player relaunches the game to report it, and that
/// relaunch must not wipe the evidence. `None` on an unwritable disk:
/// console-only is the best we can do.
fn open_log_file(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok()?;
    }
    if path.exists() {
        // Rotation is best-effort; a locked file still gets a fresh log.
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("game");
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let prev = path.with_file_name(format!("{stem}.prev{ext}"));
        let _ = fs::rename(path, prev);
    }
    File::create(path).ok()
}

fn packaged_log_path(macos: bool, game_root: &Path, data_dir: &Path) -> PathBuf {
    let root = if macos { data_dir } else { game_root };
    root.join("logs").join("game.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_level_names_parse() {
        assert_eq!(parse_level("WARNING"), LevelFilter::Warn);
        assert_eq!(parse_level("info"), LevelFilter::Info);
        assert_eq!(parse_level("DEBUG"), LevelFilter::Debug);
        assert_eq!(parse_level("nonsense"), LevelFilter::Info);
    }

    #[test]
    fn rotation_keeps_the_previous_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.log");
        fs::write(&path, "old run").unwrap();
        let mut file = open_log_file(&path).unwrap();
        writeln!(file, "new run").unwrap();
        drop(file);
        assert_eq!(
            fs::read_to_string(dir.path().join("game.prev.log")).unwrap(),
            "old run"
        );
        assert!(fs::read_to_string(&path).unwrap().starts_with("new run"));
    }

    #[test]
    fn packaged_macos_log_uses_application_support_not_the_app_bundle() {
        let app = Path::new("/Applications/FreightFate.app");
        let data = Path::new("/Users/driver/Library/Application Support/FreightFate");
        assert_eq!(
            packaged_log_path(true, app, data),
            data.join("logs").join("game.log")
        );
    }

    #[test]
    fn packaged_portable_log_stays_beside_the_game() {
        let game = Path::new("/games/FreightFate");
        let data = game.join("saves");
        assert_eq!(
            packaged_log_path(false, game, &data),
            game.join("logs").join("game.log")
        );
    }
}
```

## 8. Imports and couplings per file

Legend: "ctx.X: lines" = uses of `GameContext` members. Doc-comment-only
mentions are marked (doc).

### `states/base.rs`

```rust
use std::any::Any;

use ff_core::message_log::{Message, MessageLog};
use ff_core::radio::RadioState;

use crate::app::GameContext;
pub use crate::app::Say;
use crate::controller::{ControllerAxis, ControllerButton};
use crate::discord_presence::PresenceState;

mod menu;

pub use menu::{
    DynamicLabel, Label, Menu, MenuAction, MenuCore, MenuItem, SimpleMenuState, DEFAULT_INTRO_HELP,
};
```

- `ff_core::message_log::{Message, MessageLog}` (28): global message review
  (`handle_message_review` 557-626, `hidden_notice` 634-641,
  `speak_review_message` 644-664) and `State::handle_message_review` (532-537).
- `ff_core::radio::RadioState` (29): `State::radio` (521-524). Trucking.
- `crate::app::GameContext` (31): every `State` hook takes `&mut GameContext`.
- `pub use crate::app::Say` (32): speech options, defined in `app/speech_delivery.rs:40-87`.
- `crate::controller::{ControllerAxis, ControllerButton}` (33): `InputEvent`
  controller variants (284-302), `controller_key_map` (541-552).
- `crate::discord_presence::PresenceState` (34): `State::presence` /
  `online_presence` (477-489). Trucking (Discord Rich Presence + drivers board).
- ctx: `say` 727, 738 (426 doc); `say_with` 566, 576, 608, 620, 660, 753;
  `update_music_rotation` 465, 743 (462 doc); `audio.play` 725;
  `message_log.*` 564-656; `write_clipboard_text` 619; `stop_event_speech` 650.
- FF-only `State` hooks: `paces_main_speech` (432-434, driving),
  `presence` / `online_presence`, `ticks_covered_music` / `tick_covered_music`
  (505-510, radio playlist under a menu), `applies_radio_settings` /
  `apply_radio_settings_now` / `radio` (514-524), and the default `update`
  calling `ctx.update_music_rotation(dt)` (464-466). Note that FF's
  `update_music_rotation` also calls `self.sync_fuel_prices()` first
  (`app/context.rs:901-904`), so every state's per-frame update drives a
  trucking side effect.

### `states/base/menu.rs`

```rust
use std::rc::Rc;

use ff_core::radio::RadioState;

use crate::app::{GameContext, Say};
use crate::controller::ControllerAction;
use crate::discord_presence::PresenceState;
use crate::states::base::{end_sentence, InputEvent, Key};
```

- `ff_core::radio::RadioState` (8): `Menu::radio` (263-265) and the macro (499).
- `crate::app::{GameContext, Say}` (10).
- `crate::controller::ControllerAction` (11): `Menu::handle_controller` (295-309).
- `crate::discord_presence::PresenceState` (12): `Menu::presence` /
  `online_presence` (234-240), macro 469-480.
- `crate::states::base::{end_sentence, InputEvent, Key}` (13).
- ctx: `audio.play` 221, 377, 387, 398, 404, 423; `controller.menu_action` 296;
  `pop_state` 405; `say` 283, 305, 325, 531; `say_with` 327, 353;
  `settings.announce_menu_position` 345 (field on `ff_core::settings::Settings`,
  `ff-core/src/settings.rs:559`, default true); `stop_speech` 285;
  `update_music_rotation` 229; `quit` 196 (doc).
- The macro hard-codes `$crate::states::base::{State, Menu, InputEvent}`,
  `$crate::app::GameContext` and `$crate::discord_presence::PresenceState`, and
  uses the absolute `ff_core::radio::RadioState` (499), which only resolves in a
  crate that depends on `ff_core`.

### `states/text_entry.rs`

```rust
use crate::app::{GameContext, Say};
use crate::states::base::{spoken_char, InputEvent, Key, State};
```

- ctx: `audio.play` 112, 164, 171, 196, 209; `pop_state` 113; `say` 93;
  `say_with` 131, 165, 181, 203, 214, 222; `update_music_rotation` 103.
- Macro paths: `$crate::states::base::{State, InputEvent}`,
  `$crate::states::text_entry::TextEntry`, `$crate::app::GameContext`.
- No trucking coupling.

### `states/save_notice.rs`

See section 3 (imports 4-9; all couplings trucking; usage sample only).

### `bindings.rs`

```rust
use std::collections::HashMap;

use ff_core::settings::Settings;

use crate::app::held_keys::HeldKeys;
use crate::controller::ControllerButton;
use crate::states::base::{Key, Mods};

mod names;

pub use names::{key_saved_name, key_spoken_name, pad_button_short_name, parse_key_name};
```

- `ff_core::settings::Settings` (29): `from_settings` / `store` (737-770) read
  and write `settings.key_bindings` / `settings.pad_bindings`.
- `crate::app::held_keys::HeldKeys` (31): `KeyBindings::pressed` (825-833).
- `crate::controller::ControllerButton` (32): `PadChord`, `TABLE`,
  `reserved_pad_reason`.
- `crate::states::base::{Key, Mods}` (33).
- Tests use `ff_core::input_hints::{HINTS, control_hint, KEYBOARD, CONTROLLER}` (1093-1100).
- Trucking: `Action`, `TABLE`, reserved-key reasons, `hint_*` (see section 4).

### `bindings/names.rs`

```rust
use crate::controller::ControllerButton;
use crate::states::base::Key;
```

No trucking coupling.

### `controller.rs`

```rust
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use ff_core::input_hints::{control_hint, CONTROLLER, KEYBOARD};
use ff_core::rumble::{RumbleEngine, RumbleSink};

use crate::states::base::InputEvent;

pub mod diagnostics;
pub mod fakes;
pub mod sdl;
```

- `ff_core::input_hints::{control_hint, CONTROLLER, KEYBOARD}` (37): the
  `&'static str` device ids (369, 479, 624, 630-633, 738, 761, 767, 773) and
  `hint()` (639). The two constants are generic; `control_hint` is a trucking
  phrase table (`ff-core/src/input_hints.rs:21-62`).
- `ff_core::rumble::{RumbleEngine, RumbleSink}` (38): `DeviceRumbleSink`
  (288-309), `rumble` field (338, 376), `reset()` (428, 486, 617), `tick` (817).
  `ff-core/src/rumble.rs` is 493 lines; its effects are trucking.
- `crate::states::base::InputEvent` (40).
- No `GameContext` use. It is owned as `GameContext.controller`
  (`app/context.rs:133`) and read as `ctx.controller.*` by states.

### `controller/sdl.rs`, `controller/fakes.rs`, `controller/diagnostics.rs`

```rust
use sdl2::controller::{Axis, Button, GameController};
use sdl2::GameControllerSubsystem;

use super::{ControllerAxis, ControllerButton, PadDevice, PadSubsystem, PadSubsystemFactory};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{PadDevice, PadSubsystem, PadSubsystemFactory};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use sdl2::event::Event;
use sdl2::joystick::HatState;

use super::AXIS_MAX;
```

No trucking coupling (diagnostics window title at 126 says "Freight Fate").

### `app/held_keys.rs`

```rust
use std::collections::{HashMap, HashSet};

use crate::states::base::{Key, Mods};
pub fn os_repeat_timing() -> (u64, u64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED,
    };
```

No trucking coupling; `windows-sys` only on Windows.

### `single_instance.rs`

No `use` at file top; function-local imports:

```rust
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Threading::CreateMutexW;
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
```

Coupling: mutex name `"Local\\FreightFate.SingleInstance"` (8); `log::warn!` (103).

### `app/logging.rs`

```rust
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::LevelFilter;
```

Couplings: `crate::updater::is_frozen()` (53), `ff_core::settings::game_root()` (72),
`ff_core::models::profile::data_dir()` (73), env var names `FREIGHT_FATE_LOG_FILE` (54)
and `FREIGHT_FATE_LOG` (62), `env_logger` + `chrono` crates.

### `states/main_menu_help.rs`

```rust
use crate::app::GameContext;
use crate::bindings::Action;
use crate::states::base::{InputEvent, Key, State};
use crate::states::main_menu::ShortcutDevice;
```

- `crate::bindings::Action` (5): `render_help_line` resolves `{{id}}`,
  `{{pad:id}}`, `{{key:id}}` via `Action::from_id` (305).
- `crate::states::main_menu::ShortcutDevice` (7): device pin (300-302).
- ctx: `bindings.pad_spoken` 307, `bindings.spoken` 308, `control_name` 309,
  `say` 395, 418, 424, 428, 432, 435, `audio.play` 410, 417, 423, `pop_state` 411,
  `stop_speech` 413.
- `HELP_PAGES` (16-286) is all Freight Fate manual text. The generic parts are
  the placeholder renderer (288-328) and the `HelpState` reader (355-457).

### `states/main_menu/shortcuts.rs`

```rust
use crate::app::GameContext;
use crate::bindings::{Action, Chord, PadChord, Rebind};
use crate::controller::{ControllerAction, ControllerButton};
use crate::impl_state_for_menu;
use crate::states::base::{InputEvent, Key, Label, Menu, MenuCore, MenuItem, Mods};

use super::settings::{base_handle_event, save_settings};
```

- ctx: `bindings.spoken` 65, `bindings.pad_spoken` 66, `bindings.set_chord` 135,
  `bindings.set_pad_chord` 150, `bindings.store` 92, 159, `bindings.reset_keys` 156,
  `bindings.reset_pad` 157, `settings` 92-93, 159-160, 248, `controller.modifier` 148,
  `controller.menu_action` 234, `audio.play` 94, 104, 112, 249, `say` 84, 97, 101,
  105, 113, 131, 162, 241, `pop_state` 250.
- `super::settings::{base_handle_event, save_settings}` (18): see section 4.

### `app/sdl_shell.rs` (translation only)

```rust
use sdl2::clipboard::ClipboardUtil;
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::pixels::Color;
use sdl2::render::WindowCanvas;
use sdl2::{EventPump, Sdl, VideoSubsystem};

use crate::controller::sdl::{axis_from_sdl, button_from_sdl};
use crate::states::base::{InputEvent, Key, Mods};

use super::context::Clipboard;
use super::{BG_COLOR, WINDOW_SIZE};
```

`BG_COLOR` / `WINDOW_SIZE` come from `app.rs`; `Clipboard` from `app/context.rs`.

### `GameContext` surface the extracted code needs

Collected from the uses above. This is the minimum a `hearken-app` context
(or trait) must offer for `Menu`, `TextEntry`, `TimedMessageState` and
`HelpState` to compile unchanged:

| member | FF definition |
|---|---|
| `say(&mut self, text: &str)` | `app/speech_delivery.rs:266-268` (= `say_with(text, Say::new())`) |
| `say_with(&mut self, text: impl IntoSpoken, say: Say)` | `app/speech_delivery.rs:271-362`; FF adds ladder/pacer/terse; core is `self.speech.say(&text, interrupt)` (356) then `message_log.add(&text, MessageCategory::General)` if `review` (360-362), after `transcript!("{}", text)` (328) |
| `stop_speech(&mut self)` | `app/speech_delivery.rs:925-929` (resets event pacer, `speech.stop_main()`, `speech.stop_event()`) |
| `stop_event_speech(&mut self)` | `app/speech_delivery.rs:914-917` (message review only) |
| `audio: Box<dyn Audio>` with `play(&mut self, key: &str)` | `audio.rs:557-559` (`play_with(key, 1.0, 0.0)`); `Audio` trait at `audio.rs:520` is large and trucking-heavy (engine, weather, radio volumes) |
| `pop_state(&mut self)` | `app/context.rs:554-556` (`pop_state_with(true, true)`) |
| `update_music_rotation(&mut self, dt: f64)` | `app/context.rs:901` (also syncs fuel prices) |
| `controller: ControllerManager` (`menu_action`, `modifier`) | `app/context.rs:133` |
| `settings.announce_menu_position: bool` | `ff-core/src/settings.rs:559` |
| `message_log: MessageLog`, `write_clipboard_text` | `app/context.rs:144`, `812-814` (message review only) |
| `input: HeldKeys` | `app/context.rs:168` |
| `bindings: KeyBindings` | `app/context.rs:137` |

`Say` (`app/speech_delivery.rs:40-87`) is `{ interrupt: bool, review: bool, category: Option<SpeechCategory> }`
with `Say::new()` (interrupt + review), `Say::queued()` (no interrupt,
review), `.interrupt(b)`, `.review(b)`, `.category(c)`. `SpeechCategory`
comes from ff-core and is trucking.

## 9. How menus speak, play earcons, and do F1 help

### Speech

All menu and text-entry speech goes through the **main channel** via
`GameContext::say` / `say_with` (`app/speech_delivery.rs:266-362`), which ends in
`SpeechSink::say(&text, interrupt)` (`speech.rs:167-169`; the sink is
`GameContext.speech: Box<dyn SpeechSink>`, backed by Prism/screen reader). The
event voice (`say_event`, SAPI) is driving-only. Every main-channel line is also
logged under target `freight_fate.transcript` (section 7). `Say.review = true`
also adds the line to `message_log` (the comma/period review history).

Menu call sites (`states/base/menu.rs`):

| when | call | effect |
|---|---|---|
| enter: title (`announce_entry`, 323-328) | `ctx.say(&end_sentence(&title))` | interrupt, logged for review |
| enter: current row | `ctx.say_with(current, Say::queued().review(false))` | queued after title, not logged |
| move / jump / first-letter (`speak_current`, 351-354) | `ctx.say_with(text, Say::new().review(false))` | interrupt, not logged |
| F1 / pad Back (283, 305) | `ctx.say(&help)` | interrupt, logged |
| Left/Right Control (285) | `ctx.stop_speech()` | silences main + event |

`current_text` (339-349): `end_sentence(label)`, plus `" {i} of {n}."` when
`ctx.settings.announce_menu_position` is true; `"No options available."` for an
empty menu. Row labels are `Label::Text` or `Label::dynamic(|state, ctx| ..)`,
evaluated each time they are spoken.

Text entry (`states/text_entry.rs`): the `enter` prompt uses `ctx.say` (93);
everything else uses `Say::new().review(false)`: character echo via `spoken_char`
("space", "cap j"), "Deleted x. rest", review-cursor characters, "Start. x" /
"End. x", F2 whole text or "Empty.".

TimedMessageState: `ctx.say(&message)` on enter, `ctx.say(&status)` on
F1/Escape/Enter/Space/KpEnter, `ctx.say_with(complete_text, Say::queued())` on completion.

### Earcons

`ctx.audio.play(key)` (`Audio::play` at `audio.rs:557-559` = `play_with(key, 1.0, 0.0)`).
Keys are paths under the sound library without extension (`audio.rs:26-28`),
resolved by `audio/assets.rs` in the order generated-sound registry, shipped
packs (`assets/sounds.pak`), licensed overlay, loose `assets/sounds/`. The
`ui/*` clips are not loose files in the checkout; they come from
`assets/sounds.pak`. Keys starting with `ui/` play on the UI volume category
(`audio/backend.rs:52`). `verify_sound_assets` treats a missing
`ui/menu_select` as "sound assets missing" (`audio/assets.rs:402, 411`).

| clip | played by |
|---|---|
| `ui/menu_open` | `MenuCore::new` default `open_sound_key` (148); played in `Menu::enter` (220-222); `with_open_sound(None)` disables |
| `ui/menu_move` | `move_by` (377), `jump` (387), `first_letter_jump` (423); HelpState page change (`main_menu_help.rs:417, 423`) |
| `ui/menu_select` | `MenuItem::new` default `select_sound` (99), played in `activate` (397-399) before the action; `.select_sound(None)` for rows whose action plays its own sound; shortcuts rebind `Done` (`shortcuts.rs:94`) |
| `ui/menu_back` | `go_back` (404), text entry Escape (`text_entry.rs:112`), HelpState Escape (`main_menu_help.rs:410`), shortcuts `go_back` (249) |
| `ui/tick` | text entry character inserted (`text_entry.rs:164`) |
| `ui/error` | text entry backspace at 0 / cursor at an edge / jump to where it already is (171, 196, 209); shortcuts `Taken`/`Reserved` (104, 112) |
| `ui/notify` | `TimedMessageState` default `sound_key` (`states/base.rs:700`), played on enter (724-726) |

### F1 help

- **Per-row help in menus:** `Key::F1` in `Menu::handle_event` (281-284) and
  `ControllerAction::Help` (pad Back button) in `Menu::handle_controller`
  (303-306) both call `ctx.say(&self.current_help(ctx))`. `current_help`
  (356-368) returns the focused row's `help` label if non-empty, otherwise
  `"{label}."`; if the menu has no rows it returns `MenuCore.intro_help`
  (default `DEFAULT_INTRO_HELP` = "Enter selects, Escape goes back. Control stops
  the current speech."). `intro_help` is only spoken for an empty menu
  (other readers: `states/city/board.rs:200, 216`, `states/city.rs:862`).
- **Plain states:** `State::handle_controller` default maps pad Back to
  `Key::F1` (`states/base.rs:545`), so any state that handles F1 gets pad help.
  TimedMessageState answers F1 with its status line.
- **Text entry** has no F1 handler (F1 falls through to `_`, and since F1
  carries no typed text nothing happens). F2 reads the field.
- **The manual (`HelpState`, `states/main_menu_help.rs:355-457`)** is a separate
  "How to play" reader opened from the main menu (`states/main_menu.rs:833`,
  `HelpState::new()`) and from the pause menu at the driving-keys page
  (`states/driving_pause_states.rs:113-114`, `HelpState::at_page(controls_help_page())`).
  It is not reached through F1. Keys: Left/Right or PageUp/PageDown change page
  (wrapping, `ui/menu_move`, speaks "Page i of n: title. k lines."), Up/Down
  read line by line (clamped), Enter/KpEnter/Space read the whole page,
  Left/Right Control stop speech, Escape pops (`ui/menu_back`). Lines are
  rendered through `render_help_line` (288-328), which replaces
  `{{action_id}}`, `{{pad:id}}`, `{{key:id}}` with the player's current binding
  name and capitalizes a placeholder that opens the line.
- **F1 is reserved** from rebinding (`bindings.rs:660`, "F1 is help").

## 10. Tests

Integration tests are one binary: `crates/freight-fate/tests/it/main.rs` declares
each file as a `mod`. They run on the headless harness
`freight_fate::app::testing::TestApp` (`src/app/testing.rs`, 638 lines):
`TestApp::new()`, `push_state`, `dispatch_to_state(&InputEvent)`,
`dispatch_controller(&InputEvent)`, `state()` (downcast via `as_any()`),
`main_lines()` (spoken main-channel text), `clear_speech()`, `speech()` →
`CaptureSpeech` (`stop_main_calls()`, `stop_event_calls()`), `record_audio()` →
`AudioLog` with `.played`, `shutdown()`. `hearken-app` will need an equivalent
headless context with capture speech and recording audio.

| file (tests/it/) | lines | #tests | covers |
|---|---|---|---|
| `transcript_menu_readout.rs` | 59 | 3 | `SimpleMenuState` readout, `end_sentence`, "N of M" toggle |
| `transcript_menu_stop_speech.rs` | 79 | 4 | Left/Right Control stop speech in `Menu` and `HelpState`; `DEFAULT_INTRO_HELP` text |
| `app_message_review.rs` | 373 | 13 | review keys over a `SimpleMenuState`; menu navigation stays out of review log (155-171); text entry keeps punctuation (122, 356) |
| `app_name_entry.rs` | 315 | 14 | text entry via `NameEntryState` (echo, cap marker, review cursor, Home/End, backspace at cursor, F2) |
| `app_key_bindings.rs` | 511 | 18 | shortcuts screen capture, chords, taken/reserved refusal, reset, pad layer, bindings at the wheel, help placeholders |
| `app_controller.rs` | 774 | 33 | manager (deadzone, smoothing, clutch, modifier, repeat, hot-plug, id latch, duplicates, enable/disable), menu D-pad (359-395), plain-state B->Escape (700-720), hints by device, driving pad controls, disconnect, rumble guard |
| `app_held_keys.rs` | 143 | 3 | JAWS pulse at the wheel (driving-coupled), no-frame-clock plain semantics, new screen does not inherit hold |
| `single_instance.rs` | 107 | 5 | `FakeKernel32` over `MutexApi` |
| `app_state_stack.rs` | 186 | 6 | push/pop/replace/reset semantics |

Unit tests inside source files: `states/base.rs:770-798` (3), `bindings.rs:971-1115` (8),
`controller.rs:913-1001` (7), `app/held_keys.rs:363-656` (12), `app/logging.rs:153-199` (4),
`app/sdl_shell.rs` from 456.

### Representative test 1: menu readout (`tests/it/transcript_menu_readout.rs`, full file)

```rust
//! Spoken menu readout: no doubled periods, optional "N of M" position
//! (port of `tests/test_menu_readout.py`).
//!
//! Python built the menu on a `SimpleNamespace` context carrying nothing but
//! `settings.announce_menu_position`. Rust menus read a real `GameContext`,
//! so the rig is a headless [`TestApp`] and the throwaway two-row screen is
//! a [`SimpleMenuState`] -- the same rows, read through the same
//! `current_text`.

use freight_fate::app::testing::TestApp;
use freight_fate::app::GameContext;
use freight_fate::states::base::{end_sentence, Menu, MenuItem, SimpleMenuState};

/// `_menu(announce_position)`: the two-row screen, items already built.
fn menu(ctx: &mut GameContext, announce_position: bool) -> SimpleMenuState {
    ctx.settings.announce_menu_position = announce_position;
    let mut state = SimpleMenuState::new(
        "Menu",
        vec![
            MenuItem::new("Delivered 16 tons to Boston.", |_, _| {}),
            MenuItem::new("Sleep 10 hours", |_, _| {}),
        ],
    );
    state.refresh(ctx, false);
    state
}

#[test]
fn test_end_sentence_adds_one_period_never_two() {
    assert_eq!(end_sentence("Sleep 10 hours"), "Sleep 10 hours.");
    assert_eq!(end_sentence("Delivered to Boston."), "Delivered to Boston.");
    assert_eq!(end_sentence("Ready?"), "Ready?");
    assert_eq!(end_sentence("Note:"), "Note:");
    assert_eq!(end_sentence("trailing space.  "), "trailing space.");
}

#[test]
fn test_menu_readout_never_doubles_the_period() {
    let mut app = TestApp::new();
    let mut m = menu(&mut app.ctx, true);
    // A sentence item keeps its single period before the counter.
    assert_eq!(
        m.current_text(&app.ctx),
        "Delivered 16 tons to Boston. 1 of 2."
    );
    // A plain label still gets its period added.
    m.menu_mut().index = 1;
    assert_eq!(m.current_text(&app.ctx), "Sleep 10 hours. 2 of 2.");
}

#[test]
fn test_menu_position_can_be_suppressed() {
    let mut app = TestApp::new();
    let mut m = menu(&mut app.ctx, false);
    // no "1 of 2"
    assert_eq!(m.current_text(&app.ctx), "Delivered 16 tons to Boston.");
    m.menu_mut().index = 1;
    assert_eq!(m.current_text(&app.ctx), "Sleep 10 hours.");
}
```

### Representative test 2: text entry review cursor (`tests/it/app_name_entry.rs:16-106`)

```rust
use freight_fate::app::testing::TestApp;
use freight_fate::states::base::{InputEvent, Key, Menu};
use freight_fate::states::main_menu::{MainMenuState, NameEntryState};

fn key_event(key: Key) -> InputEvent {
    InputEvent::key(key)
}

fn open_name_entry(app: &mut TestApp) {
    app.push_state(MainMenuState::new());
    loop {
        let on_new_career = {
            let state = app.state().unwrap();
            let state = state.borrow();
            let menu = state.as_any().downcast_ref::<MainMenuState>().unwrap();
            let core = menu.menu();
            core.items[core.index].text(menu, &app.ctx) == "New career"
        };
        if on_new_career {
            break;
        }
        app.dispatch_to_state(&key_event(Key::Down));
    }
    app.dispatch_to_state(&key_event(Key::Return));
    assert!(app
        .state()
        .unwrap()
        .borrow()
        .as_any()
        .is::<NameEntryState>());
    app.clear_speech();
}

fn type_text(app: &mut TestApp, text: &str) {
    for ch in text.chars() {
        app.dispatch_to_state(&InputEvent::typed(ch));
    }
}

fn name(app: &TestApp) -> String {
    let state = app.state().unwrap();
    let state = state.borrow();
    state
        .as_any()
        .downcast_ref::<NameEntryState>()
        .unwrap()
        .name()
}

fn cursor(app: &TestApp) -> usize {
    let state = app.state().unwrap();
    let state = state.borrow();
    state
        .as_any()
        .downcast_ref::<NameEntryState>()
        .unwrap()
        .cursor()
}

fn spoken(app: &TestApp) -> Vec<String> {
    app.main_lines()
}

#[test]
fn test_typed_characters_are_echoed_including_space_and_capitals() {
    let mut app = TestApp::new();
    open_name_entry(&mut app);
    type_text(&mut app, "Jo Ann");
    assert_eq!(name(&app), "Jo Ann");
    assert_eq!(spoken(&app), vec!["cap j", "o", "space", "cap a", "n", "n"]);
    app.shutdown();
}

#[test]
fn test_left_arrow_reviews_characters_back_to_front() {
    let mut app = TestApp::new();
    open_name_entry(&mut app);
    type_text(&mut app, "Cary");
    app.clear_speech();

    for _ in 0..4 {
        app.dispatch_to_state(&key_event(Key::Left));
    }

    assert_eq!(spoken(&app), vec!["y", "r", "a", "cap c"]);
    assert_eq!(cursor(&app), 0);
    // Reviewing does not change what was typed.
    assert_eq!(name(&app), "Cary");
    app.shutdown();
}

```

### Also useful: bindings unit test (`bindings.rs:1033-1060`) and controller menu test (`tests/it/app_controller.rs:49-56, 358-395`)

```rust
    #[test]
    fn moving_a_key_frees_the_old_one_and_refuses_a_taken_one() {
        let mut b = KeyBindings::default();
        assert_eq!(
            b.set_chord(Action::Engine, Chord::plain(Key::T)),
            Rebind::Taken(Action::Rest)
        );
        assert_eq!(
            b.set_chord(Action::Engine, Chord::plain(Key::Z)),
            Rebind::Done
        );
        assert_eq!(b.action_for(Key::Z, Mods::NONE), Some(Action::Engine));
        assert_eq!(b.action_for(Key::E, Mods::NONE), None);
        assert_eq!(
            b.set_chord(Action::Engine, Chord::plain(Key::Z)),
            Rebind::Unchanged
        );
        assert_eq!(
            b.set_chord(Action::Engine, Chord::plain(Key::Escape)),
            Rebind::Reserved("Escape is the pause menu")
        );
        // Back to the default clears the override rather than recording it.
        assert_eq!(
            b.set_chord(Action::Engine, Chord::plain(Key::E)),
            Rebind::Done
        );
        assert!(b.is_default());
    }
```

```rust
/// `m._controller = object(); m._instance_id = id`: a bound pad, id latched.
fn manager_with_pad(instance_id: u32) -> ControllerManager {
    let mut m = ControllerManager::detached(true, true);
    m.bind_device(Box::new(FakePad::new(instance_id)), "test pad");
    m.set_id_pending(false);
    m
}


#[test]
fn test_menu_dpad_moves_and_adjusts() {
    use freight_fate::states::base::Menu;
    use freight_fate::states::main_menu::SettingsCategoryState;

    let mut app = TestApp::new();
    force_controller(&mut app);
    app.push_state(SettingsCategoryState::new("controls"));
    let index = |app: &TestApp| -> usize {
        let state = app.state().unwrap();
        let state = state.borrow();
        state
            .as_any()
            .downcast_ref::<SettingsCategoryState>()
            .unwrap()
            .menu()
            .index
    };
    let start_index = index(&app);
    app.dispatch_controller(&button(ControllerButton::DPadDown, 0));
    assert_eq!(index(&app), start_index + 1);
    // Move to the Units item and adjust it with D-pad right.
    {
        let state = app.state().unwrap();
        let mut state = state.borrow_mut();
        state
            .as_any_mut()
            .downcast_mut::<SettingsCategoryState>()
            .unwrap()
            .menu_mut()
            .index = 0;
    }
    let before = app.ctx.settings.imperial_units;
    app.dispatch_controller(&button(ControllerButton::DPadRight, 0));
    assert_ne!(app.ctx.settings.imperial_units, before);
    app.shutdown();
}

```

## Cut list: trucking and Freight Fate couplings to cut or parameterize

1. **`GameContext` everywhere.** `State`, `Menu`, `TextEntry`, `Label`,
   `MenuItem`, `MenuAction`, `TimedMessageState` and both macros name
   `crate::app::GameContext` directly. `hearken-app` needs its own context
   type, or a generic `C` / trait, offering the surface in the section 8 table.
   The macros must switch to `$crate::` paths that exist in `hearken-app`.
2. **`State` trait hooks to drop or move to FF:** `paces_main_speech`,
   `presence`, `online_presence` (`PresenceState`, `crate::discord_presence`),
   `ticks_covered_music`, `tick_covered_music`, `applies_radio_settings`,
   `apply_radio_settings_now`, `radio` (`ff_core::radio::RadioState`),
   `handle_message_review` (`ff_core::message_log`). The same hooks are
   mirrored on `Menu` (211-265) and forwarded by `impl_state_for_menu!`
   (437-501, including the absolute `ff_core::radio::RadioState` at 499).
3. **Default `update` calls `ctx.update_music_rotation(dt)`**
   (`states/base.rs:464-466`, `menu.rs:228-230`, `text_entry.rs:102-104`,
   `TimedMessageState` 743). In FF that also syncs fuel prices
   (`app/context.rs:901-904`). Make it an empty default or an app-level
   per-frame hook.
4. **Message review** (`states/base.rs:554-664`): depends on
   `ff_core::message_log::MessageLog` and `ctx.stop_event_speech` /
   `write_clipboard_text`. Move it together with a message log, or leave it
   in FF.
5. **`Say` / speech delivery:** `Say.category: Option<SpeechCategory>` and the
   ladder/pacer/terse logic in `say_with` are trucking. The generic core is
   interrupt + review + transcript log + `SpeechSink::say`.
6. **`ctx.settings.announce_menu_position`** lives on `ff_core::settings::Settings`
   (`settings.rs:559`). Needs a home in `hearken-app` settings, or a context accessor.
7. **Audio:** menus need only `play(&str)`. FF's `Audio` trait (`audio.rs:520`)
   carries engine/weather/radio volumes. Use a minimal trait; keep the clip
   keys `ui/menu_open`, `ui/menu_move`, `ui/menu_select`, `ui/menu_back`,
   `ui/tick`, `ui/error`, `ui/notify`.
8. **Bindings:** the entire `Action` enum + `TABLE` (51 driving actions);
   `reserved_key_reason` / `reserved_pad_reason` texts (pause menu, cruise,
   radio dial, message review, clutch, second layer);
   `hint_key_phrase` / `hint_pad_phrase` / `hint_action` (`ff_core::input_hints`);
   storage on `Settings.key_bindings` / `pad_bindings`. Keep `Chord`,
   `PadChord`, `Rebind`, the override map and storage format, and `names.rs`,
   and make `KeyBindings` generic over a game action type.
9. **`base_handle_event` duplication** (`states/main_menu/settings.rs:249-274`
   copies `Menu::handle_event`). Export the default body as a public free
   function in `hearken-app`.
10. **Controller:** pedals/wheel (`steer_target`, `throttle*`, `brake*`,
    `clutch`, `on_axis` mapping, `SMOOTH_RATE`, LeftShoulder = clutch,
    `steering()` … `clutch()` reads), rumble (`ff_core::rumble`),
    `hint()` / `control_hint`, `NO_CONTROLLER_ENV = "FREIGHT_FATE_NO_CONTROLLER"`,
    diagnostics window title. `KEYBOARD` / `CONTROLLER` device-id constants come
    from `ff_core::input_hints` and need a new home.
11. **Single instance:** mutex name `"Local\\FreightFate.SingleInstance"`
    (`single_instance.rs:8`, used by `new()` at 79). Take the name as a parameter.
12. **Logging:** `crate::updater::is_frozen()`, `ff_core::settings::game_root()`,
    `ff_core::models::profile::data_dir()`, env vars `FREIGHT_FATE_LOG_FILE` /
    `FREIGHT_FATE_LOG`, log file `logs/game.log`. Parameterize via a config
    struct (app name, env prefix, log dir, packaged flag). Transcript target
    `"freight_fate.transcript"` (`app/speech_delivery.rs:27`) becomes
    `<app>.transcript`.
13. **Help:** `HELP_PAGES` text and `controls_help_page()` ("Driving information
    keys") are FF content. `render_help_line` depends on `bindings::Action` and
    `ShortcutDevice`. `HelpState` hard-codes the `HELP_PAGES` static; take the
    pages as data.
14. **Menu example text:** the `ConfirmQuitState` doc example says "Quit Freight
    Fate?" (`menu.rs:188`), and the `SimpleMenuState::readout` doc mentions
    "the driving status screens, the logbook" (522-525). Cosmetic only.
15. **Not trucking but FF-app-bound:** `HeldKeys` wiring (`ctx.input`,
    `begin_frame`, `clear_pulses` on stack change, focus handling in
    `App::handle_event`), controller dispatch/tick/disconnect in `App`
    (`app.rs:666-806`), and `app/sdl_shell.rs::translate_events` all live in FF's
    `App`. `hearken-app` needs its own loop to replicate that routing.
