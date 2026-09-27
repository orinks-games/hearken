# Hearken M1b Input Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Everything a game needs from input beyond the M1 menus: held-key polling that survives JAWS, a controller manager with hot-plug and D-pad repeat, rebindable game actions with a rebinding screen, a text-entry screen, a timed-message screen, and the two M1 follow-ups (session log rotation and level, one Prism link recipe).

**Architecture:** All of it lands in `hearken-app`, extracted from Freight Fate (FF, `c5d3ef8d`) with its trucking cut out. Controller input stays an `InputEvent` like keys: the app hands controller events to a `ControllerManager` that admits only the bound pad's presses, and `InputEvent::pressed()` lets any screen that reads keys work from a pad (A is Enter, B and Start are Escape, Back is F1, the D-pad is the arrows), so FF's separate `handle_controller` hook and `ControllerAction` enum are not needed. Rebindable controls are a game-supplied `Action` type (`Game::Action`); the live `KeyBindings<A>` sits on `Ctx`, saved as id-to-chord maps in `settings.json`.

**Tech Stack:** Rust 1.98.0 (edition 2021), `sdl2` 0.38 (GameController API), `windows-sys` 0.59 (`SystemParametersInfoW` for the keyboard repeat timing), `serde`, `log`/`env_logger`.

**Spec:** `docs/superpowers/specs/2026-09-26-hearken-engine-design.md` (hearken-app "Input", "Menus", "Built-in screens"; Milestones, M1b). Research notes with verbatim FF source: `docs/superpowers/notes/m1-ff-menus-input.md` sections 1 (TimedMessageState), 3 (text entry), 4 (bindings, names, shortcuts screen), 5 (controller), 6 (held keys). Open follow-ups: `docs/superpowers/notes/m1-followups.md`.

## Global Constraints

- Toolchain pinned to `1.98.0`; `edition = "2021"`, `rust-version = "1.95"`. Do not move to edition 2024.
- No new crates. New modules only inside `hearken-app`.
- Source repos are read-only. Extract with `git -C C:/Users/joshu/Freight-fate show c5d3ef8d:<path>` so the pinned commit is what gets copied. Each extracted module's doc names the commit.
- No `FREIGHT_FATE_*`, trucking, driving, pedal or casino names anywhere in Hearken code or docs. Per-game environment variables are `<Game::ENV_PREFIX>_*`.
- Nothing may block the game loop. No network, IPC or thread joins in `Drop`.
- Spoken text is for players: no jargon, and silence over redundant speech.
- Code files stay at or under 1000 lines.
- Gates, each separately: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; the tests. `actionlint` after any workflow change (none planned).
- Exactly one `cargo test` run in flight on the machine at a time. Background any single command expected to take over 30 seconds. Never pipe a test run to `tail`; redirect to a file and read the result.
- Focused tests while iterating (`cargo test -p hearken-app --test <file>`, `cargo test -p hearken-app --lib <filter>`, `cargo test -p hello-hearken`); the full `cargo test --workspace --locked` once, in Task 6.
- Work on branch `m1b-input` in `C:/Users/joshu/gh-projects/hearken`, cut from `main`. One commit per task. Every commit message ends with:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`

## Deferred out of M1b (do not build)

- **M2 Sound:** every earcon. FF's menus, text entry (`ui/tick`, `ui/error`), the shortcuts screen (`ui/menu_select`, `ui/error`) and the timed message (`ui/notify`) all play sounds; Hearken's stay silent until `hearken-audio` exists. Text-entry edges (Backspace at the start, an arrow at an end) are silent here for that reason.
- **Not carried:** controller rumble, FF's controller mapping diagnostics (the missing-trigger warning, `--controller-diagnostics`), FF's analog pedal/wheel/clutch state, FF's message-review keys, `SimpleMenuState`, a save-notice screen.
- **M4 Playtest:** `<PREFIX>_NO_CONTROLLER` (keeps pads out of agent sessions) arrives with the agent server.

## Review Focus

1. **Alt-tab away while holding a D-pad direction.** SDL drops the release while the window is unfocused. Expected: no runaway repeats while away, and the first press after coming back works instead of being taken for a duplicate. Pinned in Task 2.
2. **Unplugging the controller mid-game.** Expected: the player hears one line saying so and that the keyboard still works, the top screen sees the removal (so gameplay can pause), and a second unplug event for some other pad says nothing. Pinned in Task 2.
3. **A hand-edited settings file naming a key the game does not know, or an action the game no longer has.** Expected: that one binding falls back to its default, every other binding survives, and nothing is said or crashes. Pinned in Task 3.
4. **A keyboard key pressed while the Controller buttons screen is listening** (or a pad button on the Keyboard shortcuts screen). Expected: ignored, except Escape, which keeps the old binding. Pinned in Task 4.
5. **Typing into a full text field.** Expected: the player hears that the field is full instead of silence that sounds like a dead key. Pinned in Task 5.

## File Structure

| File | Task | Responsibility |
|---|---|---|
| `crates/hearken-app/src/input.rs` | 1 | `ControllerButtonDown.repeat`, `pressed()`, `is_repeat()`, `controller_key_map` |
| `crates/hearken-app/src/held_keys.rs` | 1 | FF's `HeldKeys` (JAWS pulse layer) verbatim, with its tests |
| `crates/hearken-app/src/app.rs` | 1, 2, 3 | `admit`/`dispatch` split; held keys, controller routing, repeats |
| `crates/hearken-app/src/ctx.rs` | 1, 2, 3 | `held`, `controller`, `bindings`; `action_for`, `action_held`, `control_name`, `save_settings`, `save_bindings` |
| `crates/hearken-app/src/controller.rs` | 2 | `ControllerManager`, `RepeatTimer`, deadzone, the device seam |
| `crates/hearken-app/src/controller/sdl.rs` | 2 | `PadDevice`/`PadSubsystem` over `sdl2::controller`, `sdl_factory` |
| `crates/hearken-app/src/controller/fakes.rs` | 2 | `FakePad`, `FakeSubsystem`, `fake_factory` |
| `crates/hearken-app/src/bindings.rs` | 3 | `Action` trait, `NoAction`, `Chord`, `PadChord`, `Rebind`, `KeyBindings<A>`, `SavedBindings` |
| `crates/hearken-app/src/bindings/names.rs` | 3 | FF's key and button names verbatim |
| `crates/hearken-app/src/screens/bindings.rs` | 4 | `BindingsScreen`: the rebinding screen |
| `crates/hearken-app/src/screens/settings.rs` | 3, 4 | Keyboard shortcuts, Controller buttons and Controller rows |
| `crates/hearken-app/src/screens/text_entry.rs` | 5 | `TextEntryScreen` |
| `crates/hearken-app/src/screens/timed_message.rs` | 5 | `TimedMessage` |
| `crates/hearken-app/src/logging.rs`, `run.rs` | 2, 6 | controller factory; log rotation, `<PREFIX>_LOG` |
| `crates/hearken-speech/build.rs`, `crates/hearken-app/build.rs`, `template/build.rs` | 6 | one Prism link recipe |
| `template/src/*`, `template/help.md`, `template/tests/walkthrough.rs` | 1, 3, 4 | a rebindable Count action, rebinding reachable from Settings |

---

### Task 1: Held keys, and controller buttons standing in for keys

**Files:**
- Modify: `Cargo.toml` (workspace `windows-sys` features)
- Modify: `crates/hearken-app/src/input.rs`, `src/sdl_shell.rs`, `src/menu.rs`, `src/app.rs`, `src/ctx.rs`, `src/lib.rs`
- Create: `crates/hearken-app/src/held_keys.rs`
- Modify: `template/src/practice.rs`
- Test: `crates/hearken-app/tests/foundations.rs`, create `crates/hearken-app/tests/input.rs`

**Interfaces:**
- Consumes: M1's `InputEvent`, `App`, `Ctx`, `menu_input`.
- Produces:
  - `InputEvent::ControllerButtonDown { button: ControllerButton, instance_id: u32, repeat: bool }` (new `repeat` field; `InputEvent::button(b)` sets it `false`).
  - `InputEvent::pressed(&self) -> Option<Key>`; `InputEvent::is_repeat(&self) -> bool` (replaces `key_repeat`, now true for key and pad repeats).
  - `hearken_app::controller_key_map(ControllerButton) -> Option<Key>`.
  - `hearken_app::held_keys::HeldKeys` (re-exported as `hearken_app::HeldKeys`): `default()`, `with_timing(delay_ms, interval_ms)`, `is_pressed(Key)`, `pulsed(Key)`, `mods()`, `begin_frame(dt)`, `press(Key, Mods)`, `release(Key, Mods)`, `clear()`, `clear_pulses()`, `refresh_repeat_timing()`, `learned_spacing_ms()`, `fresh_pulse_ms()`, `repeat_pulse_ms()`.
  - `Ctx::held: HeldKeys`.
  - `App` private `admit(&mut self, &InputEvent) -> bool` and `dispatch(&mut self, &InputEvent)`; Task 2 extends both.

- [ ] **Step 1: Branch**

```bash
cd C:/Users/joshu/gh-projects/hearken
git checkout -b m1b-input
```

- [ ] **Step 2: Write the failing tests**

In `crates/hearken-app/tests/foundations.rs`, replace the last line of `typed_characters_land_on_their_keys`:

```rust
    assert!(!InputEvent::key(Key::Down).key_repeat());
```

with:

```rust
    assert!(!InputEvent::key(Key::Down).is_repeat());
```

and add after that test:

```rust
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
```

Create `crates/hearken-app/tests/input.rs`:

```rust
mod common;

use common::{Probe, TestData, TestGame};
use hearken_app::{Headless, HeldKeys, InputEvent, Key};

fn start() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    // A known repeat timing, whatever this machine's keyboard settings say.
    h.app.ctx.held = HeldKeys::with_timing(500, 33);
    h
}

#[test]
fn a_held_key_reads_held_until_it_is_released() {
    let mut h = start();
    h.press(Key::Up);
    assert!(h.app.ctx.held.is_pressed(Key::Up));
    h.frame(0.1);
    assert!(h.app.ctx.held.is_pressed(Key::Up));
    h.send(InputEvent::key_up(Key::Up));
    assert!(!h.app.ctx.held.is_pressed(Key::Up));
}

#[test]
fn leaving_the_window_lets_go_of_every_key() {
    let mut h = start();
    h.press(Key::Up);
    h.send(InputEvent::WindowFocusLost);
    assert!(!h.app.ctx.held.is_pressed(Key::Up));
}

#[test]
fn a_screen_readers_re_sent_key_reads_held_but_a_new_screen_never_inherits_it() {
    let mut h = start();
    // JAWS re-sends an arrow as a press and a release inside one frame.
    h.app.step(
        vec![InputEvent::key(Key::Up), InputEvent::key_up(Key::Up)],
        1.0 / 60.0,
    );
    assert!(h.app.ctx.held.is_pressed(Key::Up), "a pair in one frame is a hold");
    h.app.ctx.push(Probe::new("next"));
    h.frame(1.0 / 60.0);
    assert!(!h.app.ctx.held.is_pressed(Key::Up));
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p hearken-app --test foundations --test input > target/t1.txt 2>&1`
Expected: compile errors: no method `is_repeat`, no method `pressed`, no field `held`, no `HeldKeys` in `hearken_app`.

- [ ] **Step 4: Input vocabulary**

In `crates/hearken-app/src/input.rs`:

Replace the `ControllerButtonDown` variant with:

```rust
    ControllerButtonDown {
        button: ControllerButton,
        instance_id: u32,
        /// True for the controller manager's own repeats of a held D-pad
        /// left or right.
        repeat: bool,
    },
```

Replace `InputEvent::button` with:

```rust
    /// A controller button press from instance 0.
    pub fn button(button: ControllerButton) -> InputEvent {
        InputEvent::ControllerButtonDown {
            button,
            instance_id: 0,
            repeat: false,
        }
    }
```

Replace `key_repeat` (the last method of `impl InputEvent`) with the two methods below, and add `controller_key_map` as a free function after the `impl InputEvent` block. Shown together, from `is_repeat` to the end of `controller_key_map`:

```rust
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
```

In `crates/hearken-app/src/sdl_shell.rs`, the `Event::ControllerButtonDown` arm of `translate_events` becomes:

```rust
            Event::ControllerButtonDown { which, button, .. } => {
                out.push(InputEvent::ControllerButtonDown {
                    button: button_from_sdl(button),
                    instance_id: which,
                    repeat: false,
                });
            }
```

In `crates/hearken-app/src/menu.rs`, replace the head of `menu_input` (from `let InputEvent::KeyDown { key, text, .. } = event else {` through `match key {`) so the function reads:

```rust
/// The navigation every menu shares, from the keyboard or a controller. A
/// screen with extra keys overrides `handle_input` and calls this for
/// everything it does not claim.
pub fn menu_input<G: Game, S: Menu<G>>(state: &mut S, ctx: &mut Ctx<G>, event: &InputEvent) {
    let Some(key) = event.pressed() else {
        return;
    };
    let text = event.key_down().and_then(|(_, _, text)| text);
    match key {
        Key::Down => state.move_by(ctx, 1),
        Key::Up => state.move_by(ctx, -1),
        Key::Home => state.jump(ctx, 0),
        Key::End => {
            let last = state.menu().items.len().saturating_sub(1);
            state.jump(ctx, last);
        }
        Key::Return | Key::KpEnter | Key::Space => state.activate(ctx),
        Key::Left => state.adjust(ctx, -1),
        Key::Right => state.adjust(ctx, 1),
        Key::Escape => state.on_escape(ctx),
        other => {
            let typed = text
                .filter(|c| c.is_alphanumeric())
                .or_else(|| letter_of(other));
            if let Some(c) = typed {
                state.letter_jump(ctx, c);
            }
        }
    }
}
```

In `template/src/practice.rs`, `if event.key_repeat() {` becomes `if event.is_repeat() {`.

- [ ] **Step 5: Held keys, extracted verbatim**

Add `"Win32_UI_WindowsAndMessaging"` to the workspace `windows-sys` features in the root `Cargo.toml`:

```toml
windows-sys = { version = "0.59", features = ["Win32_Foundation", "Win32_Security", "Win32_System_Threading", "Win32_System_Diagnostics_ToolHelp", "Win32_UI_WindowsAndMessaging"] }
```

Extract the file:

```bash
git -C C:/Users/joshu/Freight-fate show c5d3ef8d:crates/freight-fate/src/app/held_keys.rs > crates/hearken-app/src/held_keys.rs
```

Then edit it:

1. Replace the module doc (lines 1-53, everything above `use std::collections::{HashMap, HashSet};`) with:

```rust
//! Held keys as a game that polls should see them, screen reader or not.
//!
//! A game that asks "is Up down right now?" (a throttle, a paddle stroke)
//! reads [`HeldKeys::is_pressed`], which answers from the key events the
//! app dispatched. With no screen reader, or with NVDA, that is the
//! physical keyboard: a held key reads held until the finger lifts.
//!
//! JAWS is different. It binds the arrow keys to its own scripts in every
//! application, so its keyboard hook swallows the physical press, runs the
//! script, and re-sends the key to the application as a synthetic
//! press-and-release pair. The game sees a tap that lasts zero frames, which
//! a poll never catches: menus react to the press event and work, polling
//! code does nothing. Holding the key does not help by itself, but the
//! keyboard's auto-repeat still runs underneath; every repeat goes through
//! the same hook and arrives as another pair. Measured with JAWS
//! (2026-08-24): the first repeat lands at the Windows repeat delay
//! (512 ms), the rest about 250 ms apart -- not the 33 ms Windows rate,
//! because JAWS's script takes that long per key and the repeats queue
//! behind it.
//!
//! The pulse layer here turns that train back into a hold. Every press
//! starts a pulse: a fresh press gets one long enough to reach the first
//! auto-repeat (the operating system's delay plus grace); a repeat gets one
//! just past the spacing the repeats are actually arriving at, which the
//! tracker learns from the pairs themselves (second repeat onward,
//! synthetic pairs only) and keeps for the rest of the session. Until it
//! has learned that spacing, repeats get the fresh pulse too, so the very
//! first hold never stutters. A release in the same frame as the press that
//! began the pair is synthetic -- nobody taps a key inside one frame -- and
//! leaves the pulse alone; a release in a later frame is the player's
//! finger and ends it. A key reads pressed when it is physically down OR a
//! pulse is alive, so the physical path is exactly what it always was and
//! the re-injected path reads as a hold that lapses one learned spacing
//! plus grace after the last pair.
//!
//! Two guards keep the physical path honest. SDL delivers the keyboard's
//! own auto-repeat as more `KeyDown`s of a key that is already down, so
//! only a press from the released state can begin a synthetic pair: a
//! finger lifting in the same frame as one of those repeats is a real
//! release. And the layer is inert until [`HeldKeys::begin_frame`] has
//! clocked a frame, so code that feeds keys straight in keeps plain
//! semantics.
//!
//! Two honest limits. A screen reader that re-injects keys never shows the
//! game how long a tap lasted, so under JAWS a tap reads as a hold for the
//! repeat delay (half a second at the Windows default), letting go reads
//! about a third of a second late, and a gesture built on tap length cannot
//! be seen. And the game cannot tell whether such a screen reader is
//! running, so the pulse logic is always on; on the physical path it only
//! ever adds a hold when a whole press-and-release arrives inside one short
//! frame, which a finger cannot do.
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `app/held_keys.rs`.
```

2. Replace `use crate::states::base::{Key, Mods};` with `use crate::input::{Key, Mods};`.

Nothing else changes; the file's own `#[cfg(test)] mod tests` (12 tests) comes with it. Check no word "driving", "pedal" or "Freight Fate" survives outside the extraction line:

```bash
grep -n -i "driv\|pedal\|freight\|brake" crates/hearken-app/src/held_keys.rs
```

Expected: only the `Extracted from Freight Fate` line, and `a_pair_that_lands_after_a_hitch_is_not_a_hold`'s comment "not a half-second of brake", which you reword to "not a half-second hold".

- [ ] **Step 6: Wire held keys into the context and the loop**

`crates/hearken-app/src/lib.rs`: add `pub mod held_keys;` (alphabetical, after `pub mod game;`), `pub use held_keys::HeldKeys;`, and change the input re-export to `pub use input::{controller_key_map, ControllerAxis, ControllerButton, InputEvent, Key, Mods};`.

`crates/hearken-app/src/ctx.rs`: add `use crate::held_keys::HeldKeys;`, the field (after `pub game: G::Data,`):

```rust
    /// Which keys are down now, for a screen that polls instead of reacting
    /// to presses.
    pub held: HeldKeys,
```

and `held: HeldKeys::default(),` in `Ctx::new`.

`crates/hearken-app/src/app.rs`: replace the event loop at the top of `step` (from `for event in events {` down to, not including, the `// A screen that pushed or popped from a key` comment) with:

```rust
        // The clock the held-key pulses run on; see `held_keys`.
        self.ctx.held.begin_frame(dt);
        for event in events {
            if self.admit(&event) {
                self.dispatch(&event);
            }
        }
```

Add these two methods to `impl<G: Game> App<G>` (after `step`):

```rust
    /// The bookkeeping every event gets before any screen sees it. False
    /// when no screen should see it at all.
    fn admit(&mut self, event: &InputEvent) -> bool {
        match event {
            InputEvent::Quit => {
                self.ctx.running = false;
                return false;
            }
            // Coming back to the window re-checks which screen reader is
            // running and the keyboard's repeat timing, either of which the
            // player may have changed while away, and nothing held before
            // they left is held now. The screen still sees the event.
            InputEvent::WindowFocusGained => {
                self.ctx.speech.request_refresh();
                self.ctx.held.refresh_repeat_timing();
                self.ctx.held.clear_pulses();
            }
            InputEvent::WindowFocusLost => self.ctx.held.clear(),
            InputEvent::KeyDown { key, mods, .. } => self.ctx.held.press(*key, *mods),
            InputEvent::KeyUp { key, mods } => self.ctx.held.release(*key, *mods),
            _ => {}
        }
        true
    }

    /// The keys that work on every screen, else the top screen.
    fn dispatch(&mut self, event: &InputEvent) {
        match event.pressed() {
            // F2 silences speech everywhere, so no screen can hold it.
            Some(Key::F2) => {
                self.ctx.stop_speech();
                return;
            }
            // F1, or a controller's Back button, reads the top screen's
            // help.
            Some(Key::F1) => {
                let help = self.states.last().map(|top| top.help(&self.ctx));
                if let Some(help) = help {
                    self.ctx.say(help);
                }
                return;
            }
            _ => {}
        }
        if let Some(top) = self.states.last_mut() {
            top.handle_input(&mut self.ctx, event);
        }
    }
```

At the top of `apply_pending`, before the `for _ in 0..MAX_STACK_ROUNDS` loop:

```rust
        // A new screen never inherits the last one's held keys.
        if self.ctx.has_pending() {
            self.ctx.held.clear_pulses();
        }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p hearken-app --test foundations --test input --test app --test menu > target/t1.txt 2>&1`, then `cargo test -p hearken-app --lib held_keys > target/t1-lib.txt 2>&1`, then `cargo test -p hello-hearken > target/t1-template.txt 2>&1`.
Expected: all pass (the 12 held-key tests among them; M1's F1, F2 and focus tests in `app.rs` unchanged).

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add Cargo.toml crates/hearken-app template
git commit -m "feat(app): held keys, and controller buttons standing in for keys

HeldKeys comes from Freight Fate c5d3ef8d verbatim: a poll sees a JAWS
re-injected key train as the hold it stands for. InputEvent::pressed lets
any screen that reads keys work from a controller.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The controller manager

**Files:**
- Create: `crates/hearken-app/src/controller.rs`, `src/controller/sdl.rs`, `src/controller/fakes.rs`
- Modify: `crates/hearken-app/src/settings.rs`, `src/ctx.rs`, `src/app.rs`, `src/sdl_shell.rs`, `src/run.rs`, `src/testing.rs`, `src/lib.rs`
- Test: create `crates/hearken-app/tests/controller.rs`

**Interfaces:**
- Consumes: Task 1's `InputEvent` (with `repeat`), `pressed()`, `App::admit`/`dispatch`.
- Produces:
  - `hearken_app::controller::{ControllerManager, InputDevice, RepeatTimer, PadDevice, PadSubsystem, PadSubsystemFactory, deadzone, CONTROLLER_DISCONNECTED, STICK_DEADZONE, TRIGGER_DEADZONE, AXIS_MAX, REPEAT_DELAY_S, REPEAT_FAST_S, REPEAT_RAMP_S}`; `ControllerManager` and `InputDevice` re-exported at the crate root.
  - `ControllerManager::new(enabled: bool, factory: Option<PadSubsystemFactory>) -> Self`, `detached(enabled: bool) -> Self`, `subsystem_up()`, `connected()`, `active()`, `enabled()`, `name() -> &str`, `instance_id() -> Option<u32>`, `id_pending()`, `set_id_pending(bool)`, `set_enabled(bool)`, `bind_device(Box<dyn PadDevice>, &str)`, `reopen()`, `take_disconnect() -> bool`, `release_all()`, `note_keyboard()`, `device() -> InputDevice`, `modifier() -> bool`, `is_down(ControllerButton) -> bool`, `axis(ControllerAxis) -> f64`, `process_event(&InputEvent) -> bool`, `tick(dt) -> Vec<InputEvent>`, `shutdown()`.
  - `hearken_app::controller::fakes::{FakePad, FakeSdlLog, FakeSubsystem, fake_factory}`; `FakePad::new(instance_id: u32)`, `FakePad.attached: Rc<Cell<bool>>`.
  - `hearken_app::controller::sdl::sdl_factory(sdl2::Sdl) -> PadSubsystemFactory`; `SdlShell::controller_factory(&self) -> PadSubsystemFactory`.
  - `hearken_app::settings::ControllerSettings { enabled: bool }` (default `true`), `Settings.controller`.
  - `Ctx::controller: ControllerManager`; `Headless::connect_pad(&mut self)`.

- [ ] **Step 1: Write the failing tests**

Create `crates/hearken-app/tests/controller.rs`:

```rust
mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{take_log, Probe, TestData, TestGame};
use hearken_app::controller::fakes::{fake_factory, FakePad, FakeSdlLog};
use hearken_app::controller::{
    deadzone, ControllerManager, InputDevice, RepeatTimer, CONTROLLER_DISCONNECTED,
    REPEAT_DELAY_S,
};
use hearken_app::menu::{Label, Menu, MenuCore, MenuItem};
use hearken_app::{ControllerAxis, ControllerButton as B, Ctx, Headless, InputEvent};

fn down(button: B, instance_id: u32) -> InputEvent {
    InputEvent::ControllerButtonDown {
        button,
        instance_id,
        repeat: false,
    }
}

fn up(button: B, instance_id: u32) -> InputEvent {
    InputEvent::ControllerButtonUp {
        button,
        instance_id,
    }
}

fn axis(axis: ControllerAxis, value: i16) -> InputEvent {
    InputEvent::ControllerAxis {
        axis,
        value,
        instance_id: 0,
    }
}

/// A manager with a pad bound as `instance_id`, its id latched.
fn with_pad(instance_id: u32) -> ControllerManager {
    let mut m = ControllerManager::detached(true);
    m.bind_device(Box::new(FakePad::new(instance_id)), "test pad");
    m.set_id_pending(false);
    m
}

fn with_factory(log: &Rc<RefCell<FakeSdlLog>>, enabled: bool) -> ControllerManager {
    ControllerManager::new(
        enabled,
        Some(fake_factory(vec![("Pad".to_string(), 4)], log.clone())),
    )
}

#[test]
fn the_deadzone_is_silent_inside_and_reaches_full_range_at_the_edge() {
    assert_eq!(deadzone(0.05, 0.10), 0.0);
    assert_eq!(deadzone(1.0, 0.10), 1.0);
    assert_eq!(deadzone(-1.0, 0.10), -1.0);
    assert!((deadzone(0.55, 0.10) - 0.5).abs() < 1e-9);
}

#[test]
fn a_held_dpad_side_repeats_after_a_pause_then_faster() {
    let mut t = RepeatTimer::default();
    t.press(B::A);
    assert_eq!(t.tick(1.0), 0, "only the D-pad sides repeat");
    t.press(B::DPadRight);
    assert_eq!(t.tick(REPEAT_DELAY_S - 0.01), 0);
    assert_eq!(t.tick(0.02), 1);
    // Held long enough, the repeats come every REPEAT_FAST_S.
    t.tick(2.0);
    let fast = t.tick(0.3);
    assert!((9..=11).contains(&fast), "{fast}");
    t.release(B::DPadRight);
    assert_eq!(t.tick(1.0), 0);
}

#[test]
fn only_the_bound_pads_presses_get_through_and_a_doubled_press_counts_once() {
    let mut m = with_pad(3);
    assert_eq!(m.device(), InputDevice::Keyboard);
    assert!(m.process_event(&down(B::A, 3)));
    assert!(!m.process_event(&down(B::A, 3)), "a pad that sends a press twice");
    assert!(m.process_event(&up(B::A, 3)));
    assert!(!m.process_event(&up(B::A, 3)), "a release with no press");
    assert!(!m.process_event(&down(B::A, 9)), "another pad");
    assert_eq!(m.device(), InputDevice::Controller);
    m.note_keyboard();
    assert_eq!(m.device(), InputDevice::Keyboard);
}

#[test]
fn a_freshly_opened_pad_takes_the_id_of_its_first_event() {
    let mut m = ControllerManager::detached(true);
    m.bind_device(Box::new(FakePad::new(1)), "test pad");
    assert!(m.id_pending());
    assert!(m.process_event(&down(B::A, 7)), "a hot-plug left the opened id stale");
    assert_eq!(m.instance_id(), Some(7));
    assert!(!m.process_event(&down(B::B, 1)), "the stale id is foreign now");
}

#[test]
fn unplugging_the_bound_pad_is_reported_once_and_lets_go_of_everything() {
    let mut m = with_pad(2);
    m.process_event(&down(B::RightShoulder, 2));
    assert!(m.modifier() && m.is_down(B::RightShoulder));
    assert!(m.process_event(&InputEvent::ControllerRemoved { instance_id: 2 }));
    assert!(m.take_disconnect());
    assert!(!m.take_disconnect());
    assert!(!m.connected());
    assert!(!m.modifier());
    assert!(
        !m.process_event(&InputEvent::ControllerRemoved { instance_id: 5 }),
        "some other pad"
    );
    assert!(!m.take_disconnect());
}

#[test]
fn a_new_pad_is_opened_only_when_none_is_bound() {
    let log = Rc::new(RefCell::new(FakeSdlLog::default()));
    let mut m = with_factory(&log, true);
    assert_eq!(log.borrow().opens, [0], "the pad plugged in at start");
    assert_eq!(m.name(), "Pad");
    m.process_event(&InputEvent::ControllerAdded { device_index: 0 });
    assert_eq!(log.borrow().opens, [0], "already bound and attached");
    m.process_event(&InputEvent::ControllerRemoved { instance_id: 4 });
    m.process_event(&InputEvent::ControllerAdded { device_index: 0 });
    assert_eq!(log.borrow().opens, [0, 0]);
    assert!(m.connected());
}

#[test]
fn a_pad_that_went_away_without_a_word_is_replaced_on_the_next_add() {
    let log = Rc::new(RefCell::new(FakeSdlLog::default()));
    let mut m = with_factory(&log, true);
    let pad = FakePad::new(4);
    let attached = pad.attached.clone();
    m.bind_device(Box::new(pad), "Pad");
    attached.set(false);
    m.process_event(&InputEvent::ControllerAdded { device_index: 0 });
    assert_eq!(log.borrow().opens, [0, 0]);
}

#[test]
fn controllers_off_never_touch_the_subsystem_and_on_brings_it_back() {
    let log = Rc::new(RefCell::new(FakeSdlLog::default()));
    let mut m = with_factory(&log, false);
    assert_eq!(log.borrow().inits, 0);
    assert!(!m.subsystem_up());
    m.set_enabled(true);
    assert_eq!(log.borrow().inits, 1);
    assert!(m.connected());
    m.set_id_pending(false);
    m.set_enabled(false);
    assert!(!m.subsystem_up() && !m.connected());
    assert_eq!(log.borrow().quits, 1);
    assert!(!m.process_event(&down(B::A, 4)));
    m.set_enabled(true);
    assert_eq!(log.borrow().inits, 2);
}

#[test]
fn sticks_and_triggers_read_past_their_deadzones_and_only_while_active() {
    let mut m = with_pad(0);
    m.process_event(&axis(ControllerAxis::LeftX, 1000));
    assert_eq!(m.axis(ControllerAxis::LeftX), 0.0, "inside the stick deadzone");
    assert_eq!(m.device(), InputDevice::Keyboard, "drift is not use");
    m.process_event(&axis(ControllerAxis::LeftX, -32768));
    assert_eq!(m.axis(ControllerAxis::LeftX), -1.0);
    assert_eq!(m.device(), InputDevice::Controller);
    m.process_event(&axis(ControllerAxis::TriggerRight, 32767));
    assert_eq!(m.axis(ControllerAxis::TriggerRight), 1.0);
    m.set_enabled(false);
    assert_eq!(m.axis(ControllerAxis::LeftX), 0.0);
}

/// A level that Left and Right adjust, and a second row.
struct Rows {
    menu: MenuCore<Self, TestGame>,
    level: i32,
}

impl Rows {
    fn new() -> Self {
        Rows {
            menu: MenuCore::new("Rows"),
            level: 0,
        }
    }
}

impl Menu<TestGame> for Rows {
    fn name(&self) -> &'static str {
        "rows"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        vec![
            MenuItem::new(
                Label::dynamic(|s: &Rows, _ctx: &Ctx<TestGame>| format!("Level {}", s.level)),
                |_s: &mut Rows, ctx: &mut Ctx<TestGame>| ctx.say("chosen"),
            )
            .adjustable(|s: &mut Rows, _ctx: &mut Ctx<TestGame>, delta| s.level += delta),
            MenuItem::inert("Other"),
        ]
    }
}

/// `screen` over a root screen, a pad bound, nothing spoken yet.
fn with_pad_over_root(screen: Rows) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.connect_pad();
    h.app.ctx.push(screen);
    h.frame(0.0);
    h.clear();
    h
}

#[test]
fn a_controller_drives_a_menu_like_the_arrow_keys() {
    let mut h = with_pad_over_root(Rows::new());
    h.send(InputEvent::button(B::DPadDown));
    h.send(InputEvent::button(B::DPadUp));
    h.send(InputEvent::button(B::DPadRight));
    h.send(InputEvent::button(B::A));
    assert_eq!(
        h.spoken(),
        [
            "Other. 2 of 2.",
            "Level 0. 1 of 2.",
            "Level 1. 1 of 2.",
            "chosen"
        ]
    );
    h.send(InputEvent::button(B::B));
    assert_eq!(h.app.top_name(), "root");
}

#[test]
fn the_controllers_back_button_reads_the_help() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.connect_pad();
    h.send(InputEvent::button(B::Back));
    assert_eq!(h.spoken(), ["root help"]);
}

#[test]
fn without_a_bound_pad_no_button_reaches_a_screen() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    take_log(&mut h.app.ctx);
    h.send(InputEvent::button(B::A));
    assert!(take_log(&mut h.app.ctx).is_empty());
}

#[test]
fn unplugging_the_pad_says_so_and_the_screen_hears_it_too() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.connect_pad();
    take_log(&mut h.app.ctx);
    h.send(InputEvent::ControllerRemoved { instance_id: 0 });
    assert_eq!(h.spoken(), [CONTROLLER_DISCONNECTED]);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["root input ControllerRemoved { instance_id: 0 }"]
    );
    h.clear();
    h.send(InputEvent::ControllerRemoved { instance_id: 0 });
    assert!(h.spoken().is_empty(), "said once");
}

#[test]
fn holding_the_dpad_right_keeps_adjusting_until_it_is_let_go() {
    let mut h = with_pad_over_root(Rows::new());
    h.send(InputEvent::button(B::DPadRight));
    h.frame(0.5);
    h.send(InputEvent::button_up(B::DPadRight));
    h.frame(1.0);
    assert_eq!(h.spoken(), ["Level 1. 1 of 2.", "Level 2. 1 of 2."]);
}

#[test]
fn leaving_the_window_stops_a_held_dpad_and_the_next_press_still_works() {
    let mut h = with_pad_over_root(Rows::new());
    h.send(InputEvent::button(B::DPadRight));
    // SDL drops the release while the window is in the background.
    h.send(InputEvent::WindowFocusLost);
    h.frame(2.0);
    assert_eq!(h.spoken(), ["Level 1. 1 of 2."], "no repeats while away");
    h.send(InputEvent::button(B::DPadRight));
    assert_eq!(
        h.spoken().last().map(String::as_str),
        Some("Level 2. 1 of 2."),
        "the press after coming back is not taken for a duplicate"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hearken-app --test controller > target/t2.txt 2>&1`
Expected: compile errors: no module `controller`, no method `connect_pad`.

- [ ] **Step 3: The manager**

Create `crates/hearken-app/src/controller.rs`:

```rust
//! Game controllers: which pad is bound, what its buttons and sticks are
//! doing, held D-pad repeat, and hot-plug.
//!
//! SDL maps any pad it recognises onto the Xbox layout, and the manager
//! binds one pad at a time. It never touches SDL itself: the subsystem and
//! the pad sit behind [`PadSubsystem`] and [`PadDevice`]; [`sdl`] has the
//! real ones and [`fakes`] the test doubles.
//!
//! The app hands every controller event to
//! [`ControllerManager::process_event`], and only the ones it accepts reach
//! a screen, so a second pad, or a pad that delivers a press twice, can
//! never fire an action twice.
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `controller.rs`, without
//! its analog pedal, wheel and clutch state or its rumble.

use std::collections::{HashMap, HashSet};

use crate::input::{ControllerAxis, ControllerButton, InputEvent};

pub mod fakes;
pub mod sdl;

/// What the app says when the bound pad goes away.
pub const CONTROLLER_DISCONNECTED: &str =
    "Controller disconnected. You can keep playing with the keyboard, or reconnect your controller.";

/// Deadzones: 10% for the sticks, 4% for the triggers.
pub const STICK_DEADZONE: f64 = 0.10;
pub const TRIGGER_DEADZONE: f64 = 0.04;
pub const AXIS_MAX: f64 = 32767.0;

/// Held D-pad left or right auto-repeats, for adjusting a setting.
pub const REPEAT_DELAY_S: f64 = 0.30; // wait before the first repeat
pub const REPEAT_FAST_S: f64 = 0.03; // fastest repeat once fully accelerated
pub const REPEAT_RAMP_S: f64 = 1.50; // time held over which it accelerates

/// The device the player used last, so a prompt can name the right control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDevice {
    Keyboard,
    Controller,
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

/// One opened game controller.
pub trait PadDevice {
    /// The instance id the pad opened with -- possibly stale after a
    /// hot-plug.
    fn instance_id(&self) -> u32;
    /// Whether the pad is still physically present. A stub answers `true`
    /// so a spurious add event does not drop a working pad.
    fn attached(&self) -> bool {
        true
    }
}

/// The controller subsystem.
pub trait PadSubsystem {
    fn count(&self) -> u32;
    fn is_controller(&self, index: u32) -> bool;
    fn name_for_index(&self, index: u32) -> Option<String>;
    fn open(&self, index: u32) -> Option<Box<dyn PadDevice>>;
}

/// Brings the subsystem up on demand; `None` when it is unavailable.
pub type PadSubsystemFactory = Box<dyn FnMut() -> Option<Box<dyn PadSubsystem>>>;

/// Owns the bound controller and decides which of its events a screen
/// sees.
pub struct ControllerManager {
    device: Option<Box<dyn PadDevice>>,
    enabled: bool,
    last_used: InputDevice,
    instance_id: Option<u32>,
    name: String,
    /// Latched until the app reads it with `take_disconnect`.
    disconnected: bool,
    /// A freshly opened pad reports a device id that a hot-plug can leave
    /// stale (the events then arrive under a different id). The opened id
    /// is provisional and the id of the first real event is adopted, so the
    /// binding always matches the live event stream.
    id_pending: bool,
    /// Buttons down now, so a duplicate press (some pads and drivers
    /// deliver a button twice) fires only once, on the genuine transition.
    buttons_down: HashSet<ControllerButton>,
    /// The last foreign instance id a dropped event was logged for, so a
    /// stream of them logs once rather than every frame.
    logged_mismatch_id: Option<u32>,
    /// The right bumper is held: the second layer of pad bindings.
    modifier: bool,
    /// Each axis past its deadzone: -1.0 to 1.0, triggers 0.0 to 1.0.
    axes: HashMap<ControllerAxis, f64>,
    repeat: RepeatTimer,
    /// Brought up only once controllers are on, so a player with them off
    /// never has a pad enumerated or bound.
    subsystem: Option<Box<dyn PadSubsystem>>,
    factory: Option<PadSubsystemFactory>,
}

impl ControllerManager {
    /// A manager over a subsystem factory (`None`: no subsystem on this
    /// machine, keyboard only).
    pub fn new(enabled: bool, factory: Option<PadSubsystemFactory>) -> Self {
        let mut manager = ControllerManager {
            device: None,
            enabled,
            last_used: InputDevice::Keyboard,
            instance_id: None,
            name: String::new(),
            disconnected: false,
            id_pending: false,
            buttons_down: HashSet::new(),
            logged_mismatch_id: None,
            modifier: false,
            axes: HashMap::new(),
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
    pub fn detached(enabled: bool) -> Self {
        Self::new(enabled, None)
    }

    // -- device lifecycle -----------------------------------------------------

    /// Bring the subsystem up and bind the first pad. Idempotent and
    /// headless-safe: a failure leaves the subsystem absent (keyboard only).
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

    /// Release the pad, then the subsystem, so no controller handles remain
    /// while support is off.
    fn teardown_subsystem(&mut self) {
        self.close_controller();
        if self.subsystem.take().is_some() {
            log::debug!("Controller subsystem torn down");
        }
    }

    pub fn subsystem_up(&self) -> bool {
        self.subsystem.is_some()
    }

    pub fn connected(&self) -> bool {
        self.device.is_some()
    }

    /// True when a controller is connected and controller input is on.
    pub fn active(&self) -> bool {
        self.enabled && self.device.is_some()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
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
        self.enabled = enabled;
        if enabled {
            self.init_subsystem();
        } else {
            self.teardown_subsystem();
            self.release_all();
            self.last_used = InputDevice::Keyboard;
        }
    }

    /// Bind `device` (what opening a pad does, and the test seam).
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
        self.device = Some(device);
    }

    /// Bind the controller at `index`; true on success.
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

    fn close_controller(&mut self) {
        self.device = None;
        self.instance_id = None;
        self.id_pending = false;
        self.buttons_down.clear();
    }

    fn is_attached(&self) -> bool {
        self.device.as_ref().is_some_and(|device| device.attached())
    }

    /// Drop the current pad and bind whatever is connected now.
    ///
    /// The first real event's instance id (see `id_pending`) corrects a
    /// stale id left by a hot-plug, so the SDL subsystem is never cycled:
    /// quitting and re-initializing it while the event loop is live
    /// re-registers SDL's controller event watch and makes every controller
    /// event arrive twice.
    pub fn reopen(&mut self) {
        self.close_controller();
        self.open_first();
    }

    /// Whether the bound pad went away since the last call.
    pub fn take_disconnect(&mut self) -> bool {
        std::mem::take(&mut self.disconnected)
    }

    /// Forget every button, stick position and repeat. The app calls this
    /// when the window loses focus: SDL drops the releases while the window
    /// is in the background, and a button remembered as down would make the
    /// next real press look like a duplicate.
    pub fn release_all(&mut self) {
        self.modifier = false;
        self.repeat.clear();
        self.buttons_down.clear();
        self.axes.clear();
    }

    // -- reads ------------------------------------------------------------------

    /// Record that the keyboard was just used, so prompts name keys.
    pub fn note_keyboard(&mut self) {
        self.last_used = InputDevice::Keyboard;
    }

    /// The controller while one is active and was used last, else the
    /// keyboard.
    pub fn device(&self) -> InputDevice {
        if self.active() && self.last_used == InputDevice::Controller {
            InputDevice::Controller
        } else {
            InputDevice::Keyboard
        }
    }

    /// Whether the right bumper is held: the second layer.
    pub fn modifier(&self) -> bool {
        self.active() && self.modifier
    }

    /// Whether `button` is held on the bound pad.
    pub fn is_down(&self, button: ControllerButton) -> bool {
        self.active() && self.buttons_down.contains(&button)
    }

    /// Where `axis` is, past its deadzone; 0.0 while no pad is active.
    pub fn axis(&self, axis: ControllerAxis) -> f64 {
        if self.active() {
            self.axes.get(&axis).copied().unwrap_or(0.0)
        } else {
            0.0
        }
    }

    // -- event handling -------------------------------------------------------

    /// Update the manager from a controller event. True for an event the
    /// top screen should see: a press or release from the bound pad, or the
    /// bound pad going away. Axis, add and foreign-id events return false,
    /// so a stray event -- a duplicate from a pad that enumerates twice --
    /// never reaches a game action.
    pub fn process_event(&mut self, event: &InputEvent) -> bool {
        let instance_id = match event {
            InputEvent::ControllerAdded { device_index } => {
                log::debug!("Controller device added (slot {device_index})");
                // Re-open when nothing is bound, or the bound pad has quietly
                // detached (a missed removal) -- otherwise a stale binding
                // would block the reconnect.
                if self.connected() && self.is_attached() {
                    log::debug!(
                        "Ignoring add; a pad is already bound (instance {:?})",
                        self.instance_id
                    );
                    return false;
                }
                self.reopen();
                return false;
            }
            InputEvent::ControllerRemoved { instance_id } => {
                log::debug!("Controller device removed (instance {instance_id})");
                if self.instance_id.is_some() && Some(*instance_id) == self.instance_id {
                    self.close_controller();
                    self.release_all();
                    self.disconnected = true;
                    self.logged_mismatch_id = None;
                    log::info!("Controller disconnected");
                    return true;
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
                // First real event after (re)open: its id is authoritative.
                log::info!(
                    "Controller bound to instance {instance_id} (opened as {:?})",
                    self.instance_id
                );
                self.instance_id = Some(instance_id);
                self.id_pending = false;
                self.logged_mismatch_id = None;
            } else {
                // A genuinely foreign id: a second pad, or a duplicate from
                // a pad that enumerates twice. Drop it, logging once.
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
            // Matched the bound id, so it is live: a duplicate under a
            // different id is now a duplicate.
            self.id_pending = false;
        }
        match event {
            InputEvent::ControllerAxis { axis, value, .. } => {
                self.on_axis(*axis, *value);
                false
            }
            InputEvent::ControllerButtonDown { button, .. } => {
                if !self.buttons_down.insert(*button) {
                    log::debug!(
                        "Ignoring duplicate button-down {button:?} (instance {instance_id})"
                    );
                    return false;
                }
                self.last_used = InputDevice::Controller;
                if *button == ControllerButton::RightShoulder {
                    self.modifier = true;
                }
                self.repeat.press(*button);
                true
            }
            InputEvent::ControllerButtonUp { button, .. } => {
                if !self.buttons_down.remove(button) {
                    return false; // a stray or duplicate release
                }
                if *button == ControllerButton::RightShoulder {
                    self.modifier = false;
                }
                self.repeat.release(*button);
                true
            }
            _ => false,
        }
    }

    fn on_axis(&mut self, axis: ControllerAxis, raw: i16) {
        let value = (f64::from(raw) / AXIS_MAX).clamp(-1.0, 1.0);
        let value = match axis {
            ControllerAxis::TriggerLeft | ControllerAxis::TriggerRight => {
                deadzone(value.max(0.0), TRIGGER_DEADZONE)
            }
            _ => deadzone(value, STICK_DEADZONE),
        };
        if value != 0.0 {
            self.last_used = InputDevice::Controller;
        }
        self.axes.insert(axis, value);
    }

    /// The frame's repeats of a held D-pad left or right, as presses marked
    /// `repeat`.
    pub fn tick(&mut self, dt: f64) -> Vec<InputEvent> {
        if !self.active() {
            return Vec::new();
        }
        let Some(button) = self.repeat.button() else {
            return Vec::new();
        };
        let count = self.repeat.tick(dt);
        let instance_id = self.instance_id.unwrap_or(0);
        (0..count)
            .map(|_| InputEvent::ControllerButtonDown {
                button,
                instance_id,
                repeat: true,
            })
            .collect()
    }

    /// Release the pad and the subsystem at the end of the run, and the SDL
    /// handle the factory holds, before the window begins its exit.
    pub fn shutdown(&mut self) {
        self.teardown_subsystem();
        self.factory = None;
    }
}
```

Create `crates/hearken-app/src/controller/sdl.rs`:

```rust
//! The SDL side of the controller seam: [`PadDevice`] and [`PadSubsystem`]
//! over `sdl2::controller`, and the factory the app hands the manager.
//! Converting SDL's buttons and axes is `sdl_shell`'s job.

use sdl2::controller::GameController;
use sdl2::GameControllerSubsystem;

use super::{PadDevice, PadSubsystem, PadSubsystemFactory};

/// An opened pad. Dropping it closes the device.
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
}

/// The game controller subsystem. Dropping it quits the subsystem.
pub struct SdlPadSubsystem {
    subsystem: GameControllerSubsystem,
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

/// A factory that brings SDL's game controller subsystem up on demand.
/// Every failure (no SDL, dummy drivers) is "no subsystem": keyboard only.
pub fn sdl_factory(sdl: sdl2::Sdl) -> PadSubsystemFactory {
    Box::new(move || match sdl.game_controller() {
        Ok(subsystem) => Some(Box::new(SdlPadSubsystem { subsystem }) as Box<dyn PadSubsystem>),
        Err(e) => {
            log::info!("Controller subsystem unavailable; keyboard only: {e}");
            None
        }
    })
}
```

Create `crates/hearken-app/src/controller/fakes.rs`:

```rust
//! Test doubles for the controller seam: a pad whose presence the test
//! controls, and a subsystem that hands out pads and counts how often it
//! was brought up, torn down and asked to open one.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::{PadDevice, PadSubsystem, PadSubsystemFactory};

/// A pad with a fixed instance id.
pub struct FakePad {
    pub instance_id: u32,
    /// Set false to unplug it without a removal event.
    pub attached: Rc<Cell<bool>>,
}

impl FakePad {
    pub fn new(instance_id: u32) -> Self {
        FakePad {
            instance_id,
            attached: Rc::new(Cell::new(true)),
        }
    }
}

impl PadDevice for FakePad {
    fn instance_id(&self) -> u32 {
        self.instance_id
    }

    fn attached(&self) -> bool {
        self.attached.get()
    }
}

/// What a [`FakeSubsystem`] and its factory saw.
#[derive(Debug, Default)]
pub struct FakeSdlLog {
    /// How many times the factory brought a subsystem up.
    pub inits: u32,
    /// How many subsystems were dropped.
    pub quits: u32,
    /// Every `open(index)` call.
    pub opens: Vec<u32>,
}

/// A subsystem with `pads` recognised controllers (name, instance id).
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

/// A factory handing out [`FakeSubsystem`]s with `pads`, recording into
/// `log`.
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

- [ ] **Step 4: The controller setting, the context, the loop and the window**

`crates/hearken-app/src/settings.rs`: add the field to `Settings` (after `menus`):

```rust
    #[serde(default)]
    pub controller: ControllerSettings,
```

and, after `impl Default for MenuSettings`:

```rust
/// Whether the game listens to a game controller.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ControllerSettings {
    pub enabled: bool,
}

impl Default for ControllerSettings {
    fn default() -> Self {
        ControllerSettings { enabled: true }
    }
}
```

`crates/hearken-app/src/ctx.rs`: add `use crate::controller::ControllerManager;`, the field after `held`:

```rust
    /// The bound game controller. Headless contexts get one that never
    /// reaches a real subsystem; `run` swaps in SDL's.
    pub controller: ControllerManager,
```

and in `Ctx::new`, before building the struct, `let controller = ControllerManager::detached(settings.controller.enabled);`, then `controller,` in the struct literal.

`crates/hearken-app/src/app.rs`: add `use crate::controller::CONTROLLER_DISCONNECTED;`. In `admit`, replace the `WindowFocusLost` and `KeyDown` arms and add the controller arm, so the match reads:

```rust
        match event {
            InputEvent::Quit => {
                self.ctx.running = false;
                return false;
            }
            // Coming back to the window re-checks which screen reader is
            // running and the keyboard's repeat timing, either of which the
            // player may have changed while away, and nothing held before
            // they left is held now. The screen still sees the event.
            InputEvent::WindowFocusGained => {
                self.ctx.speech.request_refresh();
                self.ctx.held.refresh_repeat_timing();
                self.ctx.held.clear_pulses();
            }
            InputEvent::WindowFocusLost => {
                self.ctx.held.clear();
                self.ctx.controller.release_all();
            }
            InputEvent::KeyDown { key, mods, .. } => {
                self.ctx.held.press(*key, *mods);
                self.ctx.controller.note_keyboard();
            }
            InputEvent::KeyUp { key, mods } => self.ctx.held.release(*key, *mods),
            // Only the bound pad's presses and releases, and its removal,
            // reach a screen.
            e if e.is_controller_event() => {
                if !self.ctx.controller.process_event(e) {
                    return false;
                }
                if self.ctx.controller.take_disconnect() {
                    self.ctx.say(CONTROLLER_DISCONNECTED);
                }
            }
            _ => {}
        }
        true
```

In `step`, right after the `for event in events { ... }` loop:

```rust
        // A held D-pad left or right repeats, for adjusting a setting.
        for event in self.ctx.controller.tick(dt) {
            self.dispatch(&event);
        }
```

In `run`, right after `G::on_exit(&mut self.ctx);`:

```rust
        self.ctx.controller.shutdown();
```

`crates/hearken-app/src/sdl_shell.rs`: rename the field `_sdl` to `sdl` (struct and constructor), add `use crate::controller::sdl::sdl_factory;` and `use crate::controller::PadSubsystemFactory;`, and add to `impl SdlShell`:

```rust
    /// Brings SDL's game controller subsystem up when the controller
    /// manager asks for it.
    pub fn controller_factory(&self) -> PadSubsystemFactory {
        sdl_factory(self.sdl.clone())
    }
```

`crates/hearken-app/src/run.rs`: add `use crate::controller::ControllerManager;` and, right after `let mut ctx = Ctx::new(...)`:

```rust
    ctx.controller = ControllerManager::new(
        ctx.settings.controller.enabled,
        Some(shell.controller_factory()),
    );
```

`crates/hearken-app/src/testing.rs`: add `use crate::controller::fakes::FakePad;` and to `impl<G: Game> Headless<G>`:

```rust
    /// Bind a fake controller as instance 0, the id `InputEvent::button`
    /// sends.
    pub fn connect_pad(&mut self) {
        let controller = &mut self.app.ctx.controller;
        controller.bind_device(Box::new(FakePad::new(0)), "test pad");
        controller.set_id_pending(false);
    }
```

`crates/hearken-app/src/lib.rs`: add `pub mod controller;` (after `pub mod app;`), `pub use controller::{ControllerManager, InputDevice};`, and add `ControllerSettings` to the settings re-export: `pub use settings::{ControllerSettings, MenuSettings, Settings};`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p hearken-app --test controller --test app --test input --test screens > target/t2.txt 2>&1`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/hearken-app
git commit -m "feat(app): the controller manager

One bound pad, hot-plug without cycling SDL, duplicate and foreign
events dropped, held D-pad repeat, and a spoken line when the pad goes
away. From Freight Fate c5d3ef8d without its pedals and rumble.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Rebindable actions

**Files:**
- Create: `crates/hearken-app/src/bindings.rs`, `src/bindings/names.rs`
- Modify: `crates/hearken-app/src/game.rs`, `src/ctx.rs`, `src/settings.rs`, `src/app.rs`, `src/screens/settings.rs`, `src/text.rs`, `src/lib.rs`
- Modify: `crates/hearken-app/tests/common/mod.rs`, `template/src/lib.rs`, `template/src/practice.rs`
- Test: create `crates/hearken-app/tests/bindings.rs`

**Interfaces:**
- Consumes: Task 1's `HeldKeys`, `InputEvent`; Task 2's `ControllerManager::{modifier, is_down, device}`, `InputDevice`.
- Produces:
  - `hearken_app::bindings::Action` trait (re-exported at the root): `fn all() -> &'static [Self]`, `fn id(self) -> &'static str`, `fn label(self) -> &'static str`, `fn default_keys(self) -> &'static [Chord]`, `fn default_pad(self) -> &'static [PadChord]` (default `&[]`), `fn reserved_key(chord: &Chord) -> Option<&'static str>` (default `None`), `fn reserved_pad(chord: &PadChord) -> Option<&'static str>` (default `None`).
  - `NoAction` (uninhabited; a game with nothing to rebind), `Chord { key, mods }` (`plain`, `shift`, `alt`, `spoken`, `saved`, `parse`), `PadChord { button, modified }` (`plain`, `modified`, `spoken`, `saved`, `parse`), `Rebind<A> { Done, Unchanged, Reserved(&'static str), Taken(A) }`, `reserved_key_reason(&Chord)`, `reserved_pad_reason(&PadChord)`, `SavedBindings { keys: BTreeMap<String, String>, pad: BTreeMap<String, String> }`, `key_saved_name`, `key_spoken_name`, `parse_key_name`, `pad_button_short_name`.
  - `KeyBindings<A: Action>`: `default()`, `from_saved(&SavedBindings)`, `store(&self, &mut SavedBindings)`, `is_default()`, `reset_keys()`, `reset_pad()`, `chords(A) -> Vec<Chord>`, `spoken(A) -> String`, `action_for(Key, Mods) -> Option<A>`, `pressed(&HeldKeys, A) -> bool`, `set_chord(A, Chord) -> Rebind<A>`, `pad_chords(A) -> Vec<PadChord>`, `pad_spoken(A) -> String`, `pad_action_for(ControllerButton, bool) -> Option<A>`, `set_pad_chord(A, PadChord) -> Rebind<A>`.
  - `Game::Action: Action` (new required associated type).
  - `Settings.bindings: SavedBindings`.
  - `Ctx::bindings: KeyBindings<G::Action>`; `Ctx::action_for(&InputEvent) -> Option<G::Action>`, `Ctx::action_held(G::Action) -> bool`, `Ctx::control_name(G::Action) -> String`, `Ctx::save_settings(&self)`, `Ctx::save_bindings(&mut self)`.
  - `hearken_app::text::sentence_start(&str) -> String`.
  - Test game: `common::TestAction { Jump, Duck, Map, Horn }` (defaults below). Template: `hello_hearken::HelloAction::Count` (Space; pad A).

- [ ] **Step 1: The test game's actions**

In `crates/hearken-app/tests/common/mod.rs`, change the import to:

```rust
use hearken_app::{Action, Chord, ControllerButton, Ctx, Game, InputEvent, Key, PadChord, State};
```

add `type Action = TestAction;` to `impl Game for TestGame` (after `type Settings = ();`), and add:

```rust
/// Jump: Space, pad A. Duck: C or keypad 0, pad X. Open the map: Shift M,
/// keyboard only. Horn: right bumper plus Y, pad only. Tab is the game's
/// own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestAction {
    Jump,
    Duck,
    Map,
    Horn,
}

impl Action for TestAction {
    fn all() -> &'static [Self] {
        &[
            TestAction::Jump,
            TestAction::Duck,
            TestAction::Map,
            TestAction::Horn,
        ]
    }
    fn id(self) -> &'static str {
        match self {
            TestAction::Jump => "jump",
            TestAction::Duck => "duck",
            TestAction::Map => "map",
            TestAction::Horn => "horn",
        }
    }
    fn label(self) -> &'static str {
        match self {
            TestAction::Jump => "Jump",
            TestAction::Duck => "Duck",
            TestAction::Map => "Open the map",
            TestAction::Horn => "Horn",
        }
    }
    fn default_keys(self) -> &'static [Chord] {
        match self {
            TestAction::Jump => const { &[Chord::plain(Key::Space)] },
            TestAction::Duck => const { &[Chord::plain(Key::C), Chord::plain(Key::Kp0)] },
            TestAction::Map => const { &[Chord::shift(Key::M)] },
            TestAction::Horn => &[],
        }
    }
    fn default_pad(self) -> &'static [PadChord] {
        match self {
            TestAction::Jump => const { &[PadChord::plain(ControllerButton::A)] },
            TestAction::Duck => const { &[PadChord::plain(ControllerButton::X)] },
            TestAction::Map => &[],
            TestAction::Horn => const { &[PadChord::modified(ControllerButton::Y)] },
        }
    }
    fn reserved_key(chord: &Chord) -> Option<&'static str> {
        (chord.key == Key::Tab).then_some("Tab reads the score")
    }
}
```

(`const { ... }` blocks give the `&'static` slices: a `const fn` call is not promoted to `'static` on its own.)

- [ ] **Step 2: Write the failing tests**

Create `crates/hearken-app/tests/bindings.rs`:

```rust
mod common;

use common::{Probe, TestAction as T, TestData, TestGame};
use hearken_app::bindings::{
    key_saved_name, parse_key_name, Chord, KeyBindings, PadChord, Rebind, SavedBindings,
};
use hearken_app::{ControllerButton as B, Ctx, Headless, HeldKeys, InputEvent, Key, Mods, Paths, Settings};
use hearken_speech::CaptureSpeech;

#[test]
fn named_keys_read_back_from_their_saved_names() {
    for key in [Key::A, Key::Num0, Key::Kp5, Key::F12, Key::Up, Key::Quote, Key::RAlt] {
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
    assert_eq!(b.set_chord(T::Jump, Chord::plain(Key::C)), Rebind::Taken(T::Duck));
    assert_eq!(b.set_chord(T::Jump, Chord::plain(Key::Z)), Rebind::Done);
    assert_eq!(b.action_for(Key::Z, Mods::NONE), Some(T::Jump));
    assert_eq!(b.action_for(Key::Space, Mods::NONE), None);
    assert_eq!(b.set_chord(T::Jump, Chord::plain(Key::Z)), Rebind::Unchanged);
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
    assert_eq!(b.action_for(Key::M, Mods::NONE), None, "the map needs its Shift");
}

#[test]
fn pad_buttons_move_on_their_own_layer() {
    let mut b = KeyBindings::<T>::default();
    assert_eq!(b.pad_action_for(B::Y, true), Some(T::Horn));
    assert_eq!(b.pad_action_for(B::Y, false), None);
    assert_eq!(b.set_pad_chord(T::Horn, PadChord::plain(B::X)), Rebind::Taken(T::Duck));
    assert_eq!(b.set_pad_chord(T::Horn, PadChord::plain(B::Y)), Rebind::Done);
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
    assert_eq!(saved.pad.get("horn").map(String::as_str), Some("left_stick"));
    assert_eq!(KeyBindings::<T>::from_saved(&saved), b);

    // A hand-edited file: a key the game cannot name, an action it no longer
    // has, and a key for a control the keyboard does not have.
    saved.keys.insert("duck".to_string(), "not_a_key".to_string());
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
    settings.bindings.keys.insert("jump".to_string(), "z".to_string());
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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p hearken-app --test bindings > target/t3.txt 2>&1`
Expected: compile errors: no module `bindings`, no `Action` in `hearken_app`, no field `bindings`.

- [ ] **Step 4: Names, extracted verbatim**

```bash
mkdir -p crates/hearken-app/src/bindings
git -C C:/Users/joshu/Freight-fate show c5d3ef8d:crates/freight-fate/src/bindings/names.rs > crates/hearken-app/src/bindings/names.rs
```

Edit it: replace the two lines

```rust
use crate::controller::ControllerButton;
use crate::states::base::Key;
```

with `use crate::input::{ControllerButton, Key};`, and change the module doc's last line to end `the screen reader says. Extracted from Freight Fate (commit c5d3ef8d).`

- [ ] **Step 5: Bindings**

Create `crates/hearken-app/src/bindings.rs`:

```rust
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
    /// id or key name, or a key for a control the keyboard does not have)
    /// is skipped, so one bad line never costs the rest.
    pub fn from_saved(saved: &SavedBindings) -> Self {
        let mut out = Self::default();
        for (id, value) in &saved.keys {
            if let (Some(action), Some(chord)) = (from_id::<A>(id), Chord::parse(value)) {
                if !action.default_keys().is_empty() {
                    out.keys.insert(action, chord);
                }
            }
        }
        for (id, value) in &saved.pad {
            if let (Some(action), Some(chord)) = (from_id::<A>(id), PadChord::parse(value)) {
                if !action.default_pad().is_empty() {
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
```

- [ ] **Step 6: The game, the settings, the context**

`crates/hearken-app/src/game.rs`: add `use crate::bindings::Action;` and, after `type Settings`:

```rust
    /// The controls the player can move to other keys and buttons, from
    /// Settings. `hearken_app::NoAction` when there are none.
    type Action: Action;
```

`crates/hearken-app/src/settings.rs`: add `use crate::bindings::SavedBindings;` and the field after `controller`:

```rust
    #[serde(default)]
    pub bindings: SavedBindings,
```

`crates/hearken-app/src/ctx.rs`: add `use crate::bindings::KeyBindings;`, `use crate::controller::InputDevice;` and `use crate::input::InputEvent;`; the field after `controller`:

```rust
    /// The game's controls, with the player's changes. Change them through
    /// `KeyBindings::set_chord` and friends, then `save_bindings`.
    pub bindings: KeyBindings<G::Action>,
```

and in `Ctx::new`, before the struct literal, `let bindings = KeyBindings::from_saved(&settings.bindings);`, then `bindings,` in the literal. Add these methods to `impl<G: Game> Ctx<G>` (after `apply_speech_settings`):

```rust
    /// Write `settings.json`. A failure is logged: the game plays on with
    /// the settings it has.
    pub fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.paths) {
            log::error!("could not save settings: {e}");
        }
    }

    /// Keep the player's bindings in the settings and save them.
    pub fn save_bindings(&mut self) {
        self.bindings.store(&mut self.settings.bindings);
        self.save_settings();
    }

    /// The game action a key press or pad button means, by the player's
    /// bindings. `None` for anything else, and for a press no action has.
    pub fn action_for(&self, event: &InputEvent) -> Option<G::Action> {
        match event {
            InputEvent::KeyDown { key, mods, .. } => self.bindings.action_for(*key, *mods),
            InputEvent::ControllerButtonDown { button, .. } => self
                .bindings
                .pad_action_for(*button, self.controller.modifier()),
            _ => None,
        }
    }

    /// Whether `action`'s key or pad button is held down right now.
    pub fn action_held(&self, action: G::Action) -> bool {
        self.bindings.pressed(&self.held, action)
            || self.bindings.pad_chords(action).iter().any(|chord| {
                self.controller.is_down(chord.button)
                    && (!chord.modified || self.controller.modifier())
            })
    }

    /// `action`'s control as a prompt names it: the pad button while the
    /// player is on a controller and the action has one, else the key.
    pub fn control_name(&self, action: G::Action) -> String {
        if self.controller.device() == InputDevice::Controller
            && !self.bindings.pad_chords(action).is_empty()
        {
            self.bindings.pad_spoken(action)
        } else {
            self.bindings.spoken(action)
        }
    }
```

Use `save_settings` for the two existing saves: in `crates/hearken-app/src/app.rs` `run`, replace

```rust
        if let Err(e) = self.ctx.settings.save(&self.ctx.paths) {
            log::error!("could not save settings: {e}");
        }
```

with `self.ctx.save_settings();`, and in `crates/hearken-app/src/screens/settings.rs` `on_escape` the same block (with `ctx.` in place of `self.ctx.`) becomes `ctx.save_settings();`.

`crates/hearken-app/src/text.rs`: add

```rust
/// `phrase` with its first letter capitalised, to open a sentence: "the A
/// button" becomes "The A button".
pub fn sentence_start(phrase: &str) -> String {
    let mut chars = phrase.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
```

`crates/hearken-app/src/lib.rs`: add `pub mod bindings;` (after `pub mod app;`) and `pub use bindings::{Action, Chord, KeyBindings, NoAction, PadChord};`.

- [ ] **Step 7: The template's Count action**

`template/src/lib.rs` becomes:

```rust
//! Hello Hearken: the smallest complete Hearken game.

mod practice;
mod title;

use hearken_app::{Action, Chord, ControllerButton, Key, PadChord};

pub use practice::Practice;
pub use title::Title;

/// The help document, compiled in.
pub const HELP: &str = include_str!("../help.md");

pub struct HelloGame;

impl hearken_app::Game for HelloGame {
    const TITLE: &'static str = "Hello Hearken";
    const DATA_DIR: &'static str = "HelloHearken";
    const ENV_PREFIX: &'static str = "HELLO_HEARKEN";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = HelloAction;
}

/// The controls the player can move to other keys and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HelloAction {
    Count,
}

impl Action for HelloAction {
    fn all() -> &'static [Self] {
        &[HelloAction::Count]
    }
    fn id(self) -> &'static str {
        "count"
    }
    fn label(self) -> &'static str {
        "Count"
    }
    fn default_keys(self) -> &'static [Chord] {
        const { &[Chord::plain(Key::Space)] }
    }
    fn default_pad(self) -> &'static [PadChord] {
        const { &[PadChord::plain(ControllerButton::A)] }
    }
}
```

`template/src/practice.rs`: change the imports to

```rust
use hearken_app::screens::PauseScreen;
use hearken_app::text::sentence_start;
use hearken_app::{Ctx, InputEvent, Key, State};

use crate::{HelloAction, HelloGame, Title, HELP};
```

add, above `impl State<HelloGame> for Practice`:

```rust
/// "Space", or "The A button" on a controller, to open a sentence.
fn count_control(ctx: &Ctx<HelloGame>) -> String {
    sentence_start(&ctx.control_name(HelloAction::Count))
}
```

and replace `enter`, `handle_input` and `help` with:

```rust
    fn enter(&mut self, ctx: &mut Ctx<HelloGame>) {
        let control = count_control(ctx);
        ctx.say(format!("The practice room. {control} counts, Escape pauses."));
    }
```

```rust
    fn handle_input(&mut self, ctx: &mut Ctx<HelloGame>, event: &InputEvent) {
        // A held key repeats; one press is one count, one pause.
        if event.is_repeat() {
            return;
        }
        if ctx.action_for(event) == Some(HelloAction::Count) {
            self.count += 1;
            ctx.say(self.count.to_string());
        } else if event.pressed() == Some(Key::Escape) {
            ctx.push(PauseScreen::<HelloGame>::new(HELP, to_title));
        }
    }
```

```rust
    fn help(&self, ctx: &Ctx<HelloGame>) -> String {
        format!(
            "{} counts out loud. Escape opens the pause menu.",
            count_control(ctx)
        )
    }
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p hearken-app --test bindings --test screens --test app > target/t3.txt 2>&1`, then `cargo test -p hello-hearken > target/t3-template.txt 2>&1`.
Expected: all pass. `screens.rs`'s row counts are unchanged in this task (the Settings rows arrive in Task 4).

- [ ] **Step 9: Commit**

```bash
cargo fmt --all
git add crates/hearken-app template
git commit -m "feat(app): rebindable actions

A game names its controls with Game::Action; Ctx carries the live
bindings, saved in settings.json, and answers what a press means, whether
a control is held, and what to call it in a prompt. From Freight Fate
c5d3ef8d's bindings with its driving table taken out.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The rebinding screen, and the input rows in Settings

**Files:**
- Create: `crates/hearken-app/src/screens/bindings.rs`
- Modify: `crates/hearken-app/src/screens/mod.rs`, `src/screens/settings.rs`
- Modify: `template/src/title.rs`, `template/help.md`
- Test: `crates/hearken-app/tests/bindings.rs` (append), `crates/hearken-app/tests/screens.rs`, `template/tests/walkthrough.rs`

**Interfaces:**
- Consumes: Task 3's `Action`, `Chord`, `PadChord`, `Rebind`, `Ctx::{bindings, save_bindings, save_settings}`, `sentence_start`; Task 2's `ControllerManager::{modifier, set_enabled, enabled}`, `Settings.controller`; M1's `Menu`, `menu_input`.
- Produces:
  - `hearken_app::screens::{BindingDevice, BindingsScreen}`: `BindingDevice { Keyboard, Controller }`, `BindingsScreen::<G>::new(BindingDevice) -> Self`, `capturing() -> Option<G::Action>`; `State::name` is `"bindings"`.
  - Settings rows, in order: Speech rate (when the voice can change rate), Menu positions, Keyboard shortcuts (when any action has a key), Controller buttons (when any action has a pad button), Controller: on/off.

- [ ] **Step 1: Write the failing tests**

Append to `crates/hearken-app/tests/bindings.rs` (and add `use hearken_app::screens::{BindingDevice, BindingsScreen};` to its imports):

```rust
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
    assert_eq!(h.spoken(), ["Press the new key for Jump. Escape keeps Space."]);
}

#[test]
fn reset_puts_every_key_back_and_saves() {
    let mut h = shortcuts(BindingDevice::Keyboard);
    h.press(Key::Return);
    h.press(Key::Z);
    h.press(Key::End);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Every keyboard shortcut is back to its default."]);
    assert!(h.app.ctx.bindings.is_default());
    let saved = std::fs::read_to_string(h.app.ctx.paths.settings_file()).unwrap();
    assert!(!saved.contains("\"jump\""), "saved: {saved}");
}

#[test]
fn the_controller_screen_takes_a_button_and_the_right_bumper_picks_the_second_layer() {
    let mut h = shortcuts(BindingDevice::Controller);
    assert_eq!(h.spoken(), ["Controller buttons.", "Jump: the A button. 1 of 4."]);
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
    assert_eq!(h.app.ctx.bindings.pad_action_for(B::LeftStick, true), Some(T::Jump));
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
```

In `crates/hearken-app/tests/screens.rs`: the Settings screen gains three rows for `TestGame`. Update the expected lines:

- `speech_rate_moves_by_five_percent_clamps_and_reaches_the_voice`: `"Speech rate: 50 percent. 1 of 5."`
- `the_new_rate_is_read_back_through_the_voice_it_changed`: `let line = "Speech rate: 55 percent. 1 of 5.";`
- `without_a_voice_that_can_change_rate_there_is_no_rate_row`: `["Settings.", "Menu positions: on. 1 of 4."]`
- `the_menu_positions_row_toggles_on_enter_left_and_right_and_reads_back_once`: `"Menu positions: on. 2 of 5."`

and add (with `use hearken_app::{Game, NoAction};` in the imports):

```rust
#[test]
fn settings_open_the_shortcut_screens_and_turn_the_controller_off_and_on() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    h.press(Key::Down);
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Keyboard shortcuts.", "Jump: Space. 1 of 4."]);
    h.press(Key::Escape);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "bindings");
    h.press(Key::Escape);
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Controller: off. 5 of 5."]);
    assert!(!h.app.ctx.settings.controller.enabled);
    assert!(!h.app.ctx.controller.enabled());
    h.press(Key::Right);
    assert!(h.app.ctx.controller.enabled());
}

struct Bare;

impl Game for Bare {
    const TITLE: &'static str = "Bare";
    const DATA_DIR: &'static str = "HearkenBareTest";
    const ENV_PREFIX: &'static str = "HEARKEN_BARE_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
}

#[test]
fn a_game_with_nothing_to_rebind_gets_no_shortcut_rows() {
    let h = Headless::<Bare>::new((), SettingsScreen::<Bare>::new());
    assert_eq!(h.spoken(), ["Settings.", "Speech rate: 50 percent. 1 of 3."]);
}
```

In `template/tests/walkthrough.rs`, `settings_open_and_the_title_comes_back_where_it_was` expects `"Speech rate: 50 percent. 1 of 5."`, and add:

```rust
#[test]
fn count_can_move_to_another_key() {
    let mut h = start();
    h.press(Key::Down);
    h.press(Key::Return); // Settings
    h.press(Key::Down);
    h.press(Key::Down);
    h.press(Key::Return); // Keyboard shortcuts
    h.press(Key::Return); // Count
    h.press(Key::C);
    assert_eq!(h.spoken().last().map(String::as_str), Some("Count is now C."));
    h.press(Key::Escape);
    h.press(Key::Escape);
    h.press(Key::Home);
    h.clear();
    h.press(Key::Return); // Play
    assert_eq!(h.spoken(), ["The practice room. C counts, Escape pauses."]);
    h.clear();
    h.press(Key::C);
    h.press(Key::Space);
    assert_eq!(h.spoken(), ["1"]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hearken-app --test bindings --test screens > target/t4.txt 2>&1`
Expected: compile errors: no `BindingDevice`/`BindingsScreen` in `hearken_app::screens`.

- [ ] **Step 3: The rebinding screen**

Create `crates/hearken-app/src/screens/bindings.rs`:

```rust
//! Keyboard shortcuts, or controller buttons: one row per control, naming
//! the key or button it is on now. Enter on a row, then press the key or
//! button you want for it.
//!
//! The capture is one press. A key that is fixed, or already another
//! control's, is refused with the reason and the screen keeps listening;
//! Escape keeps what the row had. On the controller the right bumper held
//! during the press picks the second layer, and pressing the button the row
//! already has keeps it. Each change is saved as it lands, so a crash a
//! minute later loses nothing.
//!
//! Extracted from Freight Fate (commit c5d3ef8d),
//! `states/main_menu/shortcuts.rs`.

use crate::bindings::{Action, Chord, PadChord, Rebind};
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{ControllerButton, InputEvent, Key, Mods};
use crate::menu::{menu_input, Label, Menu, MenuCore, MenuItem};
use crate::text::sentence_start;

/// Which set of bindings a [`BindingsScreen`] moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingDevice {
    Keyboard,
    Controller,
}

pub struct BindingsScreen<G: Game> {
    menu: MenuCore<Self, G>,
    device: BindingDevice,
    /// The control waiting for its new key or button.
    capturing: Option<G::Action>,
}

impl<G: Game> BindingsScreen<G> {
    pub fn new(device: BindingDevice) -> Self {
        let (title, help) = match device {
            BindingDevice::Keyboard => (
                "Keyboard shortcuts",
                "Up and Down pick a control, Enter then a key press moves it, Escape goes back.",
            ),
            BindingDevice::Controller => (
                "Controller buttons",
                "Up and Down pick a control, Enter then a button press moves it, Escape goes back.",
            ),
        };
        BindingsScreen {
            menu: MenuCore::new(title).with_intro_help(help),
            device,
            capturing: None,
        }
    }

    /// The control waiting for its new key or button, if any.
    pub fn capturing(&self) -> Option<G::Action> {
        self.capturing
    }

    fn current_name(&self, ctx: &Ctx<G>, action: G::Action) -> String {
        match self.device {
            BindingDevice::Keyboard => ctx.bindings.spoken(action),
            BindingDevice::Controller => ctx.bindings.pad_spoken(action),
        }
    }

    fn prompt(&self, ctx: &Ctx<G>, action: G::Action) -> String {
        let current = self.current_name(ctx, action);
        match self.device {
            BindingDevice::Keyboard => format!(
                "Press the new key for {}. Escape keeps {current}.",
                action.label()
            ),
            BindingDevice::Controller => format!(
                "Press the new button for {}, with the right bumper held for the second \
                 layer. Press {current} to keep it.",
                action.label()
            ),
        }
    }

    /// How a refusal ends: the way to keep what the control has.
    fn keep_hint(&self, current: &str) -> String {
        match self.device {
            BindingDevice::Keyboard => format!("Press another, or keep {current} with Escape."),
            BindingDevice::Controller => format!("Press another, or press {current} to keep it."),
        }
    }

    fn begin_capture(&mut self, ctx: &mut Ctx<G>, action: G::Action) {
        self.capturing = Some(action);
        let prompt = self.prompt(ctx, action);
        ctx.say(prompt);
    }

    fn keep(&mut self, ctx: &mut Ctx<G>, action: G::Action) {
        self.capturing = None;
        let current = self.current_name(ctx, action);
        ctx.say(format!("Kept {current}."));
    }

    fn finish(
        &mut self,
        ctx: &mut Ctx<G>,
        action: G::Action,
        result: Rebind<G::Action>,
        chosen: &str,
    ) {
        match result {
            Rebind::Done => {
                self.capturing = None;
                ctx.save_bindings();
                let now = self.current_name(ctx, action);
                ctx.say(format!("{} is now {now}.", action.label()));
            }
            Rebind::Unchanged => self.keep(ctx, action),
            Rebind::Taken(other) => {
                let hint = self.keep_hint(&self.current_name(ctx, action));
                ctx.say(format!(
                    "{} is already used for {}. {hint}",
                    sentence_start(chosen),
                    other.label()
                ));
            }
            Rebind::Reserved(reason) => {
                let hint = self.keep_hint(&self.current_name(ctx, action));
                ctx.say(format!("{}. {hint}", sentence_start(reason)));
            }
        }
    }

    fn capture_key(&mut self, ctx: &mut Ctx<G>, action: G::Action, key: Key, mods: Mods) {
        if matches!(
            key,
            Key::LCtrl | Key::RCtrl | Key::LShift | Key::RShift | Key::LAlt | Key::RAlt
        ) {
            return; // half a chord: wait for the key that goes with it
        }
        let chord = Chord { key, mods };
        let result = ctx.bindings.set_chord(action, chord);
        self.finish(ctx, action, result, &chord.spoken());
    }

    fn capture_button(&mut self, ctx: &mut Ctx<G>, action: G::Action, button: ControllerButton) {
        if button == ControllerButton::RightShoulder {
            return; // the second-layer key, held with the button that is the choice
        }
        let chord = PadChord {
            button,
            modified: ctx.controller.modifier(),
        };
        let result = ctx.bindings.set_pad_chord(action, chord);
        self.finish(ctx, action, result, &chord.spoken());
    }

    fn reset_all(&mut self, ctx: &mut Ctx<G>) {
        let line = match self.device {
            BindingDevice::Keyboard => {
                ctx.bindings.reset_keys();
                "Every keyboard shortcut is back to its default."
            }
            BindingDevice::Controller => {
                ctx.bindings.reset_pad();
                "Every controller button is back to its default."
            }
        };
        ctx.save_bindings();
        ctx.say(line);
    }
}

impl<G: Game> Menu<G> for BindingsScreen<G> {
    fn name(&self) -> &'static str {
        "bindings"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let device = self.device;
        let row_help = match device {
            BindingDevice::Keyboard => {
                "Enter, then press the key you want for this control. Escape keeps the one it has."
            }
            BindingDevice::Controller => {
                "Enter, then press the button you want for this control, with the right bumper \
                 held for the second layer."
            }
        };
        let mut items: Vec<MenuItem<Self, G>> = <G::Action as Action>::all()
            .iter()
            .copied()
            .filter(|action| match device {
                BindingDevice::Keyboard => !action.default_keys().is_empty(),
                BindingDevice::Controller => !action.default_pad().is_empty(),
            })
            .map(|action| {
                // The label is read live, so a row names its new key the
                // moment it moves.
                MenuItem::new(
                    Label::dynamic(move |s: &Self, ctx: &Ctx<G>| {
                        format!("{}: {}", action.label(), s.current_name(ctx, action))
                    }),
                    move |s: &mut Self, ctx: &mut Ctx<G>| s.begin_capture(ctx, action),
                )
                .help(row_help)
            })
            .collect();
        items.push(
            MenuItem::new(
                match device {
                    BindingDevice::Keyboard => "Reset every keyboard shortcut",
                    BindingDevice::Controller => "Reset every controller button",
                },
                |s: &mut Self, ctx: &mut Ctx<G>| s.reset_all(ctx),
            )
            .help("Puts every control on this screen back where it started."),
        );
        items
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        let Some(action) = self.capturing else {
            return menu_input(self, ctx, event);
        };
        if event.is_repeat() {
            return;
        }
        match (self.device, event) {
            (_, InputEvent::KeyDown { key: Key::Escape, .. }) => self.keep(ctx, action),
            (BindingDevice::Keyboard, InputEvent::KeyDown { key, mods, .. }) => {
                self.capture_key(ctx, action, *key, *mods)
            }
            (BindingDevice::Controller, InputEvent::ControllerButtonDown { button, .. }) => {
                self.capture_button(ctx, action, *button)
            }
            // A button on the keyboard screen, or a key on the controller
            // screen, is not a choice.
            _ => {}
        }
    }

    /// While listening, F1 repeats what the screen is waiting for.
    fn help(&self, ctx: &Ctx<G>) -> String {
        match self.capturing {
            Some(action) => self.prompt(ctx, action),
            None => self
                .current_help(ctx)
                .unwrap_or_else(|| self.menu().intro_help.clone()),
        }
    }
}
```

`crates/hearken-app/src/screens/mod.rs`: add `pub mod bindings;` (first) and `pub use bindings::{BindingDevice, BindingsScreen};`.

- [ ] **Step 4: The Settings rows**

In `crates/hearken-app/src/screens/settings.rs`:

- imports become:

```rust
use crate::bindings::Action;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Label, Menu, MenuCore, MenuItem};
use crate::screens::{BindingDevice, BindingsScreen};
```

- after `toggle_positions`, add:

```rust
fn toggle_controller<G: Game>(ctx: &mut Ctx<G>) {
    let on = !ctx.settings.controller.enabled;
    ctx.settings.controller.enabled = on;
    ctx.controller.set_enabled(on);
}
```

- in `build_items`, between the Menu positions row and `items` (the final return), add:

```rust
        let actions = <G::Action as Action>::all();
        if actions.iter().any(|action| !action.default_keys().is_empty()) {
            items.push(
                MenuItem::new("Keyboard shortcuts", |_s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(BindingsScreen::<G>::new(BindingDevice::Keyboard))
                })
                .help("Move any control to another key."),
            );
        }
        if actions.iter().any(|action| !action.default_pad().is_empty()) {
            items.push(
                MenuItem::new("Controller buttons", |_s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(BindingsScreen::<G>::new(BindingDevice::Controller))
                })
                .help("Move any control to another controller button."),
            );
        }
        items.push(
            MenuItem::new(
                Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    let on = ctx.settings.controller.enabled;
                    format!("Controller: {}", if on { "on" } else { "off" })
                }),
                |s: &mut Self, ctx: &mut Ctx<G>| {
                    toggle_controller(ctx);
                    // The rows are lent to this action while it runs; rebuild
                    // them so the read-back finds this row.
                    Menu::refresh(s, ctx, true);
                    s.speak_current(ctx);
                },
            )
            .help(
                "Whether the game listens to a game controller. Enter, Left or Right changes it.",
            )
            .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_controller(ctx)),
        );
```

- the intro help stays; the module doc's first line becomes `//! The engine's settings, and the way to the rebinding screens. Every change is heard at once; Escape saves` (rest unchanged).

- [ ] **Step 5: The template's title and help**

In `template/src/title.rs`, the Settings row's help becomes `.help("Speech, the controls, and the controller.")`.

Append to `template/help.md`:

```markdown

## Changing the controls

Settings, then Keyboard shortcuts, lists every control with its key. Press
Enter on one, then press the key you want for it. Controller buttons does the
same for a game controller.
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p hearken-app --test bindings --test screens > target/t4.txt 2>&1`, then `cargo test -p hello-hearken > target/t4-template.txt 2>&1`.
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/hearken-app template
git commit -m "feat(app): the rebinding screen

Settings gains Keyboard shortcuts, Controller buttons and a controller
switch. A capture is one press; a fixed or taken key is refused with the
reason, and each change is saved as it lands.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Text entry and the timed message

**Files:**
- Create: `crates/hearken-app/src/screens/text_entry.rs`, `src/screens/timed_message.rs`
- Modify: `crates/hearken-app/src/screens/mod.rs`
- Test: create `crates/hearken-app/tests/entry.rs`

**Interfaces:**
- Consumes: Task 1's `InputEvent::pressed`, M1's `State`, `spoken_char`, `end_sentence`.
- Produces:
  - `hearken_app::screens::TextEntryScreen<G>`: `new(heading: &str, label: &str, on_confirm: impl FnMut(&mut Ctx<G>, &str) + 'static)`, `max_len(usize) -> Self`, `with_text(&str) -> Self`, `text() -> String`, `cursor() -> usize`; `State::name` is `"text_entry"`. `text_entry::{MAX_LEN, INSTRUCTIONS, OnConfirm, is_printable}`.
  - `hearken_app::screens::TimedMessage<G>`: `new(title: &str, message: &str, status: &str, seconds: f64, on_complete: impl FnOnce(&mut Ctx<G>) + 'static)`, `complete_text(&str) -> Self`, `is_complete() -> bool`, `remaining() -> f64`; `State::name` is `"timed_message"`. `timed_message::OnComplete`.

- [ ] **Step 1: Write the failing tests**

Create `crates/hearken-app/tests/entry.rs`:

```rust
mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{Probe, TestData, TestGame};
use hearken_app::screens::text_entry::INSTRUCTIONS;
use hearken_app::screens::{TextEntryScreen, TimedMessage};
use hearken_app::{ControllerButton, Ctx, Headless, InputEvent, Key};
use hearken_speech::SpeechChannel;

/// A name field over a root screen. What Enter hands back lands in the
/// returned list.
fn field() -> (Headless<TestGame>, Rc<RefCell<Vec<String>>>) {
    let confirmed = Rc::new(RefCell::new(Vec::new()));
    let seen = confirmed.clone();
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(TextEntryScreen::<TestGame>::new(
        "New player",
        "Name",
        move |_ctx: &mut Ctx<TestGame>, text: &str| seen.borrow_mut().push(text.to_string()),
    ));
    h.frame(0.0);
    (h, confirmed)
}

fn type_text(h: &mut Headless<TestGame>, text: &str) {
    for ch in text.chars() {
        h.send(InputEvent::typed(ch));
    }
}

#[test]
fn the_field_says_what_it_is_and_how_it_works() {
    let (mut h, _) = field();
    assert_eq!(
        h.spoken(),
        [format!("New player. Name. {INSTRUCTIONS}")]
    );
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), [INSTRUCTIONS]);
}

#[test]
fn typed_characters_are_echoed_including_space_and_capitals() {
    let (mut h, _) = field();
    h.clear();
    type_text(&mut h, "Jo Ann");
    assert_eq!(h.spoken(), ["cap j", "o", "space", "cap a", "n", "n"]);
}

#[test]
fn left_reviews_back_to_front_and_right_forward_without_changing_the_text() {
    let (mut h, confirmed) = field();
    type_text(&mut h, "Cary");
    h.clear();
    for _ in 0..4 {
        h.press(Key::Left);
    }
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["y", "r", "a", "cap c", "cap c"]);
    h.press(Key::Return);
    assert_eq!(*confirmed.borrow(), ["Cary"]);
}

#[test]
fn home_end_and_up_read_the_ends_and_the_whole_text() {
    let (mut h, _) = field();
    h.clear();
    h.press(Key::Up);
    type_text(&mut h, "Bo");
    h.press(Key::Home);
    h.press(Key::End);
    h.press(Key::Down);
    assert_eq!(
        h.spoken(),
        ["Empty.", "cap b", "o", "Start. cap b", "End. o", "Bo"]
    );
}

#[test]
fn typing_and_backspace_act_where_the_cursor_is() {
    let (mut h, confirmed) = field();
    type_text(&mut h, "Cay");
    h.press(Key::Left);
    type_text(&mut h, "r");
    h.press(Key::Home);
    h.press(Key::Right);
    h.press(Key::Right);
    h.clear();
    h.press(Key::Backspace);
    assert_eq!(h.spoken(), ["Deleted a. Cry"]);
    h.press(Key::Return);
    assert_eq!(*confirmed.borrow(), ["Cry"]);
}

#[test]
fn the_edges_are_quiet_control_characters_are_not_typed_and_a_full_field_says_so() {
    let (mut h, _) = field();
    h.clear();
    h.press(Key::Backspace);
    h.press(Key::Left);
    h.press(Key::Home);
    h.send(InputEvent::key_text(Key::Tab, '\t'));
    assert!(h.spoken().is_empty(), "nothing to delete, nowhere to go, nothing typed");
    type_text(&mut h, &"x".repeat(24));
    h.clear();
    type_text(&mut h, "y");
    assert_eq!(h.spoken(), ["The field is full."]);
}

#[test]
fn a_controller_confirms_with_a_and_escape_cancels() {
    let (mut h, confirmed) = field();
    h.connect_pad();
    type_text(&mut h, "Al");
    h.send(InputEvent::button(ControllerButton::A));
    assert_eq!(*confirmed.borrow(), ["Al"]);
    assert_eq!(h.app.top_name(), "text_entry", "confirming leaves closing to the game");
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "root");
}

fn timed(seconds: f64) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(
        TimedMessage::<TestGame>::new(
            "Loading",
            "Loading the course.",
            "Still loading.",
            seconds,
            |ctx: &mut Ctx<TestGame>| ctx.replace(Probe::new("course")),
        )
        .complete_text("Ready."),
    );
    h.frame(0.0);
    h
}

#[test]
fn a_timed_message_answers_keys_with_its_status_and_moves_on_once() {
    let mut h = timed(1.0);
    assert_eq!(h.spoken(), ["Loading the course."]);
    h.clear();
    h.press(Key::Return);
    h.press(Key::F1);
    h.press(Key::Z);
    assert_eq!(
        h.spoken(),
        ["Still loading.", "Still loading."],
        "a stray key does nothing"
    );
    assert_eq!(h.app.top_name(), "timed_message");
    h.clear();
    h.frame(0.6);
    assert_eq!(h.app.top_lines()[3], "Ready in 0.4 seconds.");
    h.frame(0.6);
    assert_eq!(h.app.top_name(), "course");
    assert_eq!(
        h.speech.borrow().calls(SpeechChannel::Main),
        [("Ready.".to_string(), false)]
    );
}

#[test]
fn a_zero_second_message_moves_on_at_the_first_frame() {
    let h = timed(0.0);
    assert_eq!(h.app.top_name(), "course");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hearken-app --test entry > target/t5.txt 2>&1`
Expected: compile errors: no `TextEntryScreen`, `TimedMessage`, `text_entry` in `hearken_app::screens`.

- [ ] **Step 3: Text entry**

Create `crates/hearken-app/src/screens/text_entry.rs`:

```rust
//! Accessible text entry: characters are echoed as they are typed, and a
//! review cursor lets the player check what they typed without retyping
//! it.
//!
//! Left and Right step the cursor one character and speak the character
//! moved over, the way a screen reader reads caret movement in a text
//! field. Home and End jump to either end. Typing and Backspace act at the
//! cursor, so a player who arrows back to fix a letter edits in place. Up
//! and Down read the whole text, as they do in a one-line field under a
//! screen reader. Enter confirms; Escape cancels.
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `states/text_entry.rs`.
//! Freight Fate read the whole text on F2; F2 stops speech everywhere here.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::state::State;
use crate::text::{end_sentence, spoken_char};

/// The longest text a field takes unless the game says otherwise.
pub const MAX_LEN: usize = 24;

/// How the field works, spoken on arrival and on F1.
pub const INSTRUCTIONS: &str = "Type, then press Enter. Left and Right review what you have \
     typed, Home and End jump to the start or end, and Up reads it all. Escape cancels.";

/// What the screen does with the text on Enter. The screen stays open
/// unless this closes it, so a game can refuse a text and say why.
pub type OnConfirm<G> = Box<dyn FnMut(&mut Ctx<G>, &str)>;

pub struct TextEntryScreen<G: Game> {
    heading: String,
    label: String,
    /// The typed text, as characters, so the cursor counts characters.
    chars: Vec<char>,
    /// A gap from 0 to `chars.len()`, the way a text field's caret works.
    cursor: usize,
    max_len: usize,
    on_confirm: OnConfirm<G>,
}

impl<G: Game> TextEntryScreen<G> {
    pub fn new(
        heading: &str,
        label: &str,
        on_confirm: impl FnMut(&mut Ctx<G>, &str) + 'static,
    ) -> Self {
        TextEntryScreen {
            heading: heading.to_string(),
            label: label.to_string(),
            chars: Vec::new(),
            cursor: 0,
            max_len: MAX_LEN,
            on_confirm: Box::new(on_confirm),
        }
    }

    /// Take up to `max_len` characters.
    pub fn max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }

    /// Start with `text` in the field, the cursor at its end.
    pub fn with_text(mut self, text: &str) -> Self {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
        self
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn whole(&self) -> String {
        if self.chars.is_empty() {
            "Empty.".to_string()
        } else {
            self.text()
        }
    }

    fn insert(&mut self, ctx: &mut Ctx<G>, ch: char) {
        if self.chars.len() >= self.max_len {
            ctx.say("The field is full.");
            return;
        }
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
        ctx.say(spoken_char(ch));
    }

    fn backspace(&mut self, ctx: &mut Ctx<G>) {
        if self.cursor == 0 {
            return;
        }
        let removed = self.chars.remove(self.cursor - 1);
        self.cursor -= 1;
        let rest = self.whole();
        ctx.say(format!("Deleted {}. {rest}", spoken_char(removed)));
    }

    /// Step the cursor one character left or right and speak the character
    /// that sat between the old and new positions: the one moved over.
    fn move_cursor(&mut self, ctx: &mut Ctx<G>, delta: i64) {
        let next = self.cursor as i64 + delta;
        if next < 0 || next > self.chars.len() as i64 {
            return;
        }
        let next = next as usize;
        let ch = self.chars[self.cursor.min(next)];
        self.cursor = next;
        ctx.say(spoken_char(ch));
    }

    fn jump_cursor(&mut self, ctx: &mut Ctx<G>, target: usize, label: &str) {
        if self.cursor == target {
            return;
        }
        self.cursor = target;
        // A jump that moves the cursor means the field is not empty.
        let edge = if target == 0 {
            self.chars[0]
        } else {
            self.chars[self.chars.len() - 1]
        };
        ctx.say(format!("{label}. {}", spoken_char(edge)));
    }
}

/// Whether `ch` can be typed into a field: printable characters and the
/// space, not control characters or other whitespace.
pub fn is_printable(ch: char) -> bool {
    if ch == ' ' {
        return true;
    }
    !(ch.is_control() || ch.is_whitespace())
}

impl<G: Game> State<G> for TextEntryScreen<G> {
    fn name(&self) -> &'static str {
        "text_entry"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        let field = if self.chars.is_empty() {
            self.label.clone()
        } else {
            format!("{}: {}", self.label, self.text())
        };
        ctx.say(format!(
            "{} {} {INSTRUCTIONS}",
            end_sentence(&self.heading),
            end_sentence(&field)
        ));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        let Some(key) = event.pressed() else {
            return;
        };
        match key {
            Key::Escape => ctx.pop(),
            Key::Return | Key::KpEnter => {
                let text = self.text();
                (self.on_confirm)(ctx, &text);
            }
            Key::Backspace => self.backspace(ctx),
            Key::Left => self.move_cursor(ctx, -1),
            Key::Right => self.move_cursor(ctx, 1),
            Key::Home => self.jump_cursor(ctx, 0, "Start"),
            Key::End => {
                let end = self.chars.len();
                self.jump_cursor(ctx, end, "End");
            }
            Key::Up | Key::Down => {
                let whole = self.whole();
                ctx.say(whole);
            }
            _ => {
                if let Some((_, _, Some(ch))) = event.key_down() {
                    if is_printable(ch) {
                        self.insert(ctx, ch);
                    }
                }
            }
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        let before: String = self.chars[..self.cursor].iter().collect();
        let after: String = self.chars[self.cursor..].iter().collect();
        vec![
            self.heading.clone(),
            String::new(),
            format!("{}: {before}|{after}", self.label),
            INSTRUCTIONS.to_string(),
        ]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        INSTRUCTIONS.to_string()
    }
}
```

- [ ] **Step 4: The timed message**

Create `crates/hearken-app/src/screens/timed_message.rs`:

```rust
//! A brief spoken transition that counts down and moves on by itself. Any
//! key is answered with where it is instead of acted on, so a key still
//! held from the screen before does nothing.
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `TimedMessageState`. Its
//! arrival sound comes with audio in M2.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::state::State;

/// What a [`TimedMessage`] does when its countdown ends.
pub type OnComplete<G> = Box<dyn FnOnce(&mut Ctx<G>)>;

pub struct TimedMessage<G: Game> {
    title: String,
    message: String,
    status: String,
    remaining: f64,
    complete_text: String,
    on_complete: Option<OnComplete<G>>,
    complete: bool,
}

impl<G: Game> TimedMessage<G> {
    /// `message` is spoken on arrival and `status` answers a key press or
    /// F1. After `seconds` on top of the stack, `complete_text` (if any) is
    /// queued and `on_complete` runs, usually replacing this screen with
    /// the next.
    pub fn new(
        title: &str,
        message: &str,
        status: &str,
        seconds: f64,
        on_complete: impl FnOnce(&mut Ctx<G>) + 'static,
    ) -> Self {
        TimedMessage {
            title: title.to_string(),
            message: message.to_string(),
            status: status.to_string(),
            remaining: seconds.max(0.0),
            complete_text: String::new(),
            on_complete: Some(Box::new(on_complete)),
            complete: false,
        }
    }

    /// Spoken, queued, when the countdown ends.
    pub fn complete_text(mut self, text: &str) -> Self {
        self.complete_text = text.to_string();
        self
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn remaining(&self) -> f64 {
        self.remaining
    }
}

impl<G: Game> State<G> for TimedMessage<G> {
    fn name(&self) -> &'static str {
        "timed_message"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        ctx.say(self.message.clone());
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        if matches!(
            event.pressed(),
            Some(Key::Escape | Key::Return | Key::Space | Key::KpEnter)
        ) {
            ctx.say(self.status.clone());
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
        if self.complete {
            return;
        }
        self.remaining = (self.remaining - dt).max(0.0);
        if self.remaining > 0.0 {
            return;
        }
        self.complete = true;
        if !self.complete_text.is_empty() {
            ctx.say_queued(self.complete_text.clone());
        }
        if let Some(on_complete) = self.on_complete.take() {
            on_complete(ctx);
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        vec![
            self.title.clone(),
            String::new(),
            self.status.clone(),
            format!("Ready in {:.1} seconds.", self.remaining),
        ]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        self.status.clone()
    }
}
```

`crates/hearken-app/src/screens/mod.rs`: add `pub mod text_entry;` and `pub mod timed_message;` (alphabetical), and `pub use text_entry::TextEntryScreen;`, `pub use timed_message::TimedMessage;`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p hearken-app --test entry > target/t5.txt 2>&1`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/hearken-app
git commit -m "feat(app): text entry and the timed message

Both from Freight Fate c5d3ef8d. Text entry echoes each character and
reviews with the arrows; Up reads the whole text, since F2 stops speech
everywhere. The timed message answers any key with its status.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: The M1 follow-ups, and the full sweep

**Files:**
- Modify: `crates/hearken-app/src/logging.rs`, `src/run.rs`
- Modify: `crates/hearken-speech/build.rs`, `crates/hearken-app/build.rs`, `template/build.rs`
- Modify: `README.md`, `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`, `docs/superpowers/notes/m1-followups.md`
- Test: `crates/hearken-app/tests/platform.rs`

**Interfaces:**
- Consumes: M1's `logging::init`, `SingleInstanceGuard`, `Paths`.
- Produces:
  - `hearken_app::logging::rotate(paths: &Paths, env_prefix: &str)`; `logging::init` reads its level from `<PREFIX>_LOG` (default `info`), never `RUST_LOG`.
  - `hearken-speech` build metadata `link_args` (games read `DEP_HEARKEN_SPEECH_LINK_ARGS`, `;`-separated, empty off MSVC), replacing `delay_load_dlls`.

- [ ] **Step 1: Write the failing tests**

In `crates/hearken-app/tests/platform.rs`, make the first line of `the_log_file_goes_to_the_data_folder_and_a_panic_is_logged_once`:

```rust
    // A player's RUST_LOG, set for some other tool, must not cost them the
    // transcript.
    std::env::set_var("RUST_LOG", "off");
```

and add:

```rust
#[test]
fn starting_a_session_keeps_the_last_log_as_prev() {
    let dir = tempfile::tempdir().unwrap();
    let paths = hearken_app::Paths::at(dir.path());
    hearken_app::logging::rotate(&paths, "HEARKEN_TEST_ROTATE"); // nothing to keep yet
    std::fs::write(paths.log_file(), "first").unwrap();
    hearken_app::logging::rotate(&paths, "HEARKEN_TEST_ROTATE");
    std::fs::write(paths.log_file(), "second").unwrap();
    hearken_app::logging::rotate(&paths, "HEARKEN_TEST_ROTATE");
    assert!(!paths.log_file().exists());
    let prev = std::fs::read_to_string(dir.path().join("session.log.prev")).unwrap();
    assert_eq!(prev, "second", "the one before is replaced");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p hearken-app --test platform > target/t6.txt 2>&1`
Expected: a compile error (no `logging::rotate`); with the rotation test commented out, the logging test fails because `RUST_LOG=off` silences the file.

- [ ] **Step 3: Rotation and the level variable**

In `crates/hearken-app/src/logging.rs`, extend the module doc's first paragraph with: `` The level comes from `<env_prefix>_LOG` (env_logger's filter syntax, default `info`), never `RUST_LOG`, which a player may have set for some other tool and would otherwise lose them the transcript. ``

Add, above `pub fn init`:

```rust
/// `<env_prefix>_LOG_FILE`, else the data folder's `session.log`.
fn log_path(paths: &Paths, env_prefix: &str) -> PathBuf {
    std::env::var_os(format!("{env_prefix}_LOG_FILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.log_file())
}

/// Keep the last session's log beside this one as `<name>.prev`, replacing
/// the one before, so the log never grows without end. Call it only while
/// holding the single-instance lock: a second copy must never move the
/// running copy's log away.
pub fn rotate(paths: &Paths, env_prefix: &str) {
    let log = log_path(paths, env_prefix);
    if !log.exists() {
        return;
    }
    let mut prev = log.clone().into_os_string();
    prev.push(".prev");
    let prev = PathBuf::from(prev);
    if let Err(e) = std::fs::rename(&log, &prev) {
        // Logging is not up yet; stderr is all there is.
        eprintln!("could not keep the last log as {}: {e}", prev.display());
    }
}
```

In `init`, replace the `let log_path = std::env::var_os(...)...;` statement with `let log_path = log_path(paths, env_prefix);`, and replace both `env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))` expressions so that part reads:

```rust
    let level_var = format!("{env_prefix}_LOG");
    let env = || env_logger::Env::new().filter_or(level_var.as_str(), "info");
    let stderr_logger = env_logger::Builder::from_env(env()).build();
    match file {
        Some(f) => {
            let file_logger = env_logger::Builder::from_env(env())
                .target(env_logger::Target::Pipe(Box::new(f)))
                .build();
```

(the rest of the `match` is unchanged).

In `crates/hearken-app/src/game.rs`, add `` `<PREFIX>_LOG`, `` to the list of overrides in the `ENV_PREFIX` doc comment.

In `crates/hearken-app/src/run.rs`, the head of `run` becomes:

```rust
pub fn run<G: Game>(game: G::Data, first: impl FnOnce() -> Box<dyn State<G>>) -> i32 {
    let paths = Paths::discover(G::DATA_DIR, G::ENV_PREFIX);
    let mut guard = SingleInstanceGuard::new(&mutex_name(G::DATA_DIR));
    let first_copy = guard.acquire();
    // Only the copy holding the lock may move the last session's log aside.
    if first_copy {
        crate::logging::rotate(&paths, G::ENV_PREFIX);
    }
    crate::logging::init(&paths, G::ENV_PREFIX);
    if !first_copy {
        let line = already_running_line::<G>();
        log::warn!("{line}");
        return say_and_exit::<G>(&line, 0);
    }
```

(replacing everything from `let paths` down to the closing `}` of the old `if !guard.acquire() { ... }`).

- [ ] **Step 4: One Prism link recipe**

`crates/hearken-speech/build.rs`: replace everything from the top of the file down to (not including) the doc comment of `link_prism_system_libraries` with:

```rust
//! Work out the link arguments Prism needs, pass them on to games, apply
//! them to this crate's own binaries, and link Prism's system libraries on
//! macOS and Linux.
//!
//! On MSVC Prism's Windows screen-reader client DLLs must be delay-loaded,
//! or a binary will not start on a machine missing one of them, and Prism's
//! delay-load failure hook must be forced into the link, or a missing DLL
//! crashes the game instead of Prism reporting that backend unavailable.
//! A link argument only takes effect on the final link, which a library
//! cannot do for a game, so this crate publishes the arguments as
//! `DEP_HEARKEN_SPEECH_LINK_ARGS` (Cargo hands a `links` crate's metadata
//! to its direct dependents only) and each game's build.rs passes them on.
//! This crate's own tests, benches and examples ARE a final link, so the
//! same arguments are applied directly too; `rustc-link-arg` from a build
//! script never reaches a dependent's binaries.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_PRISMER_DELAY_LOAD_DLLS");
    let args = prism_link_args(&env::var("DEP_PRISMER_DELAY_LOAD_DLLS").unwrap_or_default());
    println!("cargo:link_args={}", args.join(";"));
    for arg in &args {
        println!("cargo:rustc-link-arg={arg}");
    }
    link_prism_system_libraries();
}

/// What a binary linking Prism needs on MSVC: every screen-reader DLL
/// delay-loaded, and Prism's delay-load failure hook kept.
///
/// The hook (prism-sys's source/delayimp.cpp) substitutes stub functions
/// when a screen-reader client DLL is missing, instead of MSVC's default of
/// raising an unhandled SEH exception (0xC06D007E) the first time a
/// delay-loaded call fails. Nothing else references that translation
/// unit's symbols, so the static-archive linker drops it unless forced in
/// -- the same problem Prism's own PrismBackends.cmake solves for backend
/// registrars with `/include:`. Nothing is needed off MSVC.
fn prism_link_args(dlls: &str) -> Vec<String> {
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return Vec::new();
    }
    let mut args: Vec<String> = dlls
        .split(';')
        .filter(|dll| !dll.is_empty())
        .map(|dll| format!("/DELAYLOAD:{dll}"))
        .collect();
    args.push("/INCLUDE:__pfnDliFailureHook2".to_string());
    args
}
```

(`link_prism_system_libraries` and `link_libstdcxx_statically` below stay as they are.)

`template/build.rs` becomes:

```rust
//! Link Prism the way it needs, or the game will not start (or will crash)
//! on a machine missing one of the screen readers' client DLLs.
//! hearken-speech works out the arguments; a link argument only reaches the
//! final link, which is this crate's, so every game passes them on here.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_HEARKEN_SPEECH_LINK_ARGS");
    match std::env::var("DEP_HEARKEN_SPEECH_LINK_ARGS") {
        Ok(args) => {
            for arg in args.split(';').filter(|arg| !arg.is_empty()) {
                println!("cargo:rustc-link-arg={arg}");
            }
        }
        // Cargo hands a build script this list only from a direct dependency.
        Err(_) => println!(
            "cargo:warning=Prism's link arguments are missing, so the game will not start on \
             a machine missing a screen reader's DLL. Fix: add hearken-speech to this \
             package's [dependencies]."
        ),
    }
}
```

`crates/hearken-app/build.rs`: replace `delay_load_prism_backends` with

```rust
/// Prism's link arguments, worked out by hearken-speech, for this crate's
/// own test binaries.
fn link_prism() {
    println!("cargo:rerun-if-env-changed=DEP_HEARKEN_SPEECH_LINK_ARGS");
    let args = std::env::var("DEP_HEARKEN_SPEECH_LINK_ARGS").unwrap_or_default();
    for arg in args.split(';').filter(|arg| !arg.is_empty()) {
        println!("cargo:rustc-link-arg={arg}");
    }
}
```

call `link_prism();` from `main` in its place, and change the module doc's second paragraph to: `Prism is linked statically through hearken-speech, which works out the link arguments it needs (see its build.rs); link arguments only apply to the final link of the package that emits them, so this crate passes them on for its own test binaries, as every game does.`

`README.md`: replace the paragraph starting `Every game needs a \`build.rs\` like \`template/build.rs\`` with:

```markdown
Every game needs a `build.rs` like `template/build.rs`. It passes on the link
arguments hearken-speech works out for Prism: the screen reader DLLs
delay-loaded, so the game still starts on a machine that lacks some of them,
and Prism's delay-load failure hook kept, so a missing DLL is reported
unavailable instead of crashing the game. The game must list `hearken-speech`
as a direct dependency, not only through `hearken-app`: Cargo hands a build
script those arguments only from a direct dependency.
```

`docs/superpowers/specs/2026-09-26-hearken-engine-design.md`: replace the bullet

```markdown
- Prism's delay-load list passed on to games: `hearken-speech` has
  `links = "hearken_speech"` and re-publishes prismer's list, and each game's
  own `build.rs` (a dozen lines, shipped in the template) turns
  `DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS` into `/DELAYLOAD` link arguments.
  Linker flags only apply to the final link, which is the game's.
```

with

```markdown
- Prism's link arguments passed on to games: `hearken-speech` has
  `links = "hearken_speech"` and publishes what Prism needs on MSVC (its
  screen-reader DLLs as `/DELAYLOAD`, and `/INCLUDE:__pfnDliFailureHook2`)
  as `DEP_HEARKEN_SPEECH_LINK_ARGS`, and each game's own `build.rs` (a few
  lines, shipped in the template) passes them to its link. Linker flags only
  apply to the final link, which is the game's.
```

`docs/superpowers/notes/m1-followups.md` becomes:

```markdown
# M1 follow-ups

Open items recorded when M1 closed (2026-09-26), updated when M1b closed.
Each is small; pick them up in whichever milestone touches the code first.

## Carry into M2 (audio)

- **BASS path per game.** `hearken-bass-sys` reads only `HEARKEN_BASS_PATH`; the
  spec says a game may name its own. `hearken-audio` should pass
  `<PREFIX>_BASS_PATH`, and the one-element loop in `loader.rs` goes.
- **Clamp `channel_get_data_float`.** Cap the byte count at `0x0FFF_FFFC` so a
  buffer over 256 MB can never set `BASS_DATA_*` flag bits.
- **Test a playing callback stream dropped mid-pull.** The drop ordering in
  `safe/callback.rs` exists for BASS's mixer thread; only decode streams are
  tested today.
- **Earcons.** Freight Fate plays move, select and back sounds in menus, a tick
  per typed character and an error at a text field's edge, select and error
  sounds on the shortcuts screen, and a notify sound on a timed message.
  Hearken's are silent until `hearken-audio` lands; text-entry edges are silent
  for the same reason.

## Carry into M4 (playtest)

- **`<PREFIX>_NO_CONTROLLER`.** Freight Fate keeps pads out of agent sessions
  with an environment variable; add it with the agent server.

## Deliberately left

- A second unreadable settings file overwrites the previous `settings.json.bad`.
- No test forces a failed settings save on Escape (logged, not panicking).
- Practice's resume line is covered only by the owner's listening pass.
- Controller rumble and Freight Fate's controller mapping diagnostics (the
  missing-trigger warning, `--controller-diagnostics`) were not carried; add
  them when a game needs haptics or a pad misbehaves.
```

- [ ] **Step 5: Run the focused tests, then check the link**

Run: `cargo test -p hearken-app --test platform > target/t6.txt 2>&1`
Expected: all pass.

Run: `cargo rustc -p hello-hearken --bin hello-hearken -- --print link-args > target/link-args.txt 2>&1`, then `grep -c "DELAYLOAD" target/link-args.txt` and `grep -c "__pfnDliFailureHook2" target/link-args.txt`.
Expected: each at least 1: the template's final link carries Prism's arguments through the new recipe.

- [ ] **Step 6: The full sweep**

Run each separately; background any that runs over 30 seconds.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings > target/clippy.txt 2>&1
cargo test --workspace --locked > target/test-all.txt 2>&1
```

Expected: fmt clean, clippy clean, every test passes. Read `target/test-all.txt` for the `test result:` lines; fix anything that fails before committing.

- [ ] **Step 7: Commit**

```bash
git add crates template README.md docs
git commit -m "fix: log rotation and level, and one Prism link recipe

session.log moves to session.log.prev at start, only in the copy holding
the single-instance lock, and <PREFIX>_LOG sets the level instead of
RUST_LOG. hearken-speech now works out Prism's link arguments once and
games pass them on.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Self-review notes (for the plan author, resolved)

- Spec coverage for M1b: rebindable actions (Task 3) and the rebinding screen (Task 4); the controller manager with hot-plug and repeat (Task 2); held-key tracking (Task 1, polled through `Ctx::action_held` in Task 3); text entry and the timed message screen (Task 5). The two "Carry into M1b" follow-ups are Task 6. Controller input arrives as `InputEvent` with no separate hook, as the spec's State design says.
- FF's `ControllerAction` enum is replaced by `controller_key_map`: its seven actions were the same seven keys a menu already reads.
- Settings row counts change once, in Task 4, where all three new rows land.
- Type names are consistent across tasks: `HeldKeys`, `ControllerManager`, `InputDevice`, `KeyBindings<A>`, `Rebind<A>`, `SavedBindings`, `BindingDevice`, `BindingsScreen`, `TextEntryScreen`, `TimedMessage`, `Ctx::{held, controller, bindings, action_for, action_held, control_name, save_settings, save_bindings}`, `Headless::connect_pad`.
