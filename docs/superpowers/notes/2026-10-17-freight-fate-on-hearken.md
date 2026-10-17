# Freight Fate on Hearken (M6)

Date: 2026-10-17. Freight Fate branch `claude/project-thread-4c14dx`, Hearken
branch of the same name. The port ran in eight steps; this note is written at
step 7, when every engine service Freight Fate used has moved and its shell
stays its own.

The decision in one line: **Freight Fate takes Hearken's services and leaf
types, and keeps its own state stack, menus and speech delivery.** No Freight
Fate screen runs inside `hearken_app::{Ctx, App, State, Menu}`, and none of
Hearken's built-in screens are used.

## What Freight Fate adopted

Each is a re-export at the path Freight Fate's code and tests always used, so a
later Hearken bump lands in one module per area.

| Area | Hearken piece | Freight Fate path |
|---|---|---|
| Single instance | `single_instance::{SingleInstanceGuard, mutex_name}` | `single_instance` |
| Text | `text::{end_sentence, spoken_char}` (with Freight Fate's punctuation names) | `states::base` |
| Keys | `input::{Key, Mods}`, `sdl_shell::{key_from_keycode, mods_from}` | `states::base`, `app::sdl_shell` |
| Held keys | `held_keys` (with the JAWS bridge) | `app::held_keys` |
| Speech sinks | `speech::{backend, live, threaded, capture, fakes}`, `SpeechSink`, `ThreadedSpeech::shutdown_pumping` | `speech` |
| Event pacer | `speech::{EventSpeechPacer, EventPriority, PacerConfig, Protect, Valid, Cut, DeliveryStatus}`, run with `app::FF_PACER` | `speech` |
| Controller types | `input::{ControllerButton, ControllerAxis}` (right-stick buttons added) | `controller` |
| Bindings | `bindings::{Chord, PadChord, KeyBindings<A>, Rebind<A>}` and the name table, behind `impl Action for Action` | `bindings` |
| Audio | `audio` pool and Kira backend (one-shots, loops, engine bands, music, the duck), `probe_clip` | `audio` (`HearkenBackend` behind Freight Fate's `AudioBackend`) |
| Radio | `radio` streams (ICY, Ogg, HLS, `.pls`/`.m3u`), `StreamOptions.user_agent`, `Stream::raw_title` | `audio::hearken_radio` |

BASS, `bass-sys`, `prismer` as a direct dependency and Freight Fate's own sink
stack and pacer are gone from Freight Fate. The priority ladder
(`SpeechCategory`, `Disposition`, the rung table) stays in `ff_core`, because
it is game content and `ff_core` is headless.

What Hearken grew for it (all additive; Echo Parlor unchanged): the JAWS bridge
and punctuation names, an iOS `sdl2` dependency and an optional `testing`
feature; the speech fixes Freight Fate made after `c5d3ef8d` (silent refresh,
the ZDSR braille fix, `is_speaking`, the VoiceOver startup hold,
`shutdown_pumping`); pacer parity (Critical wait budget, wide-character weight,
`Protect`, delivery receipts, the `note_ahead` fix); right-stick buttons,
game-first reserved reasons, `Action::keeps_engine_reserved`, kept moves and
`KeyBindings::{reset, from_parts, moved_keys, moved_pad}`; game clips with no
built-in fallback, per-part ramps, pan and pan law, gain curve, seek, length,
AAC/MP4/AIFF decoding, `probe_clip`, `start_feed_at_volume`; the radio
User-Agent and raw now-playing metadata.

The guard: Freight Fate's `rust.yml` checks job fails if
`hearken::(Ctx|App|ctx|app|screens|run|menu|state|speech_delivery)` appears
anywhere under its `crates/`.

## What Freight Fate keeps, and why

Every item here would change what a player hears, or rewrite most of Freight
Fate's 117,000 test lines, if it moved onto Hearken's version.

- **The immediate state stack.** Freight Fate's `push_state`, `pop_state`,
  `replace_state` and `reset_to` run `enter()` inside the caller's handler; a
  call that targets the state currently in its own handler is deferred until
  that handler returns. Hearken queues every stack operation and applies the
  queue after the handler. Thirty sites in Freight Fate's screens pop or push
  and then speak in the same handler (for example, closing the settings menu
  and then saying "Settings saved." after the revealed menu has announced
  itself). Under a queued stack each of those lines would come out in the
  other order. `tests/it/app_state_stack.rs` pins Freight Fate's order.
- **Shared states and `DriveRef`.** The stack is `Vec<Rc<RefCell<dyn State>>>`
  with `AsAny` downcasts. The driving menus hold an `Rc` to the drive below
  them (`DriveRef`, about 110 uses), and several app-level hooks walk the stack
  (`top_paces_main_speech`, the radio-settings and covered-music walks).
  Hearken's private `Vec<Box<dyn State<G>>>` allows none of that.
- **Stack variants with no Hearken form.** `push_state_with(should_enter:
  false)`, `pop_state_with(should_exit: false, reentry)` and
  `reset_to_with(false, false)`. A revealed screen gets `enter()` again, so a
  menu replays its open sound; Hearken's `resume` does not.
- **Menus.** `MenuCore`/`Menu`/`impl_state_for_menu!` (89 menus) differ from
  Hearken's menu in ways a player hears: the index is kept and clamped on
  entry and refresh (Hearken resets to the first row); items stay in place
  while an action runs (Hearken lends them away); an inert row plays the
  select sound; F1 reads the row's help, or "{label}." when it has none; Left
  and Right do nothing on a plain menu; Control stops speech (Hearken uses F2,
  which Freight Fate uses for the driving command list and the text-field
  readout); the auto-repeat guard; first-letter jump on the typed character;
  a blank line after the title in `lines()`; row reads kept out of the
  message log; Freight Fate's intro help text; the menu position cue as its
  own translated line.
- **Speech delivery.** `app/speech_delivery.rs` (`GameContext::say_with`,
  `say_event_with`) stays Freight Fate's. It picks the event channel from the
  `sapi_events` setting rather than from the sink; `SayEvent::new()`
  interrupts by default; it writes `[event]`, `[pacer]` and `[ladder]` lines
  under the `freight_fate.transcript` target and never a second `[event]` for
  a requeue; it applies the ladder (gated on the tutorial being done, keyed
  per condition), renders the terse/normal pair and translates before the
  pacer, the message log and the transcript see the line; it turns interrupts
  into queued lines at the wheel; it ducks only for event lines; and
  `stop_speech` resets the pacer and stops both voices separately. Hearken's
  `Ctx::say`/`say_event_with` do none of this the same way.
- **The app loop and shell extras.** Close button and Alt+F4 ask first; the
  controller's Start is not Escape; D-pad repeats go only to screens that ask
  for them; "Controller connected" lines; Shift+F1 and Back+Start open input
  practice over any screen; message review gets first refusal on every key;
  the frame time is not capped; the per-frame order of the cloud, duty and
  JAWS announcements; touch gestures; the bounded, logged `App::shutdown`,
  which still quits speech through `shutdown_pumping` so the window keeps
  answering while the last line finishes.
- **Everything else around the shell.** The settings file and its paths,
  logging layout, the playtest harness, the agent server hooks, boot timing,
  `TimedMessageState`, l10n.

## Conditions for ever moving Freight Fate's screens onto `Ctx`

Not planned. If the owner wants it later, it is its own milestone, and Hearken
needs all of the following first.

Stack and state:
- An immediate stack mode with shared (`Rc<RefCell>`) states and deferred
  self-exit, plus stack iteration and downcast (`states()`, `as_any`).
- `push(should_enter)` and `pop(should_exit, reentry)`.
- Revealed menus re-enter with their open sound.
- Per-state hooks: `paces_main_speech`, `presence`, `copy_focus`,
  `captures_text_input`, `wants_controller_repeat`, `handle_gesture`,
  `on_controller_disconnect`, and ticks for covered screens.

App loop:
- The global F1 and F2 keys made optional, and Start-as-Escape made optional.
- A quit-confirmation hook for the close button.
- A message log with the review and copy keys.
- A configurable frame-time cap (Freight Fate uses none).
- Touch gestures; a settings file whose shape the game owns.

Menus:
- Keep the index on entry and clamp it; do not lend items during an action;
  the inert select sound; the blank line in `lines()`; the repeat guard; the
  label fallback for help.

Speech (the speech reader's H11-H14):
- **H11** A game hook at the main-channel boundary (`Game::say`, or
  `translate` plus `on_spoken`), so built-in screens speak through the game's
  delivery, translated and logged.
- **H12** A configurable transcript target (`Game::TRANSCRIPT_TARGET`,
  default `hearken.transcript`).
- **H13** `State::paces_main_speech()`.
- **H14** A documented inert `Delivery` for a game with its own: `pacer: None`
  and `Verbosity::NONE` must mean no duck and no earcon.

And one rule that holds whatever moves: **Freight Fate's delivery API must
live on a type Freight Fate owns, never as a trait on `Ctx`.** Rust resolves
the inherent `Ctx::say(impl Into<String>)` before a trait method of the same
name, so `ctx.say("...")` would compile and silently skip the ladder, the
translation, the message log and the transcript target.

## Open after M6

- HE-AAC stations play their AAC-LC core (muffled, mono for v2) because the
  `fdk-aac` feature stays off; the licence call is the owner's.
- Tracker modules (`.it`, `.xm`, `.s3m`, `.mod`, `.mo3`) no longer play;
  Freight Fate ships none.
- fMP4/CMAF HLS and `EXT-X-MEDIA` renditions are not read.
- Hearken verifies TLS certificates, so a station with a bad HTTPS
  certificate fails where BASS played it.
- Freight Fate's `Cargo.lock` must be re-pinned to a pushed Hearken commit.
