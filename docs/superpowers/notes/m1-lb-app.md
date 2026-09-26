# M1 notes: Lucky Break app layer (shell, stack, menus, settings)

Source: `C:\Users\joshu\gh-projects\lucky-break\crates\lucky-break\`, commit
`a1019704bfaf1285705c07492ca13305292aef72` (a101970). All paths below are
relative to that crate directory. All code blocks are copied mechanically from
the files at that commit (`sed -n start,endp`), so line numbers are exact.

Scope reminder (from `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`
lines 170-212): hearken-app takes LB's `Shell` trait (SDL + headless) and LB's
queued `StackOp` stack; menus come from Freight Fate, so LB's menu is recorded
here for comparison and for the M3 port.

Contents: 1 shell.rs, 2 input.rs, 3 paths.rs, 4 states/mod.rs, 5 app/mod.rs,
6 menus + settings/help/pause + settings.rs, 7 tests, 8 Cargo.toml,
9 environment variables, 10 generic vs casino-specific.

---

## 1. `src/app/shell.rs` (verbatim)

Facts worth carrying into the plan:

- `Shell` trait (lines 17-23): `poll() -> Vec<InputEvent>`, `render(&[String])`,
  `frame() -> f64`. Object-safe; `App::run` takes `&mut dyn Shell`.
- `SdlShell` (25-82): window fixed at 640x360 (line 37), centered. No text is
  drawn (no SDL_ttf): `render` clears to RGB(12,24,16) and puts the last
  rendered line into the window title as `"{WINDOW_TITLE} - {last}"` (lines
  62-68). **Coupling:** line 66 reads `super::WINDOW_TITLE` (the constant in
  `app/mod.rs:33`), so a generic SdlShell needs the title stored on the struct.
- `frame()` sleeps to 1/60 s (`FRAME_SECONDS`, line 15), returns elapsed
  seconds clamped to 0.25 (line 80).
- `translate_events` (84-122): `Quit`, `Window FocusLost`, `KeyDown` with
  `repeat: false` only (OS key-repeat events are dropped), `KeyUp`, and
  `TextInput`, whose first char is attached to the immediately preceding
  `KeyDown` in the same batch (lines 93-99). A `TextInput` with no preceding
  `KeyDown` in the batch is dropped. No explicit `start_text_input` call
  exists anywhere in the crate (SDL2 has text input on by default).
- No controller/gamepad events, no mouse, no `Mod::GUI`.
- `HeadlessShell` (193-228): `queued: VecDeque<InputEvent>`, `dt: f64`
  (default `FRAME_SECONDS`), `rendered: Vec<Vec<String>>`. `poll` pops **one**
  queued event per frame (line 220); `render` records every frame's lines;
  `frame` returns `dt` with no sleep (a bare `--headless` run with no quitting
  event spins forever at full CPU).

`src/app/shell.rs` (whole file, 282 lines):

```rust
//! The window and the event pump, behind a trait so tests and `--headless`
//! runs need no display.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::pixels::Color;
use sdl2::render::WindowCanvas;
use sdl2::EventPump;

use super::input::{InputEvent, Key, Mods};

pub const FRAME_SECONDS: f64 = 1.0 / 60.0;

pub trait Shell {
    fn poll(&mut self) -> Vec<InputEvent>;
    fn render(&mut self, lines: &[String]);
    /// Sleep out the remainder of a frame and return the seconds elapsed
    /// since the previous call.
    fn frame(&mut self) -> f64;
}

pub struct SdlShell {
    _sdl: sdl2::Sdl,
    canvas: WindowCanvas,
    pump: EventPump,
    last: Instant,
}

impl SdlShell {
    pub fn new(title: &str) -> Result<Self, String> {
        let sdl = sdl2::init()?;
        let video = sdl.video()?;
        let window = video
            .window(title, 640, 360)
            .position_centered()
            .build()
            .map_err(|e| e.to_string())?;
        let canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
        let pump = sdl.event_pump()?;
        Ok(SdlShell {
            _sdl: sdl,
            canvas,
            pump,
            last: Instant::now(),
        })
    }
}

impl Shell for SdlShell {
    fn poll(&mut self) -> Vec<InputEvent> {
        let raw: Vec<Event> = self.pump.poll_iter().collect();
        translate_events(raw)
    }

    fn render(&mut self, lines: &[String]) {
        // The window is a focus target and a mirror for the transcript in the
        // title bar; text rendering needs SDL_ttf, which the release does not
        // ship. Sighted helpers read the log file or the last line here.
        self.canvas.set_draw_color(Color::RGB(12, 24, 16));
        self.canvas.clear();
        self.canvas.present();
        if let Some(last) = lines.last() {
            let title = format!("{} - {}", super::WINDOW_TITLE, last);
            let _ = self.canvas.window_mut().set_title(&title);
        }
    }

    fn frame(&mut self) -> f64 {
        let target = Duration::from_secs_f64(FRAME_SECONDS);
        let spent = self.last.elapsed();
        if spent < target {
            std::thread::sleep(target - spent);
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        dt.min(0.25)
    }
}

pub fn translate_events(raw: Vec<Event>) -> Vec<InputEvent> {
    let mut out = Vec::new();
    for event in raw {
        match event {
            Event::Quit { .. } => out.push(InputEvent::Quit),
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => out.push(InputEvent::FocusLost),
            Event::TextInput { text, .. } => {
                // SDL sends KEYDOWN before TEXTINPUT; attach the char to the
                // last KeyDown we produced this batch.
                if let Some(InputEvent::KeyDown { text: slot, .. }) = out.last_mut() {
                    *slot = text.chars().next();
                }
            }
            Event::KeyDown {
                keycode: Some(code),
                keymod,
                repeat: false,
                ..
            } => {
                out.push(InputEvent::KeyDown {
                    key: key_from_keycode(code),
                    mods: mods_from(keymod),
                    text: None,
                });
            }
            Event::KeyUp {
                keycode: Some(code),
                ..
            } => out.push(InputEvent::KeyUp {
                key: key_from_keycode(code),
            }),
            _ => {}
        }
    }
    out
}

pub fn mods_from(keymod: Mod) -> Mods {
    Mods {
        shift: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        ctrl: keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        alt: keymod.intersects(Mod::LALTMOD | Mod::RALTMOD),
    }
}

pub fn key_from_keycode(code: Keycode) -> Key {
    match code {
        Keycode::A => Key::A,
        Keycode::B => Key::B,
        Keycode::C => Key::C,
        Keycode::D => Key::D,
        Keycode::E => Key::E,
        Keycode::F => Key::F,
        Keycode::G => Key::G,
        Keycode::H => Key::H,
        Keycode::I => Key::I,
        Keycode::J => Key::J,
        Keycode::K => Key::K,
        Keycode::L => Key::L,
        Keycode::M => Key::M,
        Keycode::N => Key::N,
        Keycode::O => Key::O,
        Keycode::P => Key::P,
        Keycode::Q => Key::Q,
        Keycode::R => Key::R,
        Keycode::S => Key::S,
        Keycode::T => Key::T,
        Keycode::U => Key::U,
        Keycode::V => Key::V,
        Keycode::W => Key::W,
        Keycode::X => Key::X,
        Keycode::Y => Key::Y,
        Keycode::Z => Key::Z,
        Keycode::NUM_0 | Keycode::KP_0 => Key::Num0,
        Keycode::NUM_1 | Keycode::KP_1 => Key::Num1,
        Keycode::NUM_2 | Keycode::KP_2 => Key::Num2,
        Keycode::NUM_3 | Keycode::KP_3 => Key::Num3,
        Keycode::NUM_4 | Keycode::KP_4 => Key::Num4,
        Keycode::NUM_5 | Keycode::KP_5 => Key::Num5,
        Keycode::NUM_6 | Keycode::KP_6 => Key::Num6,
        Keycode::NUM_7 | Keycode::KP_7 => Key::Num7,
        Keycode::NUM_8 | Keycode::KP_8 => Key::Num8,
        Keycode::NUM_9 | Keycode::KP_9 => Key::Num9,
        Keycode::UP => Key::Up,
        Keycode::DOWN => Key::Down,
        Keycode::LEFT => Key::Left,
        Keycode::RIGHT => Key::Right,
        Keycode::RETURN | Keycode::KP_ENTER => Key::Return,
        Keycode::ESCAPE => Key::Escape,
        Keycode::SPACE => Key::Space,
        Keycode::BACKSPACE => Key::Backspace,
        Keycode::TAB => Key::Tab,
        Keycode::HOME => Key::Home,
        Keycode::END => Key::End,
        Keycode::PAGEUP => Key::PageUp,
        Keycode::PAGEDOWN => Key::PageDown,
        Keycode::F1 => Key::F1,
        Keycode::F2 => Key::F2,
        Keycode::PLUS | Keycode::KP_PLUS | Keycode::EQUALS => Key::Plus,
        Keycode::MINUS | Keycode::KP_MINUS => Key::Minus,
        _ => Key::Other,
    }
}

/// No window: events come from a queue the tests fill, and frames are
/// instantaneous with a fixed `dt`.
pub struct HeadlessShell {
    pub queued: VecDeque<InputEvent>,
    pub dt: f64,
    pub rendered: Vec<Vec<String>>,
}

impl HeadlessShell {
    pub fn new() -> Self {
        HeadlessShell {
            queued: VecDeque::new(),
            dt: FRAME_SECONDS,
            rendered: Vec::new(),
        }
    }
    pub fn push(&mut self, event: InputEvent) {
        self.queued.push_back(event);
    }
}

impl Default for HeadlessShell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell for HeadlessShell {
    fn poll(&mut self) -> Vec<InputEvent> {
        self.queued.pop_front().into_iter().collect()
    }
    fn render(&mut self, lines: &[String]) {
        self.rendered.push(lines.to_vec());
    }
    fn frame(&mut self) -> f64 {
        self.dt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_event_translates() {
        let raw = vec![Event::Quit { timestamp: 0 }];
        assert_eq!(translate_events(raw), vec![InputEvent::Quit]);
    }

    #[test]
    fn keydown_pairs_with_the_following_text_input() {
        let raw = vec![
            Event::KeyDown {
                timestamp: 0,
                window_id: 0,
                keycode: Some(Keycode::A),
                scancode: None,
                keymod: Mod::LSHIFTMOD,
                repeat: false,
            },
            Event::TextInput {
                timestamp: 0,
                window_id: 0,
                text: "A".to_string(),
            },
        ];
        let events = translate_events(raw);
        assert_eq!(
            events,
            vec![InputEvent::KeyDown {
                key: Key::A,
                mods: Mods {
                    shift: true,
                    ctrl: false,
                    alt: false
                },
                text: Some('A'),
            }]
        );
    }

    #[test]
    fn headless_shell_queues_and_replays_events() {
        let mut shell = HeadlessShell::new();
        shell.push(InputEvent::key(Key::Return));
        assert_eq!(shell.poll(), vec![InputEvent::key(Key::Return)]);
        assert_eq!(shell.poll(), vec![]);
        shell.render(&["hello".to_string()]);
        assert_eq!(shell.rendered, vec![vec!["hello".to_string()]]);
        assert_eq!(shell.frame(), FRAME_SECONDS);
    }
}
```

---

## 2. `src/app/input.rs` (verbatim)

`Key` (4-60) is a closed enum: A-Z, Num0-9, arrows, Return, Escape, Space,
Backspace, Tab, Home, End, PageUp, PageDown, F1, F2, Plus, Minus, Other.
No F3-F12, Delete, Insert, or punctuation beyond Plus/Minus. `Mods` (127-132)
is shift/ctrl/alt. `InputEvent` (134-146): `KeyDown { key, mods, text }`,
`KeyUp { key }`, `Quit`, `FocusLost`. Test constructors `InputEvent::key`
and `InputEvent::typed` (148-164). No key-binding names and no held-key
tracking live here (screens track held keys themselves, e.g. floor.rs
`self.held`).

`src/app/input.rs` (whole file, 196 lines):

```rust
//! Keyboard input as the states see it: a portable key enum, modifier
//! flags, and the typed character for text fields.

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
    Up,
    Down,
    Left,
    Right,
    Return,
    Escape,
    Space,
    Backspace,
    Tab,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    Plus,
    Minus,
    Other,
}

impl Key {
    pub fn from_char(c: char) -> Option<Key> {
        Some(match c.to_ascii_lowercase() {
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
            '\n' => Key::Return,
            '+' => Key::Plus,
            '-' => Key::Minus,
            _ => return None,
        })
    }

    /// The digit a number key stands for, if any.
    pub fn digit(self) -> Option<u8> {
        Some(match self {
            Key::Num0 => 0,
            Key::Num1 => 1,
            Key::Num2 => 2,
            Key::Num3 => 3,
            Key::Num4 => 4,
            Key::Num5 => 5,
            Key::Num6 => 6,
            Key::Num7 => 7,
            Key::Num8 => 8,
            Key::Num9 => 9,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    KeyDown {
        key: Key,
        mods: Mods,
        text: Option<char>,
    },
    KeyUp {
        key: Key,
    },
    Quit,
    FocusLost,
}

impl InputEvent {
    pub fn key(key: Key) -> Self {
        InputEvent::KeyDown {
            key,
            mods: Mods::default(),
            text: None,
        }
    }
    /// A printable character, as a text field receives it.
    pub fn typed(c: char) -> Self {
        InputEvent::KeyDown {
            key: Key::from_char(c).unwrap_or(Key::Other),
            mods: Mods::default(),
            text: Some(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_char_maps_letters_and_digits() {
        assert_eq!(Key::from_char('q'), Some(Key::Q));
        assert_eq!(Key::from_char('Q'), Some(Key::Q));
        assert_eq!(Key::from_char('5'), Some(Key::Num5));
        assert_eq!(Key::from_char('$'), None);
    }

    #[test]
    fn digit_reads_back_the_number_key() {
        assert_eq!(Key::Num5.digit(), Some(5));
        assert_eq!(Key::A.digit(), None);
    }

    #[test]
    fn typed_carries_the_character_and_matching_key() {
        let event = InputEvent::typed('a');
        assert_eq!(
            event,
            InputEvent::KeyDown {
                key: Key::A,
                mods: Mods::default(),
                text: Some('a')
            }
        );
    }
}
```

---

## 3. `src/app/paths.rs` (verbatim)

Casino-specific literals: env var `LUCKY_BREAK_DATA_DIR` (line 12), directory
name `LuckyBreakCasino` (14-15), file names `profile.json` (34),
`settings.json` (38), `luckybreak.log` (42), `credentials.json` (48).
`temp_for_tests` (28-31) is **not** `#[cfg(test)]`; it is called by
`App::headless_with_http` and by the playtest harness, which is why
`tempfile` is a normal dependency (section 8). Test at 75-84 depends on
`crate::online::testing::TempStore` (online/mod.rs:43-83), which sets
`LUCKY_BREAK_DATA_DIR` under a process-wide lock.

`src/app/paths.rs` (whole file, 93 lines):

```rust
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    /// Discover the data directory from `LUCKY_BREAK_DATA_DIR` env var
    /// or `dirs::data_local_dir()/LuckyBreakCasino`. Creates it if it doesn't exist.
    pub fn discover() -> Paths {
        let dir = std::env::var_os("LUCKY_BREAK_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|d| d.join("LuckyBreakCasino")))
            .unwrap_or_else(|| PathBuf::from("LuckyBreakCasino"));
        let _ = std::fs::create_dir_all(&dir);
        Paths { data_dir: dir }
    }

    /// Create a Paths struct with the given data directory. Creates it if it doesn't exist.
    pub fn at(dir: PathBuf) -> Paths {
        let _ = std::fs::create_dir_all(&dir);
        Paths { data_dir: dir }
    }

    /// Create a temporary directory for testing. Returns both the Paths and the TempDir
    /// (TempDir must be kept alive to keep the directory).
    pub fn temp_for_tests() -> (Paths, tempfile::TempDir) {
        let t = tempfile::tempdir().expect("tempdir");
        (Paths::at(t.path().to_path_buf()), t)
    }

    pub fn profile_file(&self) -> PathBuf {
        self.data_dir.join("profile.json")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("luckybreak.log")
    }

    /// Where the online credentials go when the system keyring cannot
    /// hold them.
    pub fn credentials_file(&self) -> PathBuf {
        self.data_dir.join("credentials.json")
    }
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dir_creates_and_paths_work() {
        let (paths, _temp) = Paths::temp_for_tests();
        assert!(paths.data_dir.exists());
        let profile_file = paths.profile_file();
        assert_eq!(profile_file.file_name().unwrap(), "profile.json");
        let settings_file = paths.settings_file();
        assert_eq!(settings_file.file_name().unwrap(), "settings.json");
        let log_file = paths.log_file();
        assert_eq!(log_file.file_name().unwrap(), "luckybreak.log");
    }

    /// `LUCKY_BREAK_DATA_DIR` is process-wide, so every test that leans
    /// on it takes the same turn: `TempStore` sets it, holds the lock
    /// the credential tests hold, and puts it back.
    #[test]
    fn lucky_break_data_dir_env_is_honoured() {
        let store = crate::online::testing::TempStore::new();
        let discovered = Paths::discover();
        assert_eq!(discovered.data_dir, store.dir());
        assert_eq!(
            discovered.credentials_file().file_name().unwrap(),
            "credentials.json"
        );
    }

    #[test]
    fn today_format_is_correct() {
        let today_str = today();
        assert_eq!(today_str.len(), 10); // YYYY-MM-DD
        assert_eq!(today_str.chars().nth(4).unwrap(), '-');
        assert_eq!(today_str.chars().nth(7).unwrap(), '-');
    }
}
```

---

## 4. `src/states/mod.rs` (verbatim)

What is in it:

- 6-21 `pub mod` for every screen; 28-37 re-exports.
- 41-64 `trait State`: hooks `name`, `enter`, `exit`, `resume` (default
  calls `enter`), `handle_input`, `on_event(&lb_core::protocol::Event)`,
  `update(dt)`, `lines`, `help`. Only `name` and `help` are required.
  **Coupling:** line 55 (and the `use` on 23) name `lb_core::protocol::Event`;
  the spec lifts this to `G::Event`.
- 66-130 `ESCAPE_RETRY_S` + `EscapeGuard`: two-press Escape that opens Pause
  when a table refuses a Leave. Mechanism is generic (clock-based), but its
  API names the sim's Leave/Error round trip. Used by table screens.
- 132-139 `enum StackOp { Push(Box<dyn State>), Pop, Replace(Box<dyn State>), Reset(Box<dyn State>) }`.
- 141-201 `track_profile`: casino (profile bankroll/comp/achievements).
- 203-243 `BootState`: a stub screen kept for the stack tests (used at
  `menu.rs:705`); its `enter` says `WINDOW_TITLE`, `update` quits after 5
  frames when `smoke`.

`src/states/mod.rs` (whole file, 243 lines):

```rust
//! The state stack. Every screen is a `State`; the stack is owned by
//! `App`, and a screen asks for a change through `GameContext` (which
//! queues a `StackOp`) so a handler never mutates the stack it is
//! running on.

pub mod achievements;
pub mod bar;
pub mod blackjack;
pub mod cashier;
pub mod creation;
pub mod floor;
pub mod help;
pub mod menu;
pub mod online;
pub mod pause;
pub mod quick_travel;
pub mod roulette;
pub mod settings;
pub mod slots;
pub mod title;
pub mod video_poker;

use lb_core::protocol::Event;

use crate::app::input::{InputEvent, Key};
use crate::app::GameContext;

pub use bar::BarState;
pub use blackjack::Blackjack;
pub use cashier::CashierState;
pub use floor::Floor;
pub use online::OnlineState;
pub use quick_travel::QuickTravel;
pub use roulette::RouletteState;
pub use slots::SlotsState;
pub use title::Title;
pub use video_poker::VideoPokerState;

/// One screen. Only `name` and `help` are required: a screen that does
/// nothing on a hook simply does not write it.
pub trait State {
    /// A stable identifier for logs, tests, and the playtest driver.
    fn name(&self) -> &'static str;
    /// Called when the screen is pushed onto the stack.
    fn enter(&mut self, _ctx: &mut GameContext) {}
    /// Called when the screen is popped or replaced.
    fn exit(&mut self, _ctx: &mut GameContext) {}
    /// Called when a screen above this one is popped and this one is
    /// revealed again. Announcing the screen afresh is the default.
    fn resume(&mut self, ctx: &mut GameContext) {
        self.enter(ctx)
    }
    fn handle_input(&mut self, _ctx: &mut GameContext, _event: &InputEvent) {}
    /// A simulation event, after the world view has absorbed it.
    fn on_event(&mut self, _ctx: &mut GameContext, _event: &lb_core::protocol::Event) {}
    fn update(&mut self, _ctx: &mut GameContext, _dt: f64) {}
    /// What a sighted helper sees in the window; the screen reader gets
    /// speech, not this.
    fn lines(&self, _ctx: &GameContext) -> Vec<String> {
        Vec::new()
    }
    /// Spoken on F1.
    fn help(&self) -> String;
}

/// How long after a refused Escape a second one opens the pause menu.
pub const ESCAPE_RETRY_S: f64 = 2.0;

/// The way out of a seat that will not let go.
///
/// Every table screen asks the sim to stand up and takes the refusal
/// gracefully -- "Finish the hand first." Should a screen and the sim
/// ever disagree about whether a hand is finished, that refusal is a
/// closed door with the player behind it, so a second Escape inside
/// [`ESCAPE_RETRY_S`] opens the pause menu instead. From there the
/// player can save and go back to the title, whatever the table thinks.
///
/// It is deliberately not a plain "Escape always pauses": leaving a
/// table is the common case and must not be shadowed by a pause menu.
#[derive(Default)]
pub struct EscapeGuard {
    /// A Leave has gone out and its answer has not come back.
    sent: bool,
    /// When the last refusal was heard, on `ctx.clock`.
    refused_at: Option<f64>,
}

impl EscapeGuard {
    /// Escape was pressed. `true` means the screen should push the pause
    /// menu rather than try to leave again.
    pub fn pressed(&mut self, ctx: &GameContext) -> bool {
        match self.refused_at {
            Some(at) if ctx.clock - at <= ESCAPE_RETRY_S => {
                self.refused_at = None;
                self.sent = false;
                true
            }
            _ => false,
        }
    }

    /// A `Leave` intent has just gone to the sim.
    pub fn leave_sent(&mut self) {
        self.sent = true;
    }

    /// The sim answered with an `Error`. When a Leave was outstanding,
    /// that error is the refusal, and the next Escape is the way out.
    pub fn error(&mut self, ctx: &GameContext) {
        if self.sent {
            self.sent = false;
            self.refused_at = Some(ctx.clock);
        }
    }

    /// The screen refused the Escape itself, without asking the sim.
    pub fn refused(&mut self, ctx: &GameContext) {
        self.sent = false;
        self.refused_at = Some(ctx.clock);
    }

    /// Any key but Escape. The player has gone back to playing, so the
    /// refusal is no longer the first half of a two-press exit and the
    /// next Escape is an ordinary "let me out of here" again.
    pub fn other_key(&mut self, key: &Key) {
        if !matches!(key, Key::Escape) {
            self.refused_at = None;
        }
    }
}

/// A change to the state stack, queued on the context and applied by
/// `App` once the current handler has returned.
pub enum StackOp {
    Push(Box<dyn State>),
    Pop,
    Replace(Box<dyn State>),
    Reset(Box<dyn State>),
}

/// The saved player's side of the ledger, kept in step with the world
/// view from the events themselves. `App::step` calls this for every
/// event before the top screen sees it, so the profile is right whichever
/// screen the player happens to be on, and the file is written whenever
/// the player stands up or crosses into another area.
pub fn track_profile(ctx: &mut GameContext, event: &Event) {
    let Some((me, bankroll)) = ctx
        .session
        .as_ref()
        .map(|s| (s.me.clone(), s.view.bankroll))
    else {
        return;
    };
    let today = ctx.today();
    let mut save = false;
    if let Some(profile) = ctx.profile.as_mut() {
        match event {
            // The stake from the pit boss is money and a day spent.
            Event::CompGranted { player, .. } if player == &me => {
                profile.bankroll = bankroll;
                profile.last_comp_day = Some(today);
            }
            // Every event that moves money: the view has already absorbed
            // it, so the new bankroll is simply copied across.
            Event::BankrollUpdate { player, .. }
            | Event::RoundResult { player, .. }
            | Event::SpinResult { player, .. }
            | Event::RouletteResult { player, .. }
            | Event::PokerResult { player, .. }
            | Event::DrinkServed { player, .. }
            | Event::Exchanged { player, .. }
            | Event::BetsCleared { player, .. }
                if player == &me =>
            {
                profile.bankroll = bankroll;
            }
            Event::CompPoints { player, points, .. } if player == &me => {
                profile.comp_points = *points;
            }
            Event::AchievementUnlocked {
                player,
                achievement,
            } if player == &me => {
                profile
                    .achievements
                    .entry(*achievement)
                    .or_insert_with(|| chrono::Utc::now().timestamp());
            }
            Event::RoomUnlocked { player, .. } if player == &me => {
                profile.high_limit_unlocked = true;
            }
            Event::Left { player, .. } | Event::AreaChanged { player, .. } if player == &me => {
                save = true;
            }
            _ => {}
        }
    }
    if save {
        ctx.save_profile();
    }
}

/// A bare screen kept for the state-stack tests: it names the game and,
/// under `--smoke`, quits after five frames.
pub struct BootState {
    smoke: bool,
    frames: u32,
}

impl BootState {
    pub fn new(smoke: bool) -> Self {
        BootState { smoke, frames: 0 }
    }
}

impl State for BootState {
    fn name(&self) -> &'static str {
        "boot"
    }

    fn enter(&mut self, ctx: &mut GameContext) {
        ctx.say(crate::app::WINDOW_TITLE);
    }

    fn update(&mut self, ctx: &mut GameContext, _dt: f64) {
        self.frames += 1;
        if self.smoke && self.frames >= 5 {
            ctx.quit();
        }
    }

    fn lines(&self, _ctx: &GameContext) -> Vec<String> {
        vec![format!(
            "{} {}",
            crate::app::WINDOW_TITLE,
            crate::app::VERSION
        )]
    }

    fn help(&self) -> String {
        "Lucky Break Casino is starting.".to_string()
    }
}
```

---

## 5. `src/app/mod.rs` (1134 lines)

### 5.1 Index

G = generic (hearken-app), C = casino-specific (stays in LB / moves to
`G::Data` hooks), O = online (hearken-online), T = test support.

| Lines | Item | Tag |
|---|---|---|
| 1-2 | module doc | |
| 4-6 | `pub mod input; pub mod paths; pub mod shell;` | G |
| 8-31 | imports (audio, discord, online, playtest, profile_store, session, settings, speech, states, updater, lb_core::profile) | mixed |
| 33 | `pub const WINDOW_TITLE: &str = "Lucky Break Casino";` | C (game-supplied value) |
| 34 | `pub const VERSION: &str = env!("CARGO_PKG_VERSION");` | C (game-supplied value) |
| 37 | `const TRANSCRIPT_CAP: usize = 2000;` | G |
| 41-42 | `pub type TestTranscript = Arc<Mutex<Vec<(String, bool)>>>;` `pub type TestCalls = Arc<Mutex<Vec<AudioCall>>>;` | T/G |
| 44-50 | `struct CliOptions { headless, smoke, seed, playtest }` | G (smoke/playtest belong to hearken-playtest) |
| 52-83 | `impl CliOptions { pub fn parse(args: &[String]) -> Result<Self, String> }` (53-82) | G |
| 85-96 | `pub struct OnlineStatus { credentials, presence, http }` | O |
| 98-132 | `impl OnlineStatus`: `new` 99-105, `connected` 107-109, `start_presence` 114-120, `stop_presence` 125-131 | O |
| 134-163 | `pub struct GameContext` | mixed (5.2) |
| 165-283 | `impl GameContext` (5.3) | mixed |
| 285-292 | `pub struct App { pub ctx: GameContext, states: Vec<Box<dyn State>>, presence_line: Option<(String, String)> }` | G (+O field) |
| 295-304 | `App::new(ctx, first: Box<dyn State>) -> App` | G |
| 309-311 | `App::headless_for_tests(seed) -> (App, TestTranscript, TestCalls, TempDir)` | T |
| 316-353 | `App::headless_with_http(seed, http: Arc<FakeHttp>) -> (...)` | T (C: voice clip set 325-330, Title 351; O: FakeHttp) |
| 358-431 | `App::step(&mut self, events: Vec<InputEvent>, dt: f64)` | mixed (5.5) |
| 438-460 | `fn pump_presence(&mut self)` | O |
| 465-474 | `fn pump_update_check(&mut self)` | O |
| 477-503 | `App::run(&mut self, shell: &mut dyn Shell)` | mixed (5.6) |
| 505-507 | `App::top_name(&self) -> &'static str` ("none" when empty) | G |
| 509-511 | `App::depth(&self) -> usize` | G |
| 514-519 | `App::top_lines(&self) -> Vec<String>` | G |
| 523-572 | `fn apply_pending(&mut self)` | G (5.7) |
| 579-650 | `pub fn build_context(opts: &CliOptions) -> GameContext` | mixed (5.8) |
| 656-696 | `fn init_logging(paths: &Paths)` | G |
| 699 | `pub const LOG_FILE_ENV: &str = "LUCKY_BREAK_LOG_FILE";` | G (name is C) |
| 704 | `struct Tee(Vec<Box<dyn log::Log>>);` | G |
| 706-722 | `impl log::Log for Tee` | G |
| 732-756 | `pub fn main_with(opts: CliOptions) -> i32` | mixed |
| 758-1134 | `#[cfg(test)] mod tests`: `Marker` 762-813, 20 tests | T |

### 5.2 `GameContext` struct (lines 134-163)

Field tags:

| Field | Type | Tag | Note |
|---|---|---|---|
| `speech` | `Box<dyn SpeechSink>` | G | trait at `speech/mod.rs:16-30` (say/stop/poll/available/name/set_rate) |
| `audio` | `Box<dyn Audio>` | G | trait at `audio/mod.rs:54-79` |
| `settings` | `Settings` | G, but struct has O fields | `settings.rs` |
| `paths` | `Paths` | G | |
| `profile` | `Option<lb_core::profile::Profile>` | C | -> `G::Data` |
| `session` | `Option<Session>` | C | -> `G::Data`; pumped in `step` |
| `running` | `bool` | G | |
| `transcript` | `Vec<String>` | G | capped at 2000 |
| `notices` | `Vec<String>` | G mechanism | filled by boot (corrupt profile), drained by `Title::enter` |
| `online` | `OnlineStatus` | O | |
| `version_available` | `Option<String>` | O | updater answer |
| `update_rx` | `Option<Receiver<Option<String>>>` | O | |
| `discord` | `Option<Discord>` | O | |
| `session_seed` | `Option<u64>` | G-ish | `--seed`; read by `title::new_seed` |
| `pending` | `Vec<StackOp>` (private) | G | the stack queue |
| `frame` | `u64` | G | |
| `clock` | `f64` | G | seconds, sum of dt; used by EscapeGuard |

`src/app/mod.rs` lines 134-163:

```rust
/// Everything a screen is allowed to touch: the two output devices, the
/// saved data, the live session, and the queue of stack changes.
pub struct GameContext {
    pub speech: Box<dyn SpeechSink>,
    pub audio: Box<dyn Audio>,
    pub settings: Settings,
    pub paths: Paths,
    pub profile: Option<Profile>,
    pub session: Option<Session>,
    pub running: bool,
    /// Every line spoken this run, for the playtest transcript.
    pub transcript: Vec<String>,
    /// Things to tell the player once a screen is ready to listen.
    pub notices: Vec<String>,
    pub online: OnlineStatus,
    pub version_available: Option<String>,
    /// The background update check's answer, once it lands. `None` once
    /// polled empty or once a reply has been taken -- there is only ever
    /// one message on this channel.
    pub update_rx: Option<Receiver<Option<String>>>,
    /// The Discord Rich Presence worker, when the feature is on and this
    /// build has an application id to present.
    pub discord: Option<Discord>,
    /// The seed a new session starts from: `--seed` when given, so a
    /// playtest run reproduces exactly.
    pub session_seed: Option<u64>,
    pending: Vec<StackOp>,
    pub frame: u64,
    pub clock: f64,
}
```

### 5.3 `impl GameContext` (lines 165-283)

`say` 167-171 (G, interrupt=true), `say_queued` 174-178 (G), `record` 180-190
(G; logs to target `lucky_break.transcript`, a C name), `stop_speech`
192-194 (G), `ui` 197-199 (G; always `Bus::Interface`), `push`/`pop`/`replace`/
`reset_to` 201-215 (G, the only way screens touch the stack), `quit` 217-219
(G), `save_profile` 221-227 (C), `set_online_presence` 237-247 (O),
`set_discord_presence` 257-268 (O), `apply_volumes` 271-277 (G: every `Bus`
level + speech rate), `today` 280-282 (C use, G helper).

`src/app/mod.rs` lines 165-283:

```rust
impl GameContext {
    /// Speak now, cutting off whatever is in progress.
    pub fn say(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.speech.say(&text, true);
        self.record(text);
    }

    /// Speak after whatever is in progress finishes.
    pub fn say_queued(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.speech.say(&text, false);
        self.record(text);
    }

    fn record(&mut self, text: String) {
        // `lucky_break.transcript` carries only spoken lines, so a
        // session's log file reads as a transcript of the playthrough
        // rather than a mix of that and every other diagnostic line.
        log::info!(target: "lucky_break.transcript", "{text}");
        self.transcript.push(text);
        if self.transcript.len() > TRANSCRIPT_CAP {
            let excess = self.transcript.len() - TRANSCRIPT_CAP;
            self.transcript.drain(..excess);
        }
    }

    pub fn stop_speech(&mut self) {
        self.speech.stop();
    }

    /// An interface one-shot: a menu tick, a confirmation, an error buzz.
    pub fn ui(&mut self, clip: &str) {
        self.audio.play(clip, Bus::Interface);
    }

    pub fn push(&mut self, s: impl State + 'static) {
        self.pending.push(StackOp::Push(Box::new(s)));
    }

    pub fn pop(&mut self) {
        self.pending.push(StackOp::Pop);
    }

    pub fn replace(&mut self, s: impl State + 'static) {
        self.pending.push(StackOp::Replace(Box::new(s)));
    }

    pub fn reset_to(&mut self, s: impl State + 'static) {
        self.pending.push(StackOp::Reset(Box::new(s)));
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn save_profile(&mut self) {
        if let Some(profile) = &self.profile {
            if let Err(e) = save_profile(&self.paths, profile) {
                log::error!("could not save the profile: {e}");
            }
        }
    }

    /// Turn presence on or off: the setting, the heartbeat, and the
    /// saved settings file, together.
    ///
    /// Every row that offers the switch goes through here -- the Online
    /// screen and both halves of the Settings row. Flipping the setting
    /// on its own would tell the player "off" while the heartbeat kept
    /// posting where they were, which is the one failure this feature
    /// must not have.
    pub fn set_online_presence(&mut self, on: bool) {
        self.settings.online_presence = on;
        if on {
            self.online.start_presence();
        } else {
            self.online.stop_presence();
        }
        if let Err(e) = self.settings.save(&self.paths) {
            log::warn!("could not save settings: {e}");
        }
    }

    /// Turn Discord Rich Presence on or off: the setting, the worker,
    /// and the saved settings file, together.
    ///
    /// The twin of [`set_online_presence`](Self::set_online_presence),
    /// and for the same reason: flipping the setting alone would leave a
    /// worker announcing the player's game to Discord after they had
    /// switched it off. With no application id built into this binary
    /// the worker is `None` either way and only the setting moves.
    pub fn set_discord_presence(&mut self, on: bool) {
        self.settings.discord = on;
        if let Some(discord) = self.discord.take() {
            discord.shutdown();
        }
        if on {
            self.discord = Discord::new(&self.settings);
        }
        if let Err(e) = self.settings.save(&self.paths) {
            log::warn!("could not save settings: {e}");
        }
    }

    /// Push the volume and speech-rate settings out to the devices.
    pub fn apply_volumes(&mut self) {
        for bus in Bus::ALL {
            let level = self.settings.level(bus);
            self.audio.set_volume(bus, level);
        }
        self.speech.set_rate(self.settings.speech_rate);
    }

    /// Today as `YYYY-MM-DD`, for the once-a-day comp.
    pub fn today(&self) -> String {
        paths::today()
    }
}
```

### 5.4 `App` struct, `new`, test constructors (lines 285-353)

`App::new` calls `first.enter(&mut ctx)` before the stack exists, then
`apply_pending()` so anything the first screen queued in `enter` is applied.

`src/app/mod.rs` lines 285-353:

```rust
/// The state stack and the frame loop that drives it.
pub struct App {
    pub ctx: GameContext,
    states: Vec<Box<dyn State>>,
    /// The last line handed to the presence heartbeat, so it is told
    /// only when the screen or the area under the player changes.
    presence_line: Option<(String, String)>,
}

impl App {
    pub fn new(mut ctx: GameContext, mut first: Box<dyn State>) -> App {
        first.enter(&mut ctx);
        let mut app = App {
            ctx,
            states: vec![first],
            presence_line: None,
        };
        app.apply_pending();
        app
    }

    /// An app wired to the capture speech sink and the recording audio
    /// backend, in a throwaway data directory. The returned handles read
    /// what was spoken and played; the `TempDir` must outlive the app.
    pub fn headless_for_tests(seed: u64) -> (App, TestTranscript, TestCalls, tempfile::TempDir) {
        App::headless_with_http(seed, Arc::new(FakeHttp::new()))
    }

    /// [`headless_for_tests`](Self::headless_for_tests) with the online
    /// client the test wants to watch. The caller keeps its own handle
    /// on the fake, so it can queue replies and read back every call.
    pub fn headless_with_http(
        seed: u64,
        http: Arc<FakeHttp>,
    ) -> (App, TestTranscript, TestCalls, tempfile::TempDir) {
        let (speech, transcript) = CaptureSpeech::new();
        let (mut audio, calls) = RecordingAudio::new();
        // Every voice line counts as present, so tests take the "clip
        // played" path by default; a test that wants the spoken fallback
        // clears the set.
        audio.present = lb_core::manifest::build()
            .clips
            .into_iter()
            .map(|c| c.id)
            .filter(|id| id.starts_with("voice/"))
            .collect();
        let (paths, temp) = Paths::temp_for_tests();
        let ctx = GameContext {
            speech: Box::new(speech),
            audio: Box::new(audio),
            settings: Settings::default(),
            paths,
            profile: None,
            session: None,
            running: true,
            transcript: Vec::new(),
            notices: Vec::new(),
            online: OnlineStatus::new(http),
            version_available: None,
            update_rx: None,
            discord: None,
            session_seed: Some(seed),
            pending: Vec::new(),
            frame: 0,
            clock: 0.0,
        };
        let app = App::new(ctx, Box::new(Title::new()));
        (app, transcript, calls, temp)
    }
```

### 5.5 `App::step` (lines 355-431)

Section tags:

- 359-387 input dispatch: G. `Quit` sets `running=false` and is not passed
  on; `FocusLost` is passed on; `F2` calls `stop_speech()` and is swallowed;
  `F1` speaks `top.help()` (interrupting) **and** is still passed to the
  screen (the menu queues the row help after it).
- 390 `apply_pending()`: G (a screen that pushed/popped from a key does not
  also get this frame's update).
- 392-408 session pump, `view.apply`, `track_profile`, `render::render`, then
  `top.on_event`: C. Generic shape: a game hook yields `G::Event`s, each is
  given to the top screen's `on_event`.
- 410-412 `top.update(dt)`: G (top screen only; screens below are frozen).
- 414-423 listener position + occlusion from `session.view.pos` and
  `lb_core::floor::line_of_sight`: C; the fallback `audio.update(dt, &|_| false)`
  (422) is G.
- 424 `speech.poll()`: G.
- 425-426 `pump_presence`, `pump_update_check`: O.
- 428-430 `apply_pending()`, `frame += 1`, `clock += dt`: G.

`src/app/mod.rs` lines 355-431:

```rust
    /// One frame: input to the top screen, the session's events to the
    /// world view and the screens, then update, audio, speech, and the
    /// stack changes the frame asked for.
    pub fn step(&mut self, events: Vec<InputEvent>, dt: f64) {
        for event in events {
            match &event {
                InputEvent::Quit => {
                    self.ctx.running = false;
                    continue;
                }
                // Losing focus is passed on: a screen holding a key down
                // has to let go of it, since no KeyUp is coming.
                InputEvent::FocusLost => {}
                // F2 silences speech everywhere, so no screen can hold
                // the key hostage.
                InputEvent::KeyDown { key: Key::F2, .. } => {
                    self.ctx.stop_speech();
                    continue;
                }
                // F1 reads the screen's help; the screen sees the key
                // too, so a menu can add the current row's help.
                InputEvent::KeyDown { key: Key::F1, .. } => {
                    if let Some(top) = self.states.last() {
                        let help = top.help();
                        self.ctx.say(help);
                    }
                }
                _ => {}
            }
            if let Some(top) = self.states.last_mut() {
                top.handle_input(&mut self.ctx, &event);
            }
        }
        // A screen that pushed or popped from a key must not also get
        // this frame's update.
        self.apply_pending();

        let sim_events = match self.ctx.session.as_mut() {
            Some(session) => session.pump(dt),
            None => Vec::new(),
        };
        for event in sim_events {
            if let Some(session) = self.ctx.session.as_mut() {
                let Session { me, view, .. } = session;
                view.apply(me, &event);
            }
            // The saved player is kept in step first, so every screen --
            // not just the floor -- banks what the event changed.
            crate::states::track_profile(&mut self.ctx, &event);
            crate::render::render(&mut self.ctx, &event);
            if let Some(top) = self.states.last_mut() {
                top.on_event(&mut self.ctx, &event);
            }
        }

        if let Some(top) = self.states.last_mut() {
            top.update(&mut self.ctx, dt);
        }

        match self.ctx.session.as_ref().map(|s| s.view.pos) {
            Some(me) => {
                // The ears go where the player is, every frame, so no
                // screen or event handler has to remember to do it.
                self.ctx.audio.set_listener(me);
                let occluded = move |pos| !lb_core::floor::line_of_sight(me, pos);
                self.ctx.audio.update(dt, &occluded);
            }
            None => self.ctx.audio.update(dt, &|_| false),
        }
        self.ctx.speech.poll();
        self.pump_presence();
        self.pump_update_check();

        self.apply_pending();
        self.ctx.frame += 1;
        self.ctx.clock += dt;
    }
```

### 5.6 `App::run` and helpers (lines 476-519)

Tags: 478-484 loop (poll, frame, step, render top_lines): G. 485
`save_profile`: C. 486-488 settings save: G. 489-501 presence/Discord
sign-off: O. 502 `audio.stop_all()`: G. Note the order: `poll` then
`frame` (the sleep), then `step`, then `render`.

`src/app/mod.rs` lines 476-519:

```rust
    /// Run until something quits, then save and silence the devices.
    pub fn run(&mut self, shell: &mut dyn Shell) {
        while self.ctx.running {
            let events = shell.poll();
            let dt = shell.frame();
            self.step(events, dt);
            let lines = self.top_lines();
            shell.render(&lines);
        }
        self.ctx.save_profile();
        if let Err(e) = self.ctx.settings.save(&self.ctx.paths) {
            log::error!("could not save settings: {e}");
        }
        // Leave the board before the process does. This one waits for
        // the sign-off rather than detaching it: there is no game loop
        // left to protect, and a detached thread would be killed on the
        // way out with the request half sent. The wait is bounded, and
        // the player is told it is happening rather than being left with
        // a window that has stopped responding.
        if let Some(presence) = self.ctx.online.presence.take() {
            self.ctx.say_queued("Signing off.");
            presence.shutdown();
        }
        if let Some(discord) = self.ctx.discord.take() {
            discord.shutdown();
        }
        self.ctx.audio.stop_all();
    }

    pub fn top_name(&self) -> &'static str {
        self.states.last().map(|s| s.name()).unwrap_or("none")
    }

    pub fn depth(&self) -> usize {
        self.states.len()
    }

    /// What the window shows: the top screen's lines.
    pub fn top_lines(&self) -> Vec<String> {
        self.states
            .last()
            .map(|s| s.lines(&self.ctx))
            .unwrap_or_default()
    }
```

### 5.7 How stack ops are applied: `apply_pending` (lines 521-572)

Semantics:

- Screens never touch `App.states`; they call `ctx.push/pop/replace/reset_to`
  (mod.rs 201-215), which append to `ctx.pending`.
- `apply_pending` runs in `App::new` (302) and twice per `step` (390 after
  input, 428 at end of frame).
- Ops apply in queue order. An `enter`/`exit`/`resume` may queue more; the
  outer loop drains up to 16 rounds, then logs an error and drops the rest.
- `Push`: `next.enter`, then push. `Pop`: pop + `gone.exit`, then
  `resume` on the newly revealed top (if any). `Replace`: pop + `exit`,
  `next.enter`, push (no `resume` of the screen below). `Reset`: pop every
  screen calling `exit` top-down, then `next.enter`, push.
- Only after the whole queue drains: an empty stack sets `running = false`.
  So `pop(); push(X)` on the root keeps running (test at 901-914).
- `pop(); push(X)` with a screen below: the one below gets `resume` and then
  X gets `enter`, in that order.

`src/app/mod.rs` lines 521-572:

```rust
    /// Apply the stack changes the frame queued. An `enter` may queue
    /// more, so this drains until the queue settles.
    fn apply_pending(&mut self) {
        for _ in 0..16 {
            if self.ctx.pending.is_empty() {
                break;
            }
            for op in std::mem::take(&mut self.ctx.pending) {
                match op {
                    StackOp::Push(mut next) => {
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                    }
                    StackOp::Pop => {
                        if let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        // Whether an empty stack ends the run is decided
                        // once the whole queue has drained: a handler is
                        // allowed to pop the root screen and push its
                        // replacement in the same breath.
                        if let Some(top) = self.states.last_mut() {
                            top.resume(&mut self.ctx);
                        }
                    }
                    StackOp::Replace(mut next) => {
                        if let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                    }
                    StackOp::Reset(mut next) => {
                        while let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                    }
                }
            }
        }
        if !self.ctx.pending.is_empty() {
            log::error!("stack operations kept queueing themselves; dropping the rest");
            self.ctx.pending.clear();
        }
        // Nothing left on the stack once everything queued has been
        // applied: there is no screen to run, so the game is over.
        if self.states.is_empty() {
            self.ctx.running = false;
        }
    }
```

### 5.8 `build_context` (lines 575-650)

Tags: 580 `Paths::discover` G; 581 `init_logging` G; 582 `Settings::load` G;
583 `choose_speech()` G (speech crate); 584 `choose_audio(&settings,
AssetStore::discover())` G (audio crate); 586-597 `load_profile` with a
notice on a corrupt file: C (the notice mechanism is G); 599-601 real HTTP
client + credentials: O; 603 Discord: O; 605-622 updater thread, skipped
when `opts.headless` or `LUCKY_BREAK_OFFLINE=1`: O; 624-642 construction;
645-647 presence start: O; 648 `apply_volumes`: G.

`src/app/mod.rs` lines 575-650:

```rust
/// The real context: the player's saved data, the screen reader, and the
/// audio engine. `--smoke` and `--playtest` never reach here -- they run
/// a script against a headless app with a throwaway data directory, so a
/// self-check cannot touch a saved game or open a screen reader.
pub fn build_context(opts: &CliOptions) -> GameContext {
    let paths = Paths::discover();
    init_logging(&paths);
    let settings = Settings::load(&paths);
    let speech: Box<dyn SpeechSink> = choose_speech();
    let audio: Box<dyn Audio> = choose_audio(&settings, AssetStore::discover());

    let mut notices = Vec::new();
    let profile = match load_profile(&paths) {
        LoadOutcome::Loaded(p) => Some(p),
        LoadOutcome::Fresh => None,
        LoadOutcome::Corrupt { moved_to } => {
            notices.push(format!(
                "Your saved profile could not be read. It was set aside as {}.",
                moved_to.display()
            ));
            None
        }
    };

    // The one place in the program that builds the real network client.
    let mut online = OnlineStatus::new(Arc::new(UreqHttp));
    online.credentials = load_credentials();

    let discord = Discord::new(&settings);

    // The update check reaches the real network, so it is skipped for a
    // `--headless` run (tests, playtests, CI) and whenever the player has
    // asked to stay offline. It runs on its own thread and never blocks
    // boot: the answer arrives through `update_rx`, polled from the frame
    // loop once it exists.
    let update_rx = if !opts.headless && !updater::offline() {
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("lb-updater".to_string())
            .spawn(move || {
                let version = updater::check_latest(&UreqHttp);
                let _ = tx.send(version);
            })
            .is_ok();
        spawned.then_some(rx)
    } else {
        None
    };

    let mut ctx = GameContext {
        speech,
        audio,
        settings,
        paths,
        profile,
        session: None,
        running: true,
        transcript: Vec::new(),
        notices,
        online,
        version_available: None,
        update_rx,
        discord,
        session_seed: opts.seed,
        pending: Vec::new(),
        frame: 0,
        clock: 0.0,
    };
    // Presence is opt-in and needs credentials; both have to be true
    // before a single request goes out.
    if ctx.settings.online_presence {
        ctx.online.start_presence();
    }
    ctx.apply_volumes();
    ctx
}
```

### 5.9 Logging: `init_logging`, `LOG_FILE_ENV`, `Tee` (lines 652-722)

stderr always; plus the log file (`LUCKY_BREAK_LOG_FILE` or
`paths.log_file()`), opened append+create. Both are `env_logger` loggers
(default filter `info`, overridable by `RUST_LOG` through
`env_logger::Env::default()`), fanned out by `Tee`. Max level is the max of
the two filters. Failure to open the file falls back to stderr-only with an
`eprintln!`.

`src/app/mod.rs` lines 652-722:

```rust
/// Every log record goes to stderr; when a file is available (from
/// `LUCKY_BREAK_LOG_FILE`, or `paths.log_file()` by default) it goes
/// there too, so a finished session leaves a transcript behind without
/// the player needing to have piped the window themselves.
fn init_logging(paths: &Paths) {
    let log_path = std::env::var_os(LOG_FILE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.log_file());
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => Some(f),
        Err(e) => {
            // Logging is not wired up yet at this point, so this reaches
            // the player only via the terminal env_logger falls back to
            // once it is initialised below -- fine, since it is rare and
            // stderr keeps working regardless.
            eprintln!("could not open the log file {}: {e}", log_path.display());
            None
        }
    };

    let stderr_logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    match file {
        Some(f) => {
            let file_logger =
                env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
                    .target(env_logger::Target::Pipe(Box::new(f)))
                    .build();
            let level = stderr_logger.filter().max(file_logger.filter());
            log::set_max_level(level);
            let _ = log::set_boxed_logger(Box::new(Tee(vec![
                Box::new(stderr_logger),
                Box::new(file_logger),
            ])));
        }
        None => {
            log::set_max_level(stderr_logger.filter());
            let _ = log::set_boxed_logger(Box::new(stderr_logger));
        }
    }
}

/// Override for where the session log goes. Unset, it is `paths.log_file()`.
pub const LOG_FILE_ENV: &str = "LUCKY_BREAK_LOG_FILE";

/// Fans one log record out to every sink. `env_logger`'s own `Builder`
/// only ever targets one writer, and a session log wants stderr and the
/// file at once.
struct Tee(Vec<Box<dyn log::Log>>);

impl log::Log for Tee {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.0.iter().any(|l| l.enabled(metadata))
    }

    fn log(&self, record: &log::Record) {
        for l in &self.0 {
            l.log(record);
        }
    }

    fn flush(&self) {
        for l in &self.0 {
            l.flush();
        }
    }
}
```

### 5.10 `CliOptions` (lines 44-83) and `main_with` (724-756)

Flags: `--headless`; `--smoke` (implies headless); `--seed N`; `--playtest FILE`
(implies headless). Unknown flag is an error; `main.rs` prints it and exits 2.
`main_with`: `--smoke` and `--playtest` never build a real context; they
run `playtest::run_text/run_file` against `App::headless_for_tests(seed)`
(default seed `playtest::DEFAULT_SEED = 42`). Otherwise `build_context`
first (so logging exists), then `SdlShell::new(WINDOW_TITLE)` or
`HeadlessShell::new()`; a window failure logs and returns 2. First screen is
`Title::new()` (C).

`src/app/mod.rs` lines 44-83:

```rust
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CliOptions {
    pub headless: bool,
    pub smoke: bool,
    pub seed: Option<u64>,
    pub playtest: Option<PathBuf>,
}

impl CliOptions {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut o = CliOptions::default();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--headless" => o.headless = true,
                "--smoke" => {
                    o.smoke = true;
                    o.headless = true;
                }
                "--seed" => {
                    i += 1;
                    o.seed = Some(
                        args.get(i)
                            .ok_or("--seed needs a number")?
                            .parse()
                            .map_err(|_| "--seed needs a number")?,
                    );
                }
                "--playtest" => {
                    i += 1;
                    o.playtest = Some(PathBuf::from(args.get(i).ok_or("--playtest needs a file")?));
                    o.headless = true;
                }
                other => return Err(format!("unknown option {other}")),
            }
            i += 1;
        }
        Ok(o)
    }
}
```

`src/app/mod.rs` lines 724-756:

```rust
/// Boot and run. Returns the process exit code.
///
/// The two scripted modes come first and never build a context at all:
/// `--smoke` runs the embedded self-check and `--playtest` runs a script
/// from a file, both against a headless app in a throwaway data
/// directory. Otherwise `build_context` runs ahead of the shell: it is
/// what resolves the data directory and turns on logging, and a window
/// that fails to open should still be able to log why.
pub fn main_with(opts: CliOptions) -> i32 {
    let seed = opts.seed.unwrap_or(playtest::DEFAULT_SEED);
    if opts.smoke {
        return playtest::report("smoke", &playtest::run_text(playtest::SMOKE_SCRIPT, seed));
    }
    if let Some(path) = opts.playtest.as_deref() {
        let outcome = playtest::run_file(path, seed);
        return playtest::report(&path.display().to_string(), &outcome);
    }
    let ctx = build_context(&opts);
    let mut shell: Box<dyn Shell> = if opts.headless {
        Box::new(HeadlessShell::new())
    } else {
        match SdlShell::new(WINDOW_TITLE) {
            Ok(s) => Box::new(s),
            Err(e) => {
                log::error!("could not open a window: {e}");
                return 2;
            }
        }
    };
    let mut app = App::new(ctx, Box::new(Title::new()));
    app.run(shell.as_mut());
    0
}
```

`src/main.rs` (whole file, 14 lines):

```rust
#![cfg_attr(windows, windows_subsystem = "windows")]
//! `luckybreak`: the game's entry point.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match lucky_break::app::CliOptions::parse(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    std::process::exit(lucky_break::app::main_with(opts));
}
```

---

## 6. Menus, built-in screens, settings

### 6.1 `src/states/menu.rs`: LB's menu abstraction

Shape: a screen struct embeds `MenuCore<Self>`, implements `Menu`
(`menu()`, `menu_mut()`, `build_items(&mut self, ctx) -> Vec<MenuItem<Self>>`),
and gets its `State` impl from `crate::impl_state_for_menu!(Type, "name")`.
Rows are closures over `(&mut S, &mut GameContext)`.

How it speaks and sounds (all through `GameContext`):

- enter (168-174): `refresh(ctx, false)` (cursor to 0), play `open_sound` if
  set, `announce`: `"{title}. {n} items. {row}"` (`"1 item"` singular) when
  the cursor is at 0, else `"{title}. {row}"` (227-244). Spoken with `say`
  (interrupt).
- resume (181-184): rebuild rows keeping the cursor, re-announce, no sound.
- Up/Down (363-364): wrap-around move, `ui("sfx/ui_move")`, `say(row)`.
  Home/End (365-369): jump to edges, same sound + speech.
- Return or Space (370): `ui("sfx/ui_select")`, run the row's action.
- Left/Right (371-372): only on a row with an adjuster: `ui("sfx/ui_move")`,
  adjuster gets -1/+1, then `say(row)`; rows without one are silent.
- Escape (373-376): `ui("sfx/ui_back")`, then `on_escape` (default
  `ctx.pop()`; the root Title overrides it, title.rs:237-246).
- F1 (379-383): App already said the screen's `intro_help` (interrupt); the
  menu `say_queued`s the current row's help.
- Any other key: first-letter jump on the typed alphanumeric char or the
  key's letter/digit (`letter_of`, 397-401); wraps; silent if nothing matches.
- `lines()` (194-202): title, then each row prefixed `"> "` (cursor) or `"  "`.
- Rows are lent to an action/adjuster with `mem::take`; the `revision`
  counter (111-116) tells whether the callback installed new rows
  (`set_items` bumps it), in which case the lent rows are dropped.

Coupling points: `on_event` (190) and the macro (432-438) name
`lb_core::protocol::Event`; the macro hard-codes `$crate::app::GameContext`
and `$crate::states::State`; sound ids `"sfx/ui_move"`, `"sfx/ui_select"`,
`"sfx/ui_back"` are literals (272, 282, 305, 317, 342, 374).
`DEFAULT_INTRO_HELP` (119-120). No text-entry state exists in LB: the one
text field is inline in `states/creation.rs` `Creation::name_input`
(119-155: printable chars appended and echoed, Backspace speaks the removed
char, Return confirms, Escape pops).

Non-test portion, verbatim (lines 1-452):

`src/states/menu.rs` lines 1-452:

```rust
//! The accessible menu: a list of rows that is navigated by ear. Every
//! move speaks the row it lands on and ticks the interface bus, Enter
//! runs the row's action, Left and Right adjust a row that has an
//! adjuster (a volume, a bet size) and speak the new value, and F1 reads
//! the row's help.
//!
//! A screen embeds a [`MenuCore`], implements [`Menu`] (two accessors and
//! `build_items`), and gets its whole `State` impl from
//! [`impl_state_for_menu!`](crate::impl_state_for_menu).

use crate::app::input::{InputEvent, Key};
use crate::app::GameContext;

/// A label computed while the menu is open. Rows whose text depends on
/// state that changes under them (a volume, a bankroll) must be read
/// through this and never off a string captured when the menu was built.
pub type DynamicLabel<S> = Box<dyn Fn(&S, &GameContext) -> String>;
/// What activating a row does.
pub type MenuAction<S> = Box<dyn FnMut(&mut S, &mut GameContext)>;
/// What Left and Right do to a row, given -1 or +1.
pub type MenuAdjust<S> = Box<dyn FnMut(&mut S, &mut GameContext, i32)>;

/// A row's label or help: fixed text, or text computed on demand.
pub enum Label<S> {
    Text(String),
    Dynamic(DynamicLabel<S>),
}

impl<S> Label<S> {
    pub fn dynamic(f: impl Fn(&S, &GameContext) -> String + 'static) -> Self {
        Label::Dynamic(Box::new(f))
    }

    pub fn text(&self, state: &S, ctx: &GameContext) -> String {
        match self {
            Label::Text(text) => text.clone(),
            Label::Dynamic(f) => f(state, ctx),
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

/// One row of a menu.
pub struct MenuItem<S> {
    label: Label<S>,
    help: Option<Label<S>>,
    action: MenuAction<S>,
    /// Set when Left and Right change this row instead of moving off it.
    pub adjust: Option<MenuAdjust<S>>,
}

impl<S: 'static> MenuItem<S> {
    pub fn new(
        label: impl Into<Label<S>>,
        action: impl FnMut(&mut S, &mut GameContext) + 'static,
    ) -> Self {
        MenuItem {
            label: label.into(),
            help: None,
            action: Box::new(action),
            adjust: None,
        }
    }

    /// A row that does nothing when activated: a heading or a readout.
    pub fn inert(label: impl Into<Label<S>>) -> Self {
        MenuItem::new(label, |_, _| {})
    }

    pub fn help(mut self, help: impl Into<Label<S>>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Give the row a Left/Right adjuster.
    pub fn adjustable(mut self, f: impl FnMut(&mut S, &mut GameContext, i32) + 'static) -> Self {
        self.adjust = Some(Box::new(f));
        self
    }

    pub fn text(&self, state: &S, ctx: &GameContext) -> String {
        self.label.text(state, ctx)
    }

    pub fn help_text(&self, state: &S, ctx: &GameContext) -> Option<String> {
        self.help.as_ref().map(|h| h.text(state, ctx))
    }
}

/// The reusable half of a menu screen: the title, the rows, and where
/// the cursor is.
pub struct MenuCore<S> {
    pub title: String,
    pub intro_help: String,
    pub items: Vec<MenuItem<S>>,
    pub index: usize,
    /// Played when the menu is entered. `None` for a menu whose opening
    /// is already covered by another sound.
    pub open_sound: Option<&'static str>,
    /// Bumped every time the rows are replaced. `activate` and `adjust`
    /// lend the rows to a callback and compare this afterwards to tell
    /// a callback that installed its own rows from one that did not.
    /// Anything that assigns `items` directly must bump it; `set_items`
    /// does both.
    pub revision: u64,
}

pub const DEFAULT_INTRO_HELP: &str =
    "Up and Down move, Enter chooses, Escape goes back, F2 stops speech.";

impl<S> MenuCore<S> {
    pub fn new(title: impl Into<String>) -> Self {
        MenuCore {
            title: title.into(),
            intro_help: DEFAULT_INTRO_HELP.to_string(),
            items: Vec::new(),
            index: 0,
            open_sound: None,
            revision: 0,
        }
    }

    /// Replace the rows and mark them changed.
    pub fn set_items(&mut self, items: Vec<MenuItem<S>>) {
        self.items = items;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn with_intro_help(mut self, help: impl Into<String>) -> Self {
        self.intro_help = help.into();
        self
    }

    pub fn with_open_sound(mut self, clip: Option<&'static str>) -> Self {
        self.open_sound = clip;
        self
    }
}

impl<S> Default for MenuCore<S> {
    fn default() -> Self {
        MenuCore::new("Menu")
    }
}

/// A vertically navigated, fully spoken menu. Implement the two
/// accessors and `build_items`; everything else has a default that the
/// screen may override, and `impl_state_for_menu!` forwards the `State`
/// hooks here.
pub trait Menu: Sized + 'static {
    fn menu(&self) -> &MenuCore<Self>;
    fn menu_mut(&mut self) -> &mut MenuCore<Self>;
    fn build_items(&mut self, ctx: &mut GameContext) -> Vec<MenuItem<Self>>;

    // -- State hooks, forwarded by impl_state_for_menu! ---------------------

    fn enter(&mut self, ctx: &mut GameContext) {
        self.refresh(ctx, false);
        if let Some(clip) = self.menu().open_sound {
            ctx.ui(clip);
        }
        self.announce(ctx);
    }

    fn exit(&mut self, _ctx: &mut GameContext) {}

    /// Revealed again after a screen above it closed: rebuild the rows
    /// (a setting may have changed) and re-announce without the opening
    /// sound.
    fn resume(&mut self, ctx: &mut GameContext) {
        self.refresh(ctx, true);
        self.announce(ctx);
    }

    fn handle_input(&mut self, ctx: &mut GameContext, event: &InputEvent) {
        menu_input(self, ctx, event)
    }

    fn on_event(&mut self, _ctx: &mut GameContext, _event: &lb_core::protocol::Event) {}

    fn update(&mut self, _ctx: &mut GameContext, _dt: f64) {}

    fn lines(&self, ctx: &GameContext) -> Vec<String> {
        let core = self.menu();
        let mut out = vec![core.title.clone()];
        for (i, item) in core.items.iter().enumerate() {
            let marker = if i == core.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out
    }

    /// What Escape does. Closing the screen is the default, so a menu
    /// at the root of the stack (the title screen) must override this:
    /// popping the only screen ends the run.
    fn on_escape(&mut self, ctx: &mut GameContext) {
        ctx.pop()
    }

    // -- menu behaviour -----------------------------------------------------

    /// Rebuild the rows from `build_items`, keeping the cursor where it
    /// was when `keep_index` is set.
    fn refresh(&mut self, ctx: &mut GameContext, keep_index: bool) {
        let items = self.build_items(ctx);
        let core = self.menu_mut();
        core.set_items(items);
        if !keep_index {
            core.index = 0;
        }
        if core.index >= core.items.len() {
            core.index = 0;
        }
    }

    /// `"Title. N items. First row"` on a fresh open, `"Title. Row"`
    /// when the cursor is already somewhere.
    fn announce(&mut self, ctx: &mut GameContext) {
        let title = self.menu().title.clone();
        let n = self.menu().items.len();
        let label = self.current_text(ctx);
        let text = if self.menu().index == 0 {
            let items = if n == 1 {
                "1 item".to_string()
            } else {
                format!("{n} items")
            };
            format!("{title}. {items}. {label}")
        } else {
            format!("{title}. {label}")
        };
        ctx.say(text);
    }

    fn current_text(&self, ctx: &GameContext) -> String {
        let core = self.menu();
        match core.items.get(core.index) {
            Some(item) => item.text(self, ctx),
            None => "No options".to_string(),
        }
    }

    fn current_help(&self, ctx: &GameContext) -> Option<String> {
        let core = self.menu();
        core.items.get(core.index)?.help_text(self, ctx)
    }

    fn speak_current(&mut self, ctx: &mut GameContext) {
        let text = self.current_text(ctx);
        ctx.say(text);
    }

    /// Move the cursor by `delta`, wrapping at both ends.
    fn move_by(&mut self, ctx: &mut GameContext, delta: i32) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        let here = self.menu().index as i32;
        self.menu_mut().index = (here + delta).rem_euclid(n as i32) as usize;
        ctx.ui("sfx/ui_move");
        self.speak_current(ctx);
    }

    fn jump(&mut self, ctx: &mut GameContext, index: usize) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        self.menu_mut().index = index.min(n - 1);
        ctx.ui("sfx/ui_move");
        self.speak_current(ctx);
    }

    /// The next row after the cursor whose label starts with `c`,
    /// wrapping. Silent when nothing matches.
    fn letter_jump(&mut self, ctx: &mut GameContext, c: char) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        let start = self.menu().index;
        let wanted = c.to_ascii_lowercase();
        for offset in 1..=n {
            let i = (start + offset) % n;
            let text = self.menu().items[i].text(self, ctx);
            if text
                .chars()
                .next()
                .map(|f| f.to_ascii_lowercase() == wanted)
                .unwrap_or(false)
            {
                self.menu_mut().index = i;
                ctx.ui("sfx/ui_move");
                self.speak_current(ctx);
                return;
            }
        }
    }

    fn activate(&mut self, ctx: &mut GameContext) {
        let i = self.menu().index;
        if i >= self.menu().items.len() {
            return;
        }
        ctx.ui("sfx/ui_select");
        // The action needs `&mut self`, so the rows are lent to it and
        // handed back afterwards -- unless the action installed rows of
        // its own (a row that rebuilds its own menu), which the
        // revision counter is what tells us.
        let revision = self.menu().revision;
        let mut items = std::mem::take(&mut self.menu_mut().items);
        (items[i].action)(self, ctx);
        if self.menu().revision == revision {
            self.menu_mut().items = items;
        }
    }

    /// Left and Right: change the row in place and read it back. A row
    /// with no adjuster does and says nothing.
    fn adjust(&mut self, ctx: &mut GameContext, delta: i32) {
        let i = self.menu().index;
        if !self
            .menu()
            .items
            .get(i)
            .is_some_and(|item| item.adjust.is_some())
        {
            return;
        }
        ctx.ui("sfx/ui_move");
        let revision = self.menu().revision;
        let mut items = std::mem::take(&mut self.menu_mut().items);
        if let Some(adjust) = items[i].adjust.as_mut() {
            adjust(self, ctx, delta);
        }
        if self.menu().revision == revision {
            self.menu_mut().items = items;
        }
        self.speak_current(ctx);
    }
}

/// The navigation every menu shares. `Menu::handle_input` calls this;
/// a screen with extra keys overrides `handle_input` and calls it for
/// everything it does not claim.
pub fn menu_input<S: Menu>(state: &mut S, ctx: &mut GameContext, event: &InputEvent) {
    let InputEvent::KeyDown { key, text, .. } = event else {
        return;
    };
    match key {
        Key::Down => state.move_by(ctx, 1),
        Key::Up => state.move_by(ctx, -1),
        Key::Home => state.jump(ctx, 0),
        Key::End => {
            let last = state.menu().items.len().saturating_sub(1);
            state.jump(ctx, last);
        }
        Key::Return | Key::Space => state.activate(ctx),
        Key::Left => state.adjust(ctx, -1),
        Key::Right => state.adjust(ctx, 1),
        Key::Escape => {
            ctx.ui("sfx/ui_back");
            state.on_escape(ctx);
        }
        // `App::step` has already spoken the screen's intro help; the
        // row's own help follows it.
        Key::F1 => {
            if let Some(help) = state.current_help(ctx) {
                ctx.say_queued(help);
            }
        }
        other => {
            let typed = (*text)
                .filter(|c| c.is_alphanumeric())
                .or_else(|| letter_of(*other));
            if let Some(c) = typed {
                state.letter_jump(ctx, c);
            }
        }
    }
}

/// The character a letter or digit key stands for, so first-letter jumps
/// work even when the shell reported no typed text.
fn letter_of(key: Key) -> Option<char> {
    ('a'..='z')
        .chain('0'..='9')
        .find(|&c| Key::from_char(c) == Some(key))
}

/// Write the `State` impl for a `Menu` screen, forwarding every hook to
/// the `Menu` method of the same name.
///
/// ```ignore
/// impl_state_for_menu!(TitleState, "title");
/// ```
#[macro_export]
macro_rules! impl_state_for_menu {
    ($ty:ty, $name:literal) => {
        impl $crate::states::State for $ty {
            fn name(&self) -> &'static str {
                $name
            }
            fn enter(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::menu::Menu>::enter(self, ctx)
            }
            fn exit(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::menu::Menu>::exit(self, ctx)
            }
            fn resume(&mut self, ctx: &mut $crate::app::GameContext) {
                <Self as $crate::states::menu::Menu>::resume(self, ctx)
            }
            fn handle_input(
                &mut self,
                ctx: &mut $crate::app::GameContext,
                event: &$crate::app::input::InputEvent,
            ) {
                <Self as $crate::states::menu::Menu>::handle_input(self, ctx, event)
            }
            fn on_event(
                &mut self,
                ctx: &mut $crate::app::GameContext,
                event: &lb_core::protocol::Event,
            ) {
                <Self as $crate::states::menu::Menu>::on_event(self, ctx, event)
            }
            fn update(&mut self, ctx: &mut $crate::app::GameContext, dt: f64) {
                <Self as $crate::states::menu::Menu>::update(self, ctx, dt)
            }
            fn lines(&self, ctx: &$crate::app::GameContext) -> Vec<String> {
                <Self as $crate::states::menu::Menu>::lines(self, ctx)
            }
            fn help(&self) -> String {
                <Self as $crate::states::menu::Menu>::menu(self)
                    .intro_help
                    .clone()
            }
        }
    };
}
```

Menu test fixture (used by section 7), lines 454-530:

`src/states/menu.rs` lines 454-530:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, TestCalls, TestTranscript};
    use crate::audio::{AudioCall, Bus};

    /// Three rows: Alpha adjusts, Beta does not, Gamma rebuilds the
    /// menu (and gives a first-letter jump somewhere to land).
    struct TestMenu {
        menu: MenuCore<Self>,
        activated: u32,
        volume: i32,
        /// Set by Gamma's action, which then rebuilds the rows.
        grown: bool,
    }

    impl TestMenu {
        fn new() -> Self {
            TestMenu {
                menu: MenuCore::new("Test menu"),
                activated: 0,
                volume: 70,
                grown: false,
            }
        }
    }

    impl Menu for TestMenu {
        fn menu(&self) -> &MenuCore<Self> {
            &self.menu
        }
        fn menu_mut(&mut self) -> &mut MenuCore<Self> {
            &mut self.menu
        }
        fn build_items(&mut self, _ctx: &mut GameContext) -> Vec<MenuItem<Self>> {
            let mut items = vec![
                MenuItem::new(
                    Label::dynamic(|s: &TestMenu, _| {
                        if s.volume == 70 {
                            "Alpha".to_string()
                        } else {
                            format!("Alpha: {}", s.volume)
                        }
                    }),
                    |s: &mut TestMenu, _| s.activated += 1,
                )
                .help("Alpha help")
                .adjustable(|s: &mut TestMenu, _, delta| s.volume += delta * 10),
                MenuItem::inert("Beta").help("Beta help"),
                MenuItem::new("Gamma", |s: &mut TestMenu, ctx| {
                    s.grown = true;
                    Menu::refresh(s, ctx, true);
                }),
            ];
            if self.grown {
                items.push(MenuItem::inert("Delta"));
            }
            items
        }
    }

    crate::impl_state_for_menu!(TestMenu, "test_menu");

    /// An app with the test menu open and the boot state's own speech
    /// already flushed from the transcript.
    fn open_menu() -> (App, TestTranscript, TestCalls, tempfile::TempDir) {
        let (mut app, speech, audio, temp) = App::headless_for_tests(7);
        app.ctx.push(TestMenu::new());
        app.step(vec![], 0.0);
        speech.lock().clear();
        audio.lock().clear();
        (app, speech, audio, temp)
    }

    fn spoken(lines: &TestTranscript) -> Vec<String> {
        lines.lock().iter().map(|(t, _)| t.clone()).collect()
    }
```

### 6.2 `src/states/settings.rs`: `SettingsState`

Constructed with `SettingsState::new()` (also `Default`); pushed from
`title.rs:160` and `pause.rs:44` as `ctx.push(SettingsState::new())`.
Rows (built each refresh): one per `Bus::ALL` (6 volumes, adjust by 5, clamp
0-100, applied live via `ctx.apply_volumes()`), speech rate (0.1 step,
0.5-2.0, rounded to 1 decimal, applied via `ctx.speech.set_rate`), Online
presence (O), Discord presence (O), Audio backend (`auto`/`null`, next
launch). Escape (`on_escape`, 187-193) saves `settings.json` then pops; that
is the only save point on this screen. Online-specific rows: 130-165.

Non-test portion, verbatim (lines 1-204):

`src/states/settings.rs` lines 1-204:

```rust
//! The settings screen: six volume buses, the speech rate, the two
//! presence switches, and the audio backend. Every change is applied the
//! moment it is made, so the player hears what they are choosing; Escape
//! writes `settings.json`.

use crate::app::GameContext;
use crate::audio::Bus;
use crate::settings::Settings;
use crate::states::menu::{Label, Menu, MenuCore, MenuItem};

/// A volume step, in percent.
const VOLUME_STEP: i32 = 5;
/// The speech-rate step, and the range Prism accepts.
const RATE_STEP: f64 = 0.1;
const RATE_MIN: f64 = 0.5;
const RATE_MAX: f64 = 2.0;

pub fn bus_label(bus: Bus) -> &'static str {
    match bus {
        Bus::Master => "Master",
        Bus::Music => "Music",
        Bus::Ambience => "Ambience",
        Bus::Effects => "Effects",
        Bus::Voices => "Voices",
        Bus::Interface => "Interface",
    }
}

fn bus_value(settings: &Settings, bus: Bus) -> u8 {
    match bus {
        Bus::Master => settings.master,
        Bus::Music => settings.music,
        Bus::Ambience => settings.ambience,
        Bus::Effects => settings.effects,
        Bus::Voices => settings.voices,
        Bus::Interface => settings.interface,
    }
}

fn set_bus(settings: &mut Settings, bus: Bus, value: u8) {
    let slot = match bus {
        Bus::Master => &mut settings.master,
        Bus::Music => &mut settings.music,
        Bus::Ambience => &mut settings.ambience,
        Bus::Effects => &mut settings.effects,
        Bus::Voices => &mut settings.voices,
        Bus::Interface => &mut settings.interface,
    };
    *slot = value;
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// Round a rate to one decimal so the label and the stored value agree.
fn round_rate(rate: f64) -> f64 {
    (rate * 10.0).round() / 10.0
}

pub struct SettingsState {
    menu: MenuCore<Self>,
}

impl SettingsState {
    pub fn new() -> Self {
        SettingsState {
            menu: MenuCore::new("Settings").with_intro_help(
                "Up and Down move, Left and Right change the setting under the cursor, \
                 Escape saves and goes back.",
            ),
        }
    }
}

impl Default for SettingsState {
    fn default() -> Self {
        SettingsState::new()
    }
}

impl Menu for SettingsState {
    fn menu(&self) -> &MenuCore<Self> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut GameContext) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = Bus::ALL
            .into_iter()
            .map(|bus| {
                MenuItem::inert(Label::dynamic(
                    move |_s: &SettingsState, ctx: &GameContext| {
                        format!("{}: {}", bus_label(bus), bus_value(&ctx.settings, bus))
                    },
                ))
                .help("Left and Right change this volume by five.")
                .adjustable(
                    move |_s: &mut SettingsState, ctx: &mut GameContext, delta| {
                        let next = (bus_value(&ctx.settings, bus) as i32 + delta * VOLUME_STEP)
                            .clamp(0, 100) as u8;
                        set_bus(&mut ctx.settings, bus, next);
                        ctx.apply_volumes();
                    },
                )
            })
            .collect();

        items.push(
            MenuItem::inert(Label::dynamic(|_s: &SettingsState, ctx: &GameContext| {
                format!("Speech rate: {:.1}", ctx.settings.speech_rate)
            }))
            .help("Left and Right change how fast the speech is read, from 0.5 to 2.0.")
            .adjustable(|_s: &mut SettingsState, ctx: &mut GameContext, delta| {
                let next = round_rate(
                    (ctx.settings.speech_rate + delta as f64 * RATE_STEP).clamp(RATE_MIN, RATE_MAX),
                );
                ctx.settings.speech_rate = next;
                ctx.speech.set_rate(next);
            }),
        );

        items.push(
            MenuItem::new(
                Label::dynamic(|_s: &SettingsState, ctx: &GameContext| {
                    format!("Online presence: {}", on_off(ctx.settings.online_presence))
                }),
                |_s: &mut SettingsState, ctx: &mut GameContext| {
                    // Through the context, so the heartbeat is actually
                    // started or stopped: saying "off" while the thread
                    // kept posting would be a lie about a privacy switch.
                    ctx.set_online_presence(!ctx.settings.online_presence);
                    let text = format!("Online presence: {}", on_off(ctx.settings.online_presence));
                    ctx.say(text);
                },
            )
            .help("Whether other players can see that you are playing. Enter turns it on or off.")
            .adjustable(|_s: &mut SettingsState, ctx: &mut GameContext, _delta| {
                ctx.set_online_presence(!ctx.settings.online_presence);
            }),
        );

        items.push(
            MenuItem::new(
                Label::dynamic(|_s: &SettingsState, ctx: &GameContext| {
                    format!("Discord presence: {}", on_off(ctx.settings.discord))
                }),
                |_s: &mut SettingsState, ctx: &mut GameContext| {
                    ctx.set_discord_presence(!ctx.settings.discord);
                    let text = format!("Discord presence: {}", on_off(ctx.settings.discord));
                    ctx.say(text);
                },
            )
            .help("Whether Discord shows what you are playing. Enter turns it on or off.")
            .adjustable(|_s: &mut SettingsState, ctx: &mut GameContext, _delta| {
                ctx.set_discord_presence(!ctx.settings.discord);
            }),
        );

        items.push(
            MenuItem::new(
                Label::dynamic(|_s: &SettingsState, ctx: &GameContext| {
                    format!("Audio backend: {}", ctx.settings.audio_backend)
                }),
                |_s: &mut SettingsState, ctx: &mut GameContext| {
                    let next = next_backend(&ctx.settings.audio_backend);
                    ctx.settings.audio_backend = next.to_string();
                    ctx.say(format!("Audio backend: {next}. Takes effect next launch."));
                },
            )
            .help("Auto plays sound; null runs the game silently. Takes effect next launch.")
            .adjustable(|_s: &mut SettingsState, ctx: &mut GameContext, _delta| {
                ctx.settings.audio_backend = next_backend(&ctx.settings.audio_backend).to_string();
            }),
        );

        items
    }

    /// Escape is the only way out, so it is where the file is written.
    fn on_escape(&mut self, ctx: &mut GameContext) {
        if let Err(e) = ctx.settings.save(&ctx.paths) {
            log::error!("could not save settings: {e}");
        }
        ctx.pop();
    }
}

fn next_backend(current: &str) -> &'static str {
    if current == "null" {
        "auto"
    } else {
        "null"
    }
}

crate::impl_state_for_menu!(SettingsState, "settings");
```

### 6.3 `src/states/help.rs`: `HelpState`

Constructed with `HelpState::new()` (also `Default`); pushed from
`title.rs:181` and `pause.rs:48`. Content is `docs/help.md` at the repo
root, compiled in with `include_str!("../../../../docs/help.md")` (line 10),
split on `## ` headings into `Section { title, body }` (body = non-blank
lines joined by spaces). One menu row per section; Enter `say`s the body.
The handbook has 11 sections (test at 97-117). Distinct from F1, which
`App::step` handles for every screen via `State::help`.

Non-test portion, verbatim (lines 1-89):

`src/states/help.rs` lines 1-89:

```rust
//! The help screen: `docs/help.md` read out a section at a time. The
//! document ships inside the binary, so help works before any asset or
//! data directory exists.

use crate::app::GameContext;
use crate::states::menu::{Menu, MenuCore, MenuItem};

/// The player handbook, compiled in. `crates/lucky-break/src/states/` up
/// four levels is the repository root.
const HELP_DOC: &str = include_str!("../../../../docs/help.md");

/// One `## ` heading and the prose under it.
pub struct Section {
    pub title: &'static str,
    pub body: String,
}

/// Split the handbook on `## ` headings. Anything before the first one
/// (the document's own title) is not a section.
pub fn sections() -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in HELP_DOC.lines() {
        match line.strip_prefix("## ") {
            Some(title) => out.push(Section {
                title: title.trim(),
                body: String::new(),
            }),
            None => {
                let Some(current) = out.last_mut() else {
                    continue;
                };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if !current.body.is_empty() {
                    current.body.push(' ');
                }
                current.body.push_str(line);
            }
        }
    }
    out
}

pub struct HelpState {
    menu: MenuCore<Self>,
}

impl HelpState {
    pub fn new() -> Self {
        HelpState {
            menu: MenuCore::new("Help").with_intro_help(
                "Up and Down move between help topics, Enter reads one, Escape goes back.",
            ),
        }
    }
}

impl Default for HelpState {
    fn default() -> Self {
        HelpState::new()
    }
}

impl Menu for HelpState {
    fn menu(&self) -> &MenuCore<Self> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut GameContext) -> Vec<MenuItem<Self>> {
        sections()
            .into_iter()
            .map(|section| {
                let body = section.body;
                MenuItem::new(section.title, move |_s: &mut HelpState, ctx| {
                    ctx.say(body.clone());
                })
                .help("Press Enter to hear this topic.")
            })
            .collect()
    }
}

crate::impl_state_for_menu!(HelpState, "help");
```

### 6.4 `src/states/pause.rs`: `Pause`

Constructed with `Pause::new()` (also `Default`), title "Paused",
`open_sound = Some("sfx/ui_select")`. Rows: Resume (pop), Settings (push),
Help (push), "Save and return to the title" (C: `save_profile`,
`session = None`, `reset_to(Title::new())`), Quit (C: `save_profile`, then
`quit`). Escape uses the default `on_escape` (pop). Pushed from the game
screens on Escape: `bar.rs:114`, `blackjack.rs:266` (via `EscapeGuard`),
`cashier.rs:106`, `floor.rs:485`, `roulette.rs:363`, `slots.rs:121`,
`video_poker.rs:143`.

Non-test portion, verbatim (lines 1-69):

`src/states/pause.rs` lines 1-69:

```rust
//! The pause menu: the way out of a game in progress, and the only
//! place the player can put the session down without losing it.

use crate::app::GameContext;
use crate::states::help::HelpState;
use crate::states::menu::{Menu, MenuCore, MenuItem};
use crate::states::settings::SettingsState;
use crate::states::title::Title;

pub struct Pause {
    menu: MenuCore<Self>,
}

impl Pause {
    pub fn new() -> Self {
        Pause {
            menu: MenuCore::new("Paused")
                .with_intro_help("Up and Down move, Enter chooses, Escape goes back to the game.")
                .with_open_sound(Some("sfx/ui_select")),
        }
    }
}

impl Default for Pause {
    fn default() -> Self {
        Pause::new()
    }
}

impl Menu for Pause {
    fn menu(&self) -> &MenuCore<Self> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut GameContext) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::new("Resume", |_s: &mut Pause, ctx: &mut GameContext| ctx.pop())
                .help("Go back to where you were."),
            MenuItem::new("Settings", |_s: &mut Pause, ctx: &mut GameContext| {
                ctx.push(SettingsState::new())
            })
            .help("Volumes, speech rate, and presence."),
            MenuItem::new("Help", |_s: &mut Pause, ctx: &mut GameContext| {
                ctx.push(HelpState::new())
            })
            .help("How the casino and the games work."),
            MenuItem::new(
                "Save and return to the title",
                |_s: &mut Pause, ctx: &mut GameContext| {
                    ctx.save_profile();
                    ctx.session = None;
                    ctx.reset_to(Title::new());
                },
            )
            .help("Save your player and leave the casino floor."),
            MenuItem::new("Quit", |_s: &mut Pause, ctx: &mut GameContext| {
                ctx.save_profile();
                ctx.quit();
            })
            .help("Save and close the game."),
        ]
    }
}

crate::impl_state_for_menu!(Pause, "pause");
```

### 6.5 Root menu override, for reference: `Title` (states/title.rs)

`Title` is the stack root. It overrides `Menu::enter` (208-222: status line
with `say`, boot notices with `say_queued`, then the menu announcement
queued), `update` (224-235: rebuilds when the update check lands), and
`on_escape` (237-246, never pops):

`src/states/title.rs` lines 208-248:

```rust
    /// The status line comes first, then the menu queues behind it, so
    /// the player hears the whole greeting without it being cut off.
    fn enter(&mut self, ctx: &mut GameContext) {
        ctx.audio.play_music(Some("music/title_theme"), true);
        let status = self.status_line(ctx);
        ctx.say(status);
        for notice in std::mem::take(&mut ctx.notices) {
            ctx.say_queued(notice);
        }
        self.refresh(ctx, false);
        let title = self.menu().title.clone();
        let n = self.menu().items.len();
        let label = self.current_text(ctx);
        ctx.say_queued(format!("{title}. {n} items. {label}"));
    }

    fn update(&mut self, ctx: &mut GameContext, _dt: f64) {
        // The background update check can land while the title is
        // already the top screen -- pick it up here rather than only on
        // `enter`, so a player sitting on the menu still hears about it.
        if ctx.version_available != self.known_version {
            self.known_version = ctx.version_available.clone();
            self.refresh(ctx, false);
            if let Some(version) = self.known_version.clone() {
                ctx.say_queued(format!("Version {version} is available."));
            }
        }
    }

    /// The root of the stack: Escape backs out of the confirmation and
    /// otherwise says how to leave. It must never pop.
    fn on_escape(&mut self, ctx: &mut GameContext) {
        if self.confirming {
            self.cancel_confirm(ctx);
        } else {
            ctx.say("Press Enter on Quit to leave.");
        }
    }
}

crate::impl_state_for_menu!(Title, "title");
```

### 6.6 `src/settings.rs` (verbatim)

Format: pretty JSON (`serde_json::to_string_pretty`), every field
`#[serde(default = ...)]` so partial files load; unknown fields are ignored
(no `deny_unknown_fields`). Location: `paths.settings_file()` =
`<data_dir>/settings.json`. `load`: missing file -> defaults silently; parse
error -> `log::warn!` + defaults (the bad file is not set aside and is
overwritten on the next save). Saved from: settings screen Escape
(states/settings.rs:189), `App::run` exit (app/mod.rs:486),
`set_online_presence` (244), `set_discord_presence` (265). Coupling:
`level()` (117-119) takes `crate::audio::Bus`; `bus_percent` maps by the
bus name string. Online fields: `online_presence`, `discord`.

Defaults: master 100; music, ambience, effects, voices, interface 80;
speech_rate 1.0; online_presence false; discord true; audio_backend "auto".

`src/settings.rs` (whole file, 201 lines):

```rust
use serde::{Deserialize, Serialize};
use std::io;

use crate::app::paths::Paths;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Settings {
    #[serde(default = "default_master")]
    pub master: u8,
    #[serde(default = "default_music")]
    pub music: u8,
    #[serde(default = "default_ambience")]
    pub ambience: u8,
    #[serde(default = "default_effects")]
    pub effects: u8,
    #[serde(default = "default_voices")]
    pub voices: u8,
    #[serde(default = "default_interface")]
    pub interface: u8,
    #[serde(default = "default_speech_rate")]
    pub speech_rate: f64,
    #[serde(default = "default_online_presence")]
    pub online_presence: bool,
    #[serde(default = "default_discord")]
    pub discord: bool,
    #[serde(default = "default_audio_backend")]
    pub audio_backend: String,
}

fn default_master() -> u8 {
    100
}
fn default_music() -> u8 {
    80
}
fn default_ambience() -> u8 {
    80
}
fn default_effects() -> u8 {
    80
}
fn default_voices() -> u8 {
    80
}
fn default_interface() -> u8 {
    80
}
fn default_speech_rate() -> f64 {
    1.0
}
fn default_online_presence() -> bool {
    false
}
fn default_discord() -> bool {
    true
}
fn default_audio_backend() -> String {
    "auto".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            master: 100,
            music: 80,
            ambience: 80,
            effects: 80,
            voices: 80,
            interface: 80,
            speech_rate: 1.0,
            online_presence: false,
            discord: true,
            audio_backend: "auto".to_string(),
        }
    }
}

impl Settings {
    /// Load settings from the file. Returns defaults if the file doesn't exist
    /// or cannot be parsed (with a log warning).
    pub fn load(paths: &Paths) -> Settings {
        let file = paths.settings_file();
        match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("could not parse settings file: {e}; using defaults");
                    Settings::default()
                }
            },
            Err(_) => Settings::default(),
        }
    }

    /// Save settings to file as pretty JSON.
    pub fn save(&self, paths: &Paths) -> io::Result<()> {
        let file = paths.settings_file();
        let json = serde_json::to_string_pretty(self).expect("settings serialize");
        std::fs::write(&file, json)
    }

    /// Get the volume level for a bus (master, music, ambience, effects, voices, interface)
    /// as a percentage (0-100).
    pub fn bus_percent(&self, name: &str) -> u8 {
        match name {
            "master" => self.master,
            "music" => self.music,
            "ambience" => self.ambience,
            "effects" => self.effects,
            "voices" => self.voices,
            "interface" => self.interface,
            _ => 100,
        }
    }

    /// The bus volume as a fraction (0.0..=1.0), for the audio backends.
    pub fn level(&self, bus: crate::audio::Bus) -> f64 {
        self.bus_percent(bus.name()) as f64 / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let s = Settings::default();
        assert_eq!(s.master, 100);
        assert_eq!(s.music, 80);
        assert_eq!(s.ambience, 80);
        assert_eq!(s.effects, 80);
        assert_eq!(s.voices, 80);
        assert_eq!(s.interface, 80);
        assert_eq!(s.speech_rate, 1.0);
        assert!(!s.online_presence);
        assert!(s.discord);
        assert_eq!(s.audio_backend, "auto");
    }

    #[test]
    fn save_and_load_round_trip() {
        let (paths, _temp) = Paths::temp_for_tests();
        let s1 = Settings {
            master: 50,
            music: 60,
            ..Default::default()
        };
        s1.save(&paths).expect("save");
        let s2 = Settings::load(&paths);
        assert_eq!(s1, s2);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let (paths, _temp) = Paths::temp_for_tests();
        let s = Settings::load(&paths);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn partial_json_loads_with_defaults() {
        let (paths, _temp) = Paths::temp_for_tests();
        let partial = r#"{"master": 50}"#;
        std::fs::write(paths.settings_file(), partial).expect("write partial json");
        let s = Settings::load(&paths);
        assert_eq!(s.master, 50);
        assert_eq!(s.music, 80);
        assert_eq!(s.ambience, 80);
    }

    #[test]
    fn bus_percent_returns_correct_values() {
        let s = Settings {
            master: 100,
            music: 60,
            ambience: 70,
            effects: 80,
            voices: 90,
            interface: 50,
            ..Default::default()
        };
        assert_eq!(s.bus_percent("master"), 100);
        assert_eq!(s.bus_percent("music"), 60);
        assert_eq!(s.bus_percent("ambience"), 70);
        assert_eq!(s.bus_percent("effects"), 80);
        assert_eq!(s.bus_percent("voices"), 90);
        assert_eq!(s.bus_percent("interface"), 50);
        assert_eq!(s.bus_percent("unknown"), 100);
    }

    #[test]
    fn level_divides_bus_percent_by_100() {
        let s = Settings {
            music: 60,
            ..Default::default()
        };
        assert_eq!(s.level(crate::audio::Bus::Music), 0.6);
        assert_eq!(s.level(crate::audio::Bus::Master), 1.0);
    }
}
```

---

## 7. Tests that exercise shell/stack/menu/settings headlessly

### 7.1 How headless tests drive the app

Three ways, in order of how often they are used:

1. **Direct `App::step`** (almost every unit test). `App::headless_for_tests(seed)`
   (app/mod.rs:309) returns `(App, TestTranscript, TestCalls, TempDir)`:
   a `CaptureSpeech` (speech/mod.rs:59-103) whose `(text, interrupt)` pairs
   land in `TestTranscript`, a `RecordingAudio` (audio/null.rs:68) whose
   calls land in `TestCalls` as `AudioCall` values, `Settings::default()`,
   a temp data dir, and `Title` already entered. Tests call
   `app.ctx.push(...)`/`app.step(vec![InputEvent::key(Key::Down)], 0.0)` and
   read `speech.lock()`, `audio.lock()`, `app.top_name()`, `app.depth()`,
   `app.top_lines()`, `app.ctx.transcript`, `app.ctx.settings`, or files in
   `temp.path()`. The Shell is not involved at all. Typical helper: clear
   `speech`/`audio` after opening the screen so assertions see only the
   keystroke's output (menu.rs:519-526, settings.rs:213-225).
2. **`HeadlessShell` + `App::run`** (2 tests: app/mod.rs:994-1009 and
   1100-1126). `shell.push(InputEvent::Quit)`; `app.run(&mut shell)`;
   `shell.rendered` holds each frame's `top_lines()`; one queued event per
   frame. Plus shell.rs:272-281 tests the shell on its own.
3. **Playtest scripts** (`src/playtest.rs`, used by `--smoke`, `--playtest`,
   and `tests/playtest.rs`). `run_text(text, seed)` builds
   `App::headless_for_tests(seed)` and `run_script` steps it at
   `FRAME = 1/60`: `key K` = one frame with `[KeyDown K, KeyUp K]`
   (161-164); `type TEXT` = one frame with `InputEvent::typed` per char
   (165); `wait S` = `round(S/FRAME)` empty frames (166-171); `expect X`
   searches `app.ctx.transcript[mark..]` for a line containing X and moves
   `mark` past it (174-188); `debug bankroll N` (C). `Outcome { passed,
   failures, transcript }`. Script keys by name: Return, Escape, Up, Down,
   Left, Right, Space, F1, Home, End, Backspace, or one alphanumeric char
   (126-148).

### 7.2 Test inventory (file, count, names)

- `src/app/shell.rs` (3): quit_event_translates, keydown_pairs_with_the_following_text_input, headless_shell_queues_and_replays_events
- `src/app/input.rs` (3): from_char_maps_letters_and_digits, digit_reads_back_the_number_key, typed_carries_the_character_and_matching_key
- `src/app/paths.rs` (3): temp_dir_creates_and_paths_work, lucky_break_data_dir_env_is_honoured, today_format_is_correct
- `src/app/mod.rs` (20; `Marker` helper screen at 762-813). Stack/loop (G): cli_parses_flags, smoke_boot_runs_and_exits_zero, push_and_pop_track_the_top_and_the_depth, stack_ops_from_a_handler_apply_after_it_returns, replace_swaps_the_top_and_reset_empties_the_stack, popping_the_last_screen_ends_the_run, a_pop_and_a_push_in_one_batch_keep_the_game_running, say_interrupts_and_say_queued_does_not, the_transcript_is_capped, ui_plays_on_the_interface_bus, quit_event_stops_the_run_and_f1_speaks_the_help, frame_and_clock_count_up, run_stops_when_a_screen_quits_and_saves_the_profile, apply_volumes_pushes_every_bus_out. Casino/online: the_listener_follows_the_player_every_frame, the_frame_loop_tells_presence_what_changed_and_nothing_else, presence_off_means_the_loop_says_nothing, the_discord_switch_never_starts_a_worker_without_an_app_id, quitting_with_presence_on_says_it_is_signing_off, today_is_a_calendar_day
- `src/states/menu.rs` (16; `TestMenu` fixture 460-530): entering_announces_title_count_and_first_item, down_moves_and_wraps, up_wraps_backwards, home_and_end_go_to_the_edges, typing_a_letter_jumps_to_that_item, enter_runs_the_action_and_plays_the_select_sound, an_action_that_rebuilds_the_rows_keeps_the_new_ones, an_adjuster_that_rebuilds_the_rows_keeps_the_new_ones, escape_pops_the_menu_and_plays_the_back_sound, f1_speaks_the_intro_help_then_the_item_help, left_and_right_adjust_and_read_the_new_label, adjust_on_a_row_without_one_says_nothing, every_move_ticks_the_interface_bus, lines_mark_the_cursor, resume_re_announces_with_the_cursor_kept, label_from_string_and_dynamic_read_the_same_way
- `src/states/settings.rs` (7): right_on_music_raises_it_by_five_and_applies_it_live, escape_writes_the_settings_file, a_volume_stops_at_the_ends_of_its_range, the_speech_rate_moves_by_a_tenth_inside_its_range, the_switches_toggle_on_enter_and_on_left_or_right, the_presence_switch_starts_and_stops_the_heartbeat (O), the_audio_backend_cycles_between_auto_and_null
- `src/settings.rs` (6): default_values, save_and_load_round_trip, missing_file_gives_defaults, partial_json_loads_with_defaults, bus_percent_returns_correct_values, level_divides_bus_percent_by_100
- `src/states/help.rs` (3): the_handbook_has_its_eleven_sections, opening_names_the_first_topic_and_enter_reads_it, escape_closes_the_help
- `src/states/pause.rs` (3, all need a C session/profile): saving_and_returning_ends_the_session_and_resets_to_the_title, escape_and_resume_both_go_back_to_the_game, quit_saves_and_stops_the_run
- `src/states/title.rs` (12; root-menu behaviour): the_game_boots_to_a_spoken_title_with_music, with_no_player_there_is_nothing_to_continue_and_enter_makes_one, a_saved_player_gets_a_continue_row_that_opens_the_floor, new_game_over_a_saved_player_asks_first, yes_start_over_opens_character_creation_and_keeps_the_old_player_until_it_finishes, escape_on_the_root_menu_never_ends_the_run, an_update_landing_while_the_title_is_on_top_is_spoken_once, the_update_row_only_appears_when_there_is_an_update, a_notice_from_the_boot_is_spoken_once, quit_closes_the_game, the_status_line_counts_missing_clips, the_seed_comes_from_the_command_line_when_given
- `src/states/creation.rs` (5; the only text field): a_whole_player_is_made_saved_and_walked_onto_the_floor, backspace_names_the_character_it_removed, an_empty_name_is_refused_and_a_long_one_is_capped, escape_walks_back_through_the_steps_and_then_off_the_screen, the_lines_follow_the_step
- `src/playtest.rs` (8): every_verb_parses, an_unknown_verb_is_a_parse_error_with_the_line, a_two_step_script_passes_against_the_title_screen, a_missing_line_fails_with_the_line_number_and_what_was_searched, expects_walk_forward_through_one_frame, debug_before_a_profile_or_after_a_session_fails, a_missing_file_is_a_failure_not_a_panic, the_embedded_smoke_script_parses
- `tests/playtest.rs` (10, integration): smoke, walk_every_area, blackjack_round, slots_spins, roulette_spin, video_poker_hand, bar_and_cashier, broke_and_comped, unlock_high_limit, every_script_has_a_test. Scripts in `tests/scripts/*.txt` (9 files); `smoke.txt` is also embedded via `include_str!` (playtest.rs:38).
- Not relevant here: `tests/assets_present.rs`, `tests/manifest_ids.rs` (asset/manifest checks).

Every `App::headless_for_tests` test boots the real `Title` as the root, so
generic stack/menu tests in hearken-app need their own root screen (e.g.
`Marker`/`BootState`) instead.

### 7.3 Representative test 1: stack ops apply after the handler returns

`Marker` helper (app/mod.rs 762-813) and the test (858-870):

`src/app/mod.rs` lines 762-813:

```rust
    /// A screen that writes down every hook it is given.
    struct Marker {
        name: &'static str,
        log: Arc<Mutex<Vec<String>>>,
        /// Pushed when this screen sees a key.
        pushes: Option<&'static str>,
    }

    impl Marker {
        fn new(name: &'static str, log: &Arc<Mutex<Vec<String>>>) -> Marker {
            Marker {
                name,
                log: Arc::clone(log),
                pushes: None,
            }
        }
        fn pushing(mut self, name: &'static str) -> Marker {
            self.pushes = Some(name);
            self
        }
        fn note(&self, what: &str) {
            self.log.lock().push(format!("{what} {}", self.name));
        }
    }

    impl State for Marker {
        fn name(&self) -> &'static str {
            self.name
        }
        fn enter(&mut self, _ctx: &mut GameContext) {
            self.note("enter");
        }
        fn exit(&mut self, _ctx: &mut GameContext) {
            self.note("exit");
        }
        fn resume(&mut self, _ctx: &mut GameContext) {
            self.note("resume");
        }
        fn handle_input(&mut self, ctx: &mut GameContext, _event: &InputEvent) {
            if let Some(name) = self.pushes {
                ctx.push(Marker::new(name, &self.log));
            }
            self.note("handled");
        }
        fn update(&mut self, _ctx: &mut GameContext, _dt: f64) {}
        fn lines(&self, _ctx: &GameContext) -> Vec<String> {
            vec![self.name.to_string()]
        }
        fn help(&self) -> String {
            format!("{} help", self.name)
        }
    }
```

`src/app/mod.rs` lines 858-870:

```rust
    #[test]
    fn stack_ops_from_a_handler_apply_after_it_returns() {
        let (mut app, _speech, _audio, _t) = App::headless_for_tests(1);
        let log = Arc::new(Mutex::new(Vec::new()));
        app.ctx.push(Marker::new("alpha", &log).pushing("beta"));
        app.step(vec![], 0.0);
        log.lock().clear();

        app.step(vec![InputEvent::key(Key::Return)], 0.0);
        // The handler finished before the pushed screen entered.
        assert_eq!(*log.lock(), vec!["handled alpha", "enter beta"]);
        assert_eq!(app.top_name(), "beta");
    }
```

### 7.4 Representative test 2: F1 speaks intro help, then row help

Uses the `open_menu`/`spoken` helpers shown in 6.1 (menu.rs 517-530):

`src/states/menu.rs` lines 632-640:

```rust
    #[test]
    fn f1_speaks_the_intro_help_then_the_item_help() {
        let (mut app, speech, _audio, _t) = open_menu();
        app.step(vec![InputEvent::key(Key::F1)], 0.0);
        assert_eq!(spoken(&speech), vec![DEFAULT_INTRO_HELP, "Alpha help"]);
        // The intro interrupts, the row's help queues behind it.
        assert!(speech.lock()[0].1);
        assert!(!speech.lock()[1].1);
    }
```

The HeadlessShell-driven test, for reference (app/mod.rs 994-1009):

`src/app/mod.rs` lines 994-1009:

```rust
    #[test]
    fn run_stops_when_a_screen_quits_and_saves_the_profile() {
        let (mut app, _speech, _audio, temp) = App::headless_for_tests(1);
        app.ctx.profile = Some(Profile::new(
            "Jo",
            lb_core::protocol::Footwear::Boots,
            lb_core::protocol::AddressStyle::Friend,
            1000,
        ));
        let mut shell = HeadlessShell::new();
        shell.push(InputEvent::Quit);
        app.run(&mut shell);
        assert!(!app.ctx.running);
        assert!(temp.path().join("profile.json").exists());
        assert!(temp.path().join("settings.json").exists());
    }
```

---

## 8. Cargo.toml

`crates/lucky-break/Cargo.toml` (whole file). No `[features]` section. One
lib (`lucky_break`, src/lib.rs) and one bin (`luckybreak`, src/main.rs).
`build = "build.rs"`: on Windows it adds `vendor/sdl2/<os>-<arch>/` to the
link path and copies the SDL2 DLL next to the build output; macOS/Linux use
the `bundled` + `static-link` sdl2 features instead.

`Cargo.toml` (whole file, 54 lines):

```toml
[package]
name = "lucky-break"
description = "Lucky Break Casino: the accessible, audio-first casino game"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
build = "build.rs"

[lib]
name = "lucky_break"
path = "src/lib.rs"

[[bin]]
name = "luckybreak"
path = "src/main.rs"

[lints]
workspace = true

[dependencies]
lb-core = { path = "../lb-core", features = ["test-hooks"] }
prism = { path = "../prism" }
bass-sys = { path = "../bass-sys" }
anyhow.workspace = true
thiserror.workspace = true
log.workspace = true
env_logger.workspace = true
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
once_cell.workspace = true
parking_lot.workspace = true
rand.workspace = true
sdl2.workspace = true
ureq.workspace = true
keyring.workspace = true
dirs.workspace = true
discord-rich-presence.workspace = true
webbrowser.workspace = true
sha2.workspace = true
hex.workspace = true
tempfile.workspace = true

[target.'cfg(target_os = "macos")'.dependencies]
sdl2 = { workspace = true, features = ["bundled", "static-link"] }

[target.'cfg(target_os = "linux")'.dependencies]
sdl2 = { workspace = true, features = ["bundled", "static-link"] }

[dev-dependencies]
tempfile.workspace = true
```

Workspace dependency versions (root `Cargo.toml`, `[workspace.dependencies]`)
for what the app layer uses:

```toml
log = "0.4"
env_logger = "0.11"
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["float_roundtrip"] }
chrono = { version = "0.4", default-features = false, features = ["std", "clock", "serde"] }
parking_lot = "0.12"
tempfile = "3"
sdl2 = "0.38"
dirs = "6"
```

Workspace package: edition 2021, rust-version 1.95, version 1.0.0.

Deps actually needed by shell/input/paths/stack/menu/settings: `sdl2`
(shell), `log` + `env_logger` (logging/Tee), `serde` + `serde_json`
(settings), `dirs` (paths), `chrono` (`paths::today`), `tempfile` (as a
normal dep, because `Paths::temp_for_tests` / `App::headless_for_tests` are
public non-test API), `parking_lot` (the `TestTranscript` mutex).

---

## 9. Environment variables read by `crates/lucky-break`

| Var | Read at | Effect | Tag |
|---|---|---|---|
| `LUCKY_BREAK_DATA_DIR` | `src/app/paths.rs:12` (`var_os`) | data dir override; default `dirs::data_local_dir()/LuckyBreakCasino`, else `./LuckyBreakCasino` | G (spec: `<GAME>_DATA_DIR`) |
| `LUCKY_BREAK_LOG_FILE` | `src/app/mod.rs:657` (`var_os(LOG_FILE_ENV)`, const at 699) | log file override; default `<data_dir>/luckybreak.log` | G |
| `LUCKY_BREAK_ASSETS` | `src/audio/assets.rs:23` (`var_os`) | asset root override; then `<exe dir>/assets`, then `./assets` (first with `manifest.json`) | audio |
| `LUCKY_BREAK_AUDIO_BACKEND` | `src/audio/mod.rs:86` (`== "null"`); test save/restore at 179, 196 | forces null audio | audio |
| `LUCKY_BREAK_NO_SPEECH` | `src/speech/mod.rs:106` (`== "1"`) | forces `ConsoleSpeech` | speech |
| `LUCKY_BREAK_ONLINE_URL` | `src/online/net.rs:40` (const `BASE_URL_ENV` at 36) | online base URL | O |
| `LUCKY_BREAK_NO_KEYRING` | `src/online/presence.rs:66` (const `NO_KEYRING_ENV` at 31); test helper `src/online/mod.rs:55` | credentials in a file instead of keyring | O |
| `LUCKY_BREAK_OFFLINE` | `src/updater.rs:21` (const `OFFLINE_ENV` at 17) | skip update check | O |
| `RUST_LOG` | implicitly via `env_logger::Env::default()` at `src/app/mod.rs:677, 681` | log filter (default `info`) | G |

Tests that set env vars: `src/online/mod.rs:56-57` (`TempStore`, process-wide
lock), `src/audio/mod.rs:180, 188-189, 197, 208`, `src/speech/mod.rs:149, 151`.
Compile-time: `env!("CARGO_PKG_VERSION")` (app/mod.rs:34),
`env!("CARGO_MANIFEST_DIR")` in `tests/*.rs`; `build.rs` reads
`CARGO_MANIFEST_DIR`, `CARGO_CFG_TARGET_OS`, `CARGO_CFG_TARGET_ARCH`, `OUT_DIR`.
The player-facing list is `docs/help.md` "## Environment variables" (line 121,
repo root).

---

## 10. Generic vs casino-specific

| Piece | Location | Verdict for hearken-app |
|---|---|---|
| `Shell` trait | shell.rs:17-23 | Take as is |
| `SdlShell` | shell.rs:25-82 | Take; store window title (line 66 uses `super::WINDOW_TITLE`), size 640x360 hard-coded |
| `translate_events`, `mods_from`, `key_from_keycode` | shell.rs:84-189 | Take (no controller, no key repeat) |
| `HeadlessShell` | shell.rs:193-228 | Take |
| `Key`, `Mods`, `InputEvent` | input.rs | Take |
| `Paths` | paths.rs:3-50 | Take; env name, dir name, file names become game-supplied; `profile_file`/`credentials_file` are C/O |
| `paths::today` | paths.rs:52-54 | Helper; used by C (daily comp) |
| `State` trait | states/mod.rs:41-64 | Take with `on_event(&G::Event)` |
| `StackOp` | states/mod.rs:134-139 | Take (generic over G) |
| `EscapeGuard` | states/mod.rs:66-130 | C (table Leave refusal); optional |
| `track_profile` | states/mod.rs:146-201 | C |
| `BootState` | states/mod.rs:205-243 | G test stub (text is C) |
| `CliOptions` | app/mod.rs:44-83 | G (`--smoke`/`--playtest` belong to hearken-playtest) |
| `OnlineStatus`, `pump_presence`, `pump_update_check`, `set_online_presence`, `set_discord_presence` | app/mod.rs:85-132, 229-268, 433-474 | O (hearken-online) |
| `GameContext` speech/audio/settings/paths/running/transcript/notices/pending/frame/clock | app/mod.rs:136-162 | G (`Ctx<G>`) |
| `GameContext` profile/session | app/mod.rs:141-142 | C (`G::Data`) |
| `GameContext` online/version_available/update_rx/discord | app/mod.rs:148-156 | O |
| `say`/`say_queued`/`record`/`stop_speech`/`ui`/stack ops/`quit`/`apply_volumes` | app/mod.rs:167-219, 271-277 | G |
| `save_profile`, `today` | app/mod.rs:221-227, 280-282 | C |
| `App` new/step/run/top_*/depth/apply_pending | app/mod.rs:285-572 | G, minus C session pump (392-408), listener/occlusion (414-421), profile save (485), O calls |
| `headless_for_tests`/`headless_with_http` | app/mod.rs:306-353 | T; C voice clip set (325-330), `Title` root (351), O `FakeHttp` |
| `build_context` | app/mod.rs:579-650 | Mixed; G core is paths, logging, settings, speech, audio, volumes |
| `init_logging` + `Tee` + `LOG_FILE_ENV` | app/mod.rs:652-722 | G (env name + transcript target name are C) |
| `main_with` | app/mod.rs:732-756 | G shape; C first screen `Title` |
| `Menu`/`MenuCore`/`MenuItem`/`Label`/`menu_input`/`impl_state_for_menu!` | states/menu.rs:1-452 | G except `lb_core::protocol::Event` (190, 435); spec takes FF's menu instead |
| `SettingsState` | states/settings.rs | G for bus/rate/backend rows; O for presence/Discord rows (130-165) |
| `HelpState` | states/help.rs | G mechanism; doc path (line 10) game-supplied |
| `Pause` | states/pause.rs | G for Resume/Settings/Help; C for save-and-title/Quit bodies (51-64) |
| `Settings` | settings.rs | G for volumes/speech_rate/audio_backend; O for online_presence/discord; `level()` tied to `audio::Bus` |
| `Title`, `Creation`, `Floor`, tables | states/*.rs | C |
| `playtest.rs` | src/playtest.rs | hearken-playtest (not hearken-app); `debug bankroll` is C |

Status: COMPLETE.
