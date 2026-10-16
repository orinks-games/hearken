# Hearken Engine Design

- Date: 2026-09-26; M2 revised 2026-09-27
- Status: M1 and M1b built from this; M2 approved 2026-09-27
- Repository: `Orinks/hearken` (local only until the owner says to publish)
- Sources: Freight Fate `c5d3ef8d` (2026-09-25), Lucky Break `a101970` (2026-09-23)

## Summary

Hearken is a set of Rust crates for audio-first, screen-reader-first games,
extracted from the two games that already share its design: Freight Fate and
Lucky Break Casino. It gives a new game a window and loop, a screen stack,
menus, key and controller bindings, settings and help screens, speech through
Prism, 3D audio through Kira, synthesized and recorded sound, optional online
services, and a headless test harness an AI assistant can play through.

It exists because the next two games (a watersports arcade game and a
football game) would otherwise copy that shell a third and fourth time. Lucky
Break's copy has already drifted from Freight Fate's.

It is a general toolkit for audio games in the spirit of NVGT, not a copy of
either game's shell. The two games are where its code comes from, not the
shape of its API: a new game should never have to look like Freight Fate.

## Goals

1. One shared implementation of everything that is not a game's own rules.
2. A `cargo generate` template that starts as a playable, accessible shell:
   title menu, settings, key rebinding, help, Learn Sounds, pause.
3. A sound pipeline where every sound works as pure synthesis (prototype
   tier) and improves with real recordings layered on top (shipping tier),
   from whatever sources the game's author chooses.
4. Live procedural sound (engines, water, wind, crowds) driven by game
   parameters, on the same recipes that bake offline sounds.
5. Headless, transcript-backed testing and the agent-server MCP for every
   game, not only Freight Fate.
6. Open source under MIT/Apache-2.0 once Locomotive runs on it and the
   template builds from a clean clone.
7. Every built-in is a default made from the same public pieces the engine
   uses, so a game can tune it, switch it off, or replace it with its own:
   the audio library, positioning, speech pacing and verbosity, earcons,
   and the built-in screens.

## Non-goals (v1)

- A scripting language. The audience is Rust developers and people building
  with AI coding assistants. Revisit when someone asks.
- A visual editor, a no-code workflow, or 3D graphics.
- Localization. Neither source game has it today.
- macOS and Linux support in CI. v1 targets Windows, like Lucky Break;
  a game that ships elsewhere follows Freight Fate's nightly recipes
  (SDL2 compiled in with `bundled` + `static-link`, never Homebrew's sdl2).
- Moving Freight Fate before Career 1.9 ships.
- Anything to do with where a game's sound files come from. The engine
  ships no ElevenLabs or other generator or download service, no Faust
  bridge, and no Python studio tool; the author makes files however they
  like and the engine takes files.
- A second built-in audio library. The backend is a public trait, so a game
  that does not want Kira brings its own.

## Decisions already made

| Decision | Made | Notes |
|---|---|---|
| Name is Hearken; crates `hearken-*` | 2026-09-26 | All names were free on crates.io |
| Audience: Rust developers and AI-assisted creators | 2026-09-26 | No scripting layer in v1 |
| Shipping sound is hybrid; pure synthesis must stand alone | 2026-09-26 | From an A/B spike, below |
| Sound sources are the game author's choice | 2026-09-26 | No assumed provider |
| Speech uses the `prismer` crate | 2026-09-26 | Lucky Break's in-tree `prism` crates are dropped |
| Freight Fate moves only after 1.9 ships | 2026-09-26 | |
| The Python `fate-engine` repo is archived, not reused | 2026-09-26 | It targets the pygame stack Freight Fate left on 2026-08-22 |
| A general toolkit in the spirit of NVGT, in Rust | 2026-09-27 | The source games supply code, not API shape |
| Every built-in is a replaceable default | 2026-09-27 | Goal 7 |
| No sound-source integrations; a layer is a file | 2026-09-27 | Drops ElevenLabs, the provider trait and `generate` layers |
| No Faust bridge or Python studio tool | 2026-09-27 | Recipes are Rust on fundsp |
| The audio backend is a public trait; the built-in one is a default feature | 2026-09-27 | A game may bring its own library, or leave the built-in one out |
| The engine does positioning; a backend is handed a mix | 2026-09-27 | Same sound on every library, tested without a device |
| The audio API is handles and a pool | 2026-09-27 | Like NVGT's `sound_pool` |
| Positions use NVGT's axes | 2026-09-27 | x east, y north, z up; facing in degrees clockwise from north |
| M2 splits into M2 (runtime) and M2b (pipeline) | 2026-09-27 | |
| Kira replaces BASS as the built-in backend; `hearken-bass-sys` is dropped | 2026-10-01 | Compiled in: no DLL to ship, no commercial licence. A game that needs BASS brings it behind the trait |
| Opus decodes through libopus, compiled in | 2026-10-01 | Symphonia has no Opus decoder yet, and Freight Fate's music and most of its sounds are Opus. libopus is BSD-3: its notice ships with the game |
| A `hearken` facade crate is the core a game depends on; extras are its optional features | 2026-10-01 | See `2026-10-01-hearken-radio-design.md` |
| Internet radio is `hearken-radio`, behind the facade's `radio` feature, not BASS | 2026-10-01 | The point of Kira was no BASS. HE-AAC via `fdk-aac` is opt-in |
| The speech ladder is generic and comes in M2 | 2026-09-27 | Game-declared categories and levels |
| Freight Fate: Locomotive is the first game on Hearken; Lucky Break waits until Locomotive ships | 2026-10-16 | Locomotive is ported from Python, and Hearken is fixed wherever the port finds it short. M3 is now Locomotive |
| A looping clip can repeat a region inside its file, and a release plays out the rest | 2026-10-16 | Loop points from a WAV's `smpl` chunk, `LOOPSTART` tags, or the game; one horn file holds its attack, sustain and release |

### Evidence behind the sound decision

A throwaway spike re-made three Lucky Break sounds and one jet ski loop and
measured each against CC0 recordings (mean third-octave band deviation):

| Sound | Current | New |
|---|---|---|
| Chip click (Faust modal model, modes measured from a recording) | 15.6 dB | 4.4 dB |
| Crowd murmur (fundsp babble: moving formants, consonants, room) | 11.0 dB | 4.2 dB |
| Jet ski ride (fundsp, throttle-driven) | none | 5.7 dB |
| Win jingle (Surge XT through pedalboard) | not measured | not measured |

The owner's verdict after a loudness-matched listen: recording plus synthesis
is best for a shipped game, and pure synthesis must still be enough to
prototype with. All four toolchains (fundsp 0.23, Faust 2.88 to Rust,
pedalboard 0.9, Surge XT 1.3.4) worked on the owner's machine, and rendering
every clip took 0.2 seconds. The spike's sources are kept in
`docs/superpowers/notes/m2-spike/`; M2b ports its recipes to plain fundsp.

## Crates

```
hearken/
  crates/
    hearken            the facade a game depends on; optional extras as features
    hearken-speech     prismer, speech worker, two channels, capture, fakes
    hearken-audio      handles and pool, positioning, backends, live voices
    hearken-synth      fundsp recipes, category defaults, layer helpers
    hearken-app        shell, loop, screen stack, menus, input, settings
    hearken-radio      internet radio: HTTP/ICY, playlists, live HLS (optional)
    hearken-online     updater, sign-in, keyring, cloud saves, presence
    hearken-playtest   headless harness, scenario runner, agent-server MCP
  tools/
    hearken-bake       sound manifest -> sound pack, credits, --check
  template/            cargo-generate starter game
```

Dependency direction is strictly downward: `hearken-app` depends on speech
and audio; audio depends on synth and Kira; nothing depends on
`hearken-app` except games, playtest and the template. `hearken-online` is
optional and depends only on `hearken-app`'s service traits.

### hearken-speech

Freight Fate's speech stack, which is the more complete of the two:

- `prismer` (crates.io, `static` feature) on its own worker thread, so speech
  never blocks the loop.
- Two channels, never interchangeable. `say` is the menu and screen-reader
  channel. `say_event` is the gameplay channel: priority ladder, anti-backlog
  pacer, audio duck, and it can be silenced or cut. Freight Fate's
  `SpeechSink` (`say`, `say_event`, `stop_main`, `stop_event`, `stop`,
  `poll`) is the interface.
- A capture sink that records a transcript for tests, and fakes.
- Both channels log to one transcript log target.
- Prism's link arguments passed on to games: `hearken-speech` has
  `links = "hearken_speech"` and publishes what Prism needs on MSVC (its
  screen-reader DLLs as `/DELAYLOAD`, and `/INCLUDE:__pfnDliFailureHook2`)
  as `DEP_HEARKEN_SPEECH_LINK_ARGS`, and each game's own `build.rs` (a few
  lines, shipped in the template) passes them to its link. Linker flags only
  apply to the final link, which is the game's.
- Toolchain: prismer compiles Prism from source (CMake 3.24+, a C++23
  compiler) unless `PRISM_LIB_DIR` points at a prebuilt library. The template
  README documents both.

Gameplay speech gets three pieces in M2. Until then `say_event` goes
straight to the event voice.

- **Pacer**, Freight Fate's `EventSpeechPacer` ported whole into
  `hearken-speech` (it is plain logic, no audio):
  - It drops a line repeated within 2.5 s.
  - A keyed standing condition speaks again only when its text changes;
    `ctx.clear_condition(key)` resets it.
  - Queued lines that would start too late for their priority are flushed.
  - An important line an interrupt cut early is put back behind it; one the
    player had mostly heard is dropped.
  - Priorities are Ambient (waits up to 3 s), Important (0.8 s; Freight
    Fate's "Route") and Critical (always interrupts).
  - Thresholds are a config struct with Freight Fate's values as defaults.
    The pacer pauses with the game.
- **Ladder**, generic:
  - A game declares its event categories as `Game::Category`, the pattern
    `Game::Action` set; `NoCategory` for none, and Settings then has no
    verbosity row.
  - The game names its verbosity levels (for example Full, Quiet, Urgent
    only), and for each level and category gives one rule:
    - **Full** speaks the line.
    - **Terse** speaks the short line the caller gave, or the line itself.
    - **First time** speaks until `ctx.reset_first_times()`, which a game
      calls at a new level or leg.
    - **Earcon** plays a sound key instead.
    - **Silent.**
  - The table is plain data in `hearken-speech`, checked at start-up. A
    category missing from a level is logged once and treated as Full.
  - Freight Fate's "Transitions" rule is not carried; its notion of a
    condition worsening belongs to the driving game, and keyed conditions
    cover most of it.
- **Duck**, a Settings toggle, off by default as in Freight Fate:
  - While event speech is talking, and for 0.25 s after a ladder earcon,
    the game's chosen buses (music and ambience unless it says otherwise)
    drop to 50%.
  - It lives in `Ctx`, the one place that joins speech to audio.

The call is `ctx.say_event_with(text, SayEvent::new().priority(..)
.category(..).key(..).terse(..))`; `ctx.say_event(text)` still works. All
three pieces are optional: the pacer can be switched off or reconfigured,
the duck's buses and level are the game's, the raw `ctx.speech` sink stays
public, and a game can build its own delivery from the public pacer and
ladder types.

### hearken-audio

Handles and a pool, like NVGT's `sound_pool`, with Freight Fate's safety.

- **What a game calls**, `ctx.audio`, one concrete type:
  - `play(key, bus)`, `play_at(key, pos, bus)`,
    `start_loop(key, Option<pos>, bus)` and
    `start_voice(recipe, params, Option<pos>, bus)` each return a `Sound`
    handle (`Copy`). Ignoring it is fire-and-forget.
  - On a handle: `stop(h, fade_ms)`, `pause`, `resume`, `set_volume`,
    `set_pitch`, `set_position`, `set_voice_param(h, "throttle", 0.8)`,
    `is_playing`. A handle whose sound has ended does nothing.
  - Slots: `set_bed(Option<key>)` crossfades; `play_music(Option<key>,
    looping)` does nothing when asked for what is already playing.
  - `set_listener(pos, facing)`, `set_bus_volume`, `pause_all` and
    `resume_all`, `missing_clips()`.
- **Positions** are a `Vec3` in the game's own units on NVGT's axes: x east,
  y north, z up; facing is degrees clockwise from north.
- **Buses**: master, UI, effects, ambience, music and voice are built in; a
  game adds its own by name (an `engine` bus, say). Volumes are saved by
  name. Nothing game-specific in the API (Freight Fate's `engine_volume`
  and `weather_volume` stay in Freight Fate).
- **The engine does the positioning**, not the backend. From the listener
  and a sound's position it computes distance attenuation, pan, and a cue
  for a sound behind the listener (a gentle low-pass and a small drop).
  Occlusion comes from a game hook, `Game::occluded(data, listener, pos) ->
  bool`, and adds a low-pass and -6 dB, as in Lucky Break. The model's
  numbers (distances, rolloff, the behind cue, the occlusion drop) are a
  public config. This is what makes a backend small, keeps the sound the
  same on any library, and lets the maths be tested without a device.
- **The backend** is a public `Send` trait: `play(key, source, looping, mix)
  -> Channel`, `set_mix(channel, mix, ramp_ms)`, `stop(channel, fade_ms)`,
  `pause(channel, bool)`, `is_playing(channel)`.
  - A source is a clip's file bytes, which the backend decodes, or a live
    voice's renderer, which fills f32 buffers.
  - A mix is gain, pan, pitch, an optional low-pass, and the position
    relative to the listener for a backend that does its own 3D (HRTF).
  - A backend never sees handles, buses or fades.
  - M2 shipped BASS; since 2026-10-01 the built-in backend is Kira (the
    default `kira` Cargo feature), beside a silent backend and a recording
    fake for tests. Kira's mixer runs one sub-track per sound, carrying its
    gain, pan and low-pass; a clip over 256 KiB streams instead of decoding
    whole, so starting music never stalls the loop. Clips decode through
    Symphonia plus libopus for Opus, outside Kira, whose own loaders take
    no extra codecs.
- **Choosing**: the game's own backend if it supplies one, else Kira when
  the feature is on, else silent. The backend opens off the loop, as Freight
  Fate's does: the game runs silent for its first frames, then the real
  backend is swapped in and the music and volumes set meanwhile are
  replayed. No device means silence and one spoken notice.
- **Clips** come from a loose sounds folder behind a clip-source trait, so
  M2b's pack slots in. A clip that is missing or will not decode plays its
  category default from `hearken-synth` and is listed by `missing_clips()`.
- **Live voices** run a `hearken-synth` recipe on a callback stream
  positioned like any sound. `set_voice_param` updates a named parameter
  lock-free (fundsp shared values). Under the silent backend a voice is not
  rendered. This is what drives a jet ski, a boat, wind, or a football
  crowd that swells with the play.

### hearken-synth

- fundsp for graphs, band-limited oscillators, filters, reverb and noise,
  and no other external tool: no Faust, no Python.
- A recipe is a Rust function from named parameters to a fundsp node or a
  rendered buffer.
- **M2**: the category defaults (a tone for `ui/`, an impact for `sfx/`, a
  noise bed for `amb/`, a pad for `music/`, a blip for anything else), the
  UI earcon set (open, move, select, back, tick, error, notify), and the
  recipe type live voices run.
- **M2b**: the recipes that proved out in the spike, ported to plain fundsp:
  modal impacts (chips, glasses, bells; the spike's Faust chip is a bank of
  resonators), crowd babble, a pulse-train engine through fixed resonances
  (the firing rate moves; resonances never do, and loops are never
  pitch-shifted), water and spray.
- Layer helpers: re-cutting a recording into fresh grains, pitch and level
  variation, so a layered sound never repeats exactly.
- A measurement module (third-octave band balance, centroid, crest, decay)
  used by `hearken-bake --check`.

### hearken-app

The shell and everything a menu-driven audio game needs:

- **Shell**: Lucky Break's `Shell` trait (`poll`, `render`, `frame`) with an
  SDL implementation and a headless one. Freight Fate's shell is concrete; it
  gains the trait when it moves.
- **Screens**: one `State` trait, generic over the game.
  ```rust
  pub trait Game: 'static {
      type Event;          // what the game's rules emit (Lucky Break: protocol::Event)
      type Data;           // the game's own services and session
  }
  pub trait State<G: Game> {
      fn name(&self) -> &'static str;
      fn enter(&mut self, ctx: &mut Ctx<G>) {}
      fn exit(&mut self, ctx: &mut Ctx<G>) {}
      fn resume(&mut self, ctx: &mut Ctx<G>) {}
      fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {}
      fn on_event(&mut self, ctx: &mut Ctx<G>, event: &G::Event) {}
      fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {}
      fn lines(&self, ctx: &Ctx<G>) -> Vec<String> { Vec::new() }
      fn help(&self) -> String;
  }
  ```
  This is Lucky Break's trait with the event type lifted out. Controller
  input arrives as `InputEvent` like keys do, replacing Freight Fate's
  separate `handle_controller`. Presence hooks move to `hearken-online`.
- **Stack**: Lucky Break's queued `StackOp { Push, Pop, Replace, Reset }`,
  applied between frames, so a state never re-enters itself mid-handler.
- **Context**: `Ctx<G>` holds speech, audio, settings, bindings, the stack
  queue, logging, and `game: G::Data`.
- **Menus**: Lucky Break's `Menu`, `MenuCore`, `MenuItem` and `Label`, which
  are Freight Fate's design with the trucking hooks already removed. Every
  `Menu` is a `State` through a blanket impl, so no macro is needed. F1 reads
  the focused row's help, or the menu's own help when the row has none.
  Freight Fate's text-entry state comes with input in M1b.
- **Input**: Freight Fate's key bindings (with names for rebinding) and
  controller support, plus held-key tracking.
- **Built-in screens**: settings (volumes per bus, speech, verbosity, key
  rebinding), help (F1 on every screen reads that screen's help), Learn
  Sounds (built from the sound manifest's descriptions), pause, a timed
  message screen, and a save notice. Each is an ordinary screen a game can
  swap for its own, and Settings lets a game hide built-in rows and add
  its own.
- **Sound in the shell** (M2):
  - Menus, text entry, the shortcuts screen and timed messages play
    Freight Fate's keys (`ui/menu_open`, `ui/menu_move`, `ui/menu_select`,
    `ui/menu_back`, `ui/tick`, `ui/error`, `ui/notify`) through a `UiSounds`
    table on `Ctx`. A game points any entry at its own key or at nothing,
    and a menu row can override its select sound.
  - Settings gains a volume row per bus (5% steps, applied live), a
    verbosity row when the game has categories, and "duck audio for
    speech", saved as `audio.volumes` by bus name, `audio.duck_for_speech`
    and `speech.verbosity`.
  - Pause pauses all sound and the speech pacer, and resumes both.
  - A silent start (no device) is said once through the notices.
  - `audio.shutdown()` is explicit and bounded; it runs after the game's
    `on_exit` and before speech shuts down.
- **Services**: single-instance guard, file logging with a transcript
  target, a data directory named after the game (`<GAME>_DATA_DIR`
  overrides it).

### hearken-online (optional feature)

Freight Fate's online services, generalized: updater with download and apply
(Lucky Break's only checks for a release), device-code sign-in, keyring
token storage, cloud saves, presence heartbeat, Discord presence. The server
address and product identifiers come from the game; nothing points at
orinks.net by default. Everything runs off the loop and talks to it through
bounded channels, with an explicit, idempotent, time-bounded shutdown.

### hearken-playtest

Freight Fate's playtest library and agent server, generalized:

- A headless harness: dummy SDL drivers, capture speech, silent audio, a
  throwaway data directory, so tests never touch a player's settings or
  saves.
- A scenario runner a game registers its own scenarios with (Freight Fate's
  adversarial battery is one such game-side set).
- The agent-server MCP: keys in through the normal input path; both speech
  channels, earcons and cues out. Games add their own tools (Freight Fate's
  `pedal`, `cruise`, `scenario`) through a registration hook. Behind a Cargo
  feature that player builds leave out.

### hearken-bake and the sound manifest

Every sound is one entry, keyed by the name the game plays:

```toml
[sound."sfx/chip_25"]
describe = "Two chips clacked onto felt"   # Learn Sounds reads this
recipe = "chips"                           # a recipe the game registers
params = { count = 2, spread = 0.10 }
seconds = 0.5
reference = "refs/chip_single.wav"         # optional; for --check

[[sound."sfx/chip_25".layer]]
file = "chips/clack_03.wav"                # relative to the layers folder
source = "Freesound 633997"
license = "CC0-1.0"
attribution = "reg7783"
gain_db = -6.0
vary = { pitch_semitones = 1.0 }
```

- **Tiers resolve per sound at bake time.** Layers present means a hybrid;
  none, or none on disk, means the recipe alone. Game code never knows which.
  A project with only recipes has a complete prototype sound set.
- **Sources are the author's business.** A layer is a file: their own
  recordings, royalty-free packs, CC0 downloads, the output of whatever
  generator or synth they use. The engine never generates, fetches or
  talks to a service; a layer whose file is not on disk is left out and
  the sound bakes from its recipe, reported per sound.
- **Licensed layers stay out of the repo.** The layers folder can live
  outside the checkout (`[paths] layers = ...` or `HEARKEN_LAYERS_DIR`), the
  way Freight Fate's `sounds-licensed` overlay works today, so a public repo
  ships synthesis and a private build ships the hybrid.
- **Nothing refuses; everything falls back to synthesis.**
  - A sound the game plays with no manifest entry, or an entry with no
    recipe, gets a default synth sound chosen by its key's category
    (`ui/` a tone, `sfx/` an impact, `amb/` a noise bed, `music/` a pad,
    anything else a neutral blip), so a brand-new project is audible before
    anyone writes a recipe. The bake lists which sounds are on defaults.
  - A layer without `source` and `license` is left out of that sound, which
    bakes from its recipe instead, and the bake report names it. Unlabelled
    audio therefore never ships by accident, and the build still succeeds.
  - At run time, a clip missing from the pack plays its category default
    rather than silence, and `missing_clips()` still reports it.
- **Credits are generated.** `hearken-bake` writes the credits file from the
  layers' provenance, so attribution-required packs are credited without
  anyone doing it by hand.
- **Output**: a sound pack (one file of Ogg Vorbis and WAV entries plus an
  index carrying each sound's tier and provenance). During development,
  loose files override the pack.
- **`--check`** measures every recipe that names a reference and prints its
  band deviation. It informs; the owner's listening pass decides.

## Template

`cargo generate --git https://github.com/Orinks/hearken template` produces a
game that:

- opens a title menu with New Game, Settings, Help, Learn Sounds, Quit;
- has one playable screen that moves a sound around the listener in 3D and
  runs one live voice from a parameter;
- has a `sounds.toml` with five recipes and no layers, and plays category
  defaults for any sound it has not defined yet;
- has the `build.rs` that delay-loads the screen-reader DLLs;
- has one headless test that walks the menus and asserts the transcript;
- has CI on pull requests, `v*` tags and manual dispatch, Windows only;
- has a README for a first-time Rust developer covering the toolchain, the
  decoder's licence note, Prism's build requirements, and running the agent server
  so an assistant can play the game.

## Failure handling

Carried over from Freight Fate's engineering rules:

- Optional everything: no sound device, silent audio; no screen reader, SAPI through
  Prism; missing layers, synthesis; no network, online features say so once
  and the game plays on.
- Nothing unbounded on the loop: speech, audio device discovery, HTTP,
  cloud saves and Discord are off-loop with bounded channels and a defined
  policy for full queues.
- No network, IPC or thread joins in `Drop`; each worker has an explicit
  shutdown that signals, stops accepting work and waits within a measured
  bound, logging each step.
- `unwrap` and indexing only for real invariants; player data, devices and
  network responses are fallible and return contextual errors.

## Testing

- Each crate tests headless: speech through the capture sink, audio through
  the silent backend (Kira's backend tests run on Kira's mock output and
  listen to what it renders, so they need no device), recipes render and are measured, the manifest parser and tier
  resolution are property-tested.
- Positioning (pan, gain, the behind cue, occlusion) is plain unit tests;
  handles, buses, the duck and earcons are asserted through the recording
  backend. Freight Fate's pacer tests port with the pacer.
- `hearken-playtest` is itself tested by driving the template.
- Lucky Break's existing tests are the acceptance test for M3: they must
  pass unchanged on Hearken, apart from import paths.
- `cargo fmt --check`, Clippy with warnings denied on all targets, and the
  tests are separate CI gates. `actionlint` runs on every workflow change.

## Milestones

Each milestone gets its own implementation plan.

1. **M1 Skeleton.** Workspace, `hearken-bass-sys` (with callback streams),
   `hearken-speech`, `hearken-app` (shell, stack, menus, settings, help and
   pause screens), and a bare template that boots and passes a headless
   walkthrough.
   **M1b Input.** Rebindable actions and the rebinding screen, the
   controller manager, held-key tracking, text entry, the timed message
   screen.
2. **M2 Sound runtime.** `hearken-audio` (handles and pool, engine-side
   positioning and occlusion, the backend trait with BASS, silent and
   recording backends, off-loop open, beds, music, live voices, loose
   clips with category defaults); `hearken-synth`'s category defaults, UI
   earcons and recipe type; the speech pacer, generic ladder and duck;
   earcons, volume, verbosity and duck rows in `hearken-app`; the template's
   circling sound and live voice with its own recipe; the M1 follow-ups
   that touch audio. At the end a game makes sound with no sound files.
   **M2b Sound pipeline.** The `sounds.toml` manifest and `hearken-bake`
   (tiers, file layers with required provenance, generated credits, the
   pack, `--check`); the spike's recipes, layer helpers and measurement in
   `hearken-synth`; Learn Sounds.
3. **M3 Locomotive on Hearken.** Freight Fate: Locomotive is ported from
   Python to Rust on Hearken, its rules in an engine-free crate and its
   screens, cab and sound on Hearken; whatever the port finds missing is
   added here. Lucky Break follows once Locomotive ships: it drops its
   `prism`, `prism-sys`, `bass-sys` and shell code for Hearken. Its existing Python-rendered WAVs
   enter the manifest as layers, so nothing sounds different on day one;
   recipes then replace them one sound at a time, each with a listening pass.
4. **M4 Playtest and template.** Generic agent server; the template complete;
   the watersports game starts from it.
5. **M5 Public.** Licence files, README, a clean-clone CI run, crates.io
   publishing, then the repo goes public when the owner says.
6. **M6 Freight Fate**, after Career 1.9 ships, one crate at a time.

## Risks

- **Freight Fate keeps changing the code Hearken copies.** Speech, menus,
  bindings and controller code are extracted from `c5d3ef8d`. Until Freight
  Fate moves, a fix to any of those modules there gets ported to Hearken, so
  the move stays a swap. The extraction commit is recorded in each crate's
  docs.
- **First-time setup is heavy.** A C++23 compiler and CMake for Prism and libopus, SDL2
  vendored for Windows, ALSA headers on Linux. The template README covers
  all of it on one page.
- **Kira plays less than BASS did.** No internet radio yet, which Freight
  Fate uses; it comes as `hearken-radio`, the facade's `radio` feature,
  before Freight Fate moves (M6). No tracker music either, which Freight Fate supports but
  ships none of.
- **Decoder licences.** Symphonia is MPL-2.0: a game that ships it
  unchanged says where its source is; a game that changes its files
  publishes those files. libopus is BSD-3: a game ships its notice.
- **Source terms.** Each game's author is responsible for the terms of the
  sounds they use; the manifest's required provenance is what makes that
  checkable.
