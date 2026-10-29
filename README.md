# Hearken

Hearken is a game engine for audio games: games you play by ear, built for
blind and visually impaired players first. It is written in Rust as a set
of crates, and a game made with it speaks through the player's screen
reader, plays positioned 3D sound, and works with the keyboard or a game
controller from the first menu on.

Hearken runs on Windows, Linux and macOS. It is young and still changing:
there is no release on crates.io yet, and its API can change between
commits.

## What players get

Every Hearken game has these, without the game writing any of it:

- **Your own screen reader.** Speech and braille go through Prism, which
  reaches NVDA, JAWS, VoiceOver, Speech Dispatcher, SAPI and more. Hearken
  picks the one that is actually running. Switch screen readers while you play, and the game
  follows within a few seconds instead of going silent. With no screen
  reader on Windows, SAPI voices speak instead.
- **The same keys everywhere.** Up and Down move through menus, Enter
  chooses and Escape goes back. F1 reads help for the screen you are on, and
  F2 stops speech.
- **Settings**: speech rate, how much the game says while you play (when
  the game offers levels for it), a volume for each kind of sound, lowering
  music and background sound while the game speaks, and whether menus say where you are ("2 of 4").
- **Controls you can move.** Any key or controller button the game lets
  you move can go to another one, from Settings. Each change is saved at
  once.
- **Game controllers**, through SDL2, with a spoken notice when one
  disconnects.
- **A help document** read a section at a time, which works before the game
  has saved anything.
- **A game that keeps running when sound or speech fails.** Without a sound
  device, the game says so and keeps running. A missing sound file plays a
  built-in stand-in. Speech runs on its own thread, so a screen reader that
  hangs loses a sentence instead of freezing the game.

## What developers get

A game depends on one crate, `hearken`, which brings in the rest:

| Crate | What it does |
|-------|--------------|
| `hearken` | The one dependency for a game. It re-exports everything below. |
| `hearken-app` | The window, the screen stack, spoken menus, and ready-made Settings, Help, Pause, Confirm, text-entry and key-binding screens. Includes a headless harness for tests. |
| `hearken-speech` | Screen reader, braille and text-to-speech output through Prism. Includes an event voice with pacing, priorities and repeat suppression, and a verbosity table the player sets. |
| `hearken-audio` | A pool of positioned sounds, buses and live voices, played through [Kira](https://github.com/tesselode/kira), which compiles into the game. |
| `hearken-synth` | Sound made with [fundsp](https://github.com/SamiPerttu/fundsp): built-in menu earcons, a default sound for every category, and recipes for live voices such as engines. A game can make sound with no sound files at all. |
| `hearken-pack` | A release's sounds in one encrypted, authenticated `sounds.pak`, so players cannot browse or replace them. Written by `hearken release`, read by `hearken-audio`. |
| `hearken-seal` | A game's records and saves, sealed in a release so a player cannot edit, swap or restore them (`ctx.vault`, `hearken::seal`). |
| `hearken-replay` | Verified online runs: a run recorded as its seed and inputs, and the game's rules built for WebAssembly so a server can replay it and trust only the result it computes (`hearken::replay`, `ctx.online` with the `online` feature). The server side is `server/`; see `docs/online.md`. |
| `hearken-bake` | Build scripts only: puts a release's keys into the program as scattered shares. |
| `hearken-discord` | Discord Rich Presence: the "Playing ..." status friends see, from the game's own Discord application. Off the game loop, throttled, and silent when Discord is not running (`ctx.discord`, with the `discord` feature). |
| `hearken-radio` | Internet radio, played as a sound: HTTP and ICY stations, playlists and live HLS, with now-playing titles. Optional. |

Some of what that gives a game:

- **Screens on a stack.** A game is a set of screens that implement
  `State`. Push one, pop back, and the screen underneath announces itself
  again. Closing a screen can also leave the one beneath silent
  (`ctx.pop_quiet()`), hand it a result to speak first
  (`ctx.pop_with("Route deleted.")`), or go back to a named screen
  (`ctx.pop_to("routes")`).
- **Spoken menus.** A `Menu` builds its rows; Hearken speaks them, plays the
  earcons and handles the keys. Rows carry their own help for F1.
- **Remappable actions.** A game lists its controls as an `Action` enum with
  default keys and controller buttons. `ctx.action_for(event)` and
  `ctx.action_held(action)` read input through the player's bindings, and
  `ctx.control_name(action)` says the key the player chose ("the Up arrow").
- **Gameplay speech that keeps up.** The event voice drops a line the
  player just heard, speaks a standing warning only when it changes, and
  lets an important line go ahead of chatter instead of waiting behind it.
  A game names its own event categories and what each one does at each
  verbosity level, so a quiet mode can turn chatter into earcons. Speech
  can also go to the braille display alone.
- **3D sound.** Sounds play at a position around a listener with a heading,
  on buses (UI, effects, ambience, music, voice) the player sets volumes
  for. A game can add buses, muffle sounds behind walls through
  `Game::occluded`, and put effects such as cab speakers on a bus.
- **Loops with a start and a release.** A looping clip can repeat a region
  inside its file and play out the rest when released, so a horn is one
  file. The loop points come from a WAV's `smpl` chunk, `LOOPSTART` and
  `LOOPEND` (or `LOOPLENGTH`) tags in Ogg, Opus or FLAC files, or
  `Audio::start_loop_region`.
- **Sound files that replace built-in sounds.** Sounds are named by key,
  such as `sfx/beacon`. Hearken looks for `sfx/beacon.ogg` (or `.opus`,
  `.wav`, `.flac`, `.mp3`) in a `sounds` folder next to the game, and plays
  the game's built-in sound when there is no file. Dropping in a recording
  needs no code change.
- **Unlockable soundtracks.** A game lists its music as `Track`s; the
  player unlocks each by hearing it in play (or when the game says so) and
  listens from `SoundtrackScreen`, which plays each track as it was heard
  in the game. A game can let players export a finished album to WAV, as
  heard or freshly varied; the stems never leave the sound pack. See
  [Soundtracks](#soundtracks).
- **Tests without a window.** `hearken::Headless` runs a game with no
  window, sound or screen reader. Press keys, then check what it said.
- **Saved settings and logs.** Settings, including the game's own, are
  saved as JSON in the platform's local data folder, next to a session log.
  The log can also carry a transcript of everything the game said.

## Getting started

### Build the template game

The repository includes Hello Hearken (`template/`), the smallest complete
game: a title menu, settings, help, and a practice room with a beacon that
circles you and an engine you can rev.

1. Install Rust with [rustup](https://rustup.rs). `rust-toolchain.toml`
   picks the compiler version.
2. Install CMake 3.24 or newer and a C++23 compiler (Visual Studio 2022 on
   Windows). Prism, SDL2 and libopus, the Opus decoder, are compiled from
   source the first time you build. To skip Prism's build, set
   `PRISM_LIB_DIR` to a folder holding a prebuilt Prism library.
3. On Linux, install the development files for ALSA (`libasound2-dev` on
   Debian and Ubuntu), Speech Dispatcher (`libspeechd-dev`), and SDL2's
   desktop backends. Prism will not build without at least one Linux speech
   backend. `.github/actions/linux-deps/action.yml` lists every package CI
   installs on Ubuntu.
4. Turn your screen reader on, then run:

   ```sh
   cargo run -p hello-hearken
   ```

### Make your own game

Copy `template/` to start, or set a game up by hand:

1. Add Hearken to your game's `Cargo.toml`:

   ```toml
   [dependencies]
   hearken = { git = "https://github.com/orinks-games/hearken", branch = "main" }
   ```

   `hearken` must be a direct dependency: the build script below can only
   read link arguments from a direct dependency.

2. Copy `template/build.rs` into your game and set `build = "build.rs"` in
   `[package]`. It passes on the link arguments that `hearken-speech` works
   out for Prism. There are none since Prism 0.1.4, but the build script
   means a future one reaches your game with no change to it.

3. Describe your game by implementing `hearken::Game`:

   ```rust
   pub struct MyGame;

   impl hearken::Game for MyGame {
       const TITLE: &'static str = "My Game";
       // The folder that holds settings and logs.
       const DATA_DIR: &'static str = "MyGame";
       // Names the environment overrides, such as MY_GAME_NO_SPEECH.
       const ENV_PREFIX: &'static str = "MY_GAME";
       type Event = ();
       type Data = ();
       type Settings = ();
       type Action = hearken::NoAction;
       type Category = hearken::NoCategory;
   }
   ```

   Fill in the associated types as the game grows: `Data` for your game's
   state, `Settings` for your own saved settings, `Action` for controls the
   player can move, `Event` and `Category` for gameplay events and their
   verbosity. Optional methods add your own sound setup, built-in sounds,
   occlusion and Settings rows.

4. Start it from `main`, with the first screen:

   ```rust
   fn main() {
       std::process::exit(hearken::run::<MyGame>((), || Box::new(Title::new())));
   }
   ```

5. Write screens. A screen implements `hearken::State`, and only `name`
   and `help` are required:

   ```rust
   use hearken::{Ctx, InputEvent, Key, State};

   struct Room;

   impl State<MyGame> for Room {
       fn name(&self) -> &'static str {
           "room"
       }

       fn enter(&mut self, ctx: &mut Ctx<MyGame>) {
           ctx.say("A quiet room. Space plays a sound.");
       }

       fn handle_input(&mut self, ctx: &mut Ctx<MyGame>, event: &InputEvent) {
           if event.pressed() == Some(Key::Space) {
               ctx.audio.play("sfx/beacon", hearken::Bus::EFFECTS);
           }
       }

       fn help(&self, _ctx: &Ctx<MyGame>) -> String {
           "Space plays a sound. Escape goes back.".into()
       }
   }
   ```

   For menus, implement `hearken::menu::Menu` instead, as `template/src/title.rs`
   does. `hearken::screens` has the ready-made Settings, Help, Pause and
   Confirm screens to push from them.

6. Test it without a window. With a title menu like the template's:

   ```rust
   use hearken::{Headless, Key};

   #[test]
   fn the_title_announces_itself() {
       let mut h = Headless::<MyGame>::new((), Title::new());
       assert_eq!(h.spoken(), ["My Game.", "Play. 1 of 4."]);
       h.press(Key::Return);
       assert_eq!(h.app.top_name(), "room");
   }
   ```

`template/src/` shows the rest in a few short files: remappable actions in
`lib.rs`, a menu in `title.rs`, positioned sound and a pause menu in
`practice.rs`, and a synthesized engine voice in `sound.rs`.
`template/tests/walkthrough.rs` plays through the game in tests.

### Environment overrides

Each game's `ENV_PREFIX` names a set of environment variables, so two games
on one machine never share them. For a prefix of `MY_GAME`:

| Variable | Effect |
|----------|--------|
| `MY_GAME_DATA_DIR` | Keep settings and logs in this folder instead. A `portable.txt` beside the executable keeps them in a `saves` folder there. A sealed release ignores it. |
| `MY_GAME_SOUNDS` | Load sound files from this folder. A packed release ignores it. |
| `MY_GAME_LOG`, `MY_GAME_LOG_FILE` | Set the log level and where the log goes. |
| `MY_GAME_NO_SPEECH` | Turn speech off, for test runs. |
| `MY_GAME_SPEECH_BACKEND` | Speak through this Prism backend only. |
| `MY_GAME_STREAM_TRANSCRIPT` | Print each spoken line to standard output. |
| `MY_GAME_AUDIO_BACKEND` | `silent` turns sound off. |

### Play and release with the hearken command

```sh
cargo install --git https://github.com/orinks-games/hearken hearken-cli
```

- `hearken play <game>` starts a game. With Rust installed it pulls the
  latest source and builds a developer build, agent server included.
  Without Rust, or with `--download`, it downloads the game's latest
  GitHub release instead.
- `hearken release <game>` builds the zip players get, in `dist/`: the
  executable, the game's files, and on Windows SDL2 and the Visual C++
  runtime. It leaves out an `agent-server` Cargo feature. The archive is
  named for the computer it was built on (`windows-x64.zip`,
  `macos-arm64.zip`, `macos.zip` for Intel Macs, `linux-x64.tar.gz`), so
  run it once on each system you ship for and upload every archive to the
  same GitHub release. `hearken play --download` picks the one for the
  player's computer, and on an Arm Windows PC or Mac takes the x64 build
  when there is no Arm one.
- `hearken seal-key <game>` shows where a sealing game's key is kept and
  its ID; `--print` shows the key itself, for a CI secret.

The command knows no games of its own: a game is its GitHub repository or
a folder on this computer.

```sh
hearken play orinks-games/freight-fate        # or https://github.com/orinks-games/freight-fate
hearken play orinks-games/freight-fate --download
hearken release ../my-game
```

A repository is played from the checkout you are standing in when it is
that repository's, or else from a folder of the same name in your home
folder, cloned the first time. A download goes under your local data
folder, one folder per release, and starts the release's only program, or
the one named like the repository. Inside a game's own folder the name can
be left out; `hearken play` then builds the work in progress as it is. The
game's `Cargo.toml` says which package is the game and what a release
holds:

```toml
[package.metadata.hearken]
portable = true                               # saves beside the game
pack = true                                   # sounds in an encrypted sounds.pak
seal = true                                   # data written through ctx.vault is sealed
seal-id = "k1:3f9a0c2e7b14d685"               # the seal key's ID, added after the first release
include = ["sounds", "README.md", "LICENSE"]  # the default
```

A portable release carries `portable.txt`, so the game keeps its settings,
saves and log in a `saves` folder beside it instead of the user's folder.

A packed release ships the `sounds` folder as one file, `sounds.pak`, so a
player who opens the zip cannot listen to the sounds or read their names,
and a game's secrets stay secret. Each release is encrypted and
authenticated (ChaCha20-Poly1305) with
a new random key, built into that release's program and kept nowhere else,
so no one tool opens every Hearken game, and a tool made for one release
does not open the next. The key is split into shares that are put back
together only when the pack is opened, so it is not sitting whole in the
program's bytes, and sounds stay encrypted in memory except while they
play. Any changed byte is caught: a damaged pack plays only the game's
built-in sounds and tells the player to reinstall, and a changed sound
plays nothing. A packed release reads its pack and nothing else, so a
loose `sounds` folder or `MY_GAME_SOUNDS` cannot replace its sounds. Files
in `sounds` that are not sounds, such as credits, still ship loose.
Developer builds and `hearken play` read the loose folder as before, and
`MY_GAME_SOUNDS` points them at any folder.

This stops browsing, not a determined person: the key is inside the program
players run, and someone willing to take that program apart can find it.
Any game that plays its own sounds on the player's computer has the same
limit.

### Sealed saves

A game that keeps high scores, unlocks or progress can stop players
editing them. It writes those files through `ctx.vault` instead of
`std::fs`, and sets `seal = true`:

```rust
use hearken::{Opened, Tamper};

match ctx.vault.read("progress.json") {
    Opened::Sealed(bytes) | Opened::Plain(bytes) => load(&bytes),
    Opened::Missing => start_fresh(),
    // A plain file from a version before sealing: import it, or don't.
    Opened::Legacy(bytes) => load(&bytes),
    Opened::Tampered(how) => {
        log::warn!("progress was changed outside the game: {how}");
        let _ = ctx.vault.set_aside("progress.json");
        start_fresh();
    }
    // A developer build with a release's saves, or a disk error: keep the
    // file and don't save over it.
    Opened::Locked | Opened::Failed(_) => start_fresh_without_saving(),
}
ctx.vault.write("progress.json", &bytes)?;
```

In a release, each file is encrypted and authenticated with its name bound
in, and the data folder holds the game's files and nothing else. A file
edited in any editor, copied over another of the game's files, or forged
reads as `Tampered`;
what the game does then is its own choice. Developer builds, tests and
`hearken play` read and write plain files, so you can look at them, and
never overwrite a release's sealed files. Settings stay plain: they are
the player's own. A sealed release ignores `MY_GAME_DATA_DIR`.

The key is the game's own, the same for every release, so updates keep
players' saves. The first `hearken release` makes it in your config
folder (`hearken/seal-keys/<package>.key`) and says where. **Back it up:**
if it is lost, players' saves stop loading in the next release. It also
prints a `seal-id = "k1:..."` line to commit; from then on a release with
no key, or another key, is refused. On CI, store the key
(`hearken seal-key --print` shows it) as a secret and pass it as
`HEARKEN_SEAL_SECRET`; keys are never made on CI. Never commit the key.

The limits are those of the sound pack, and a few more. The key is in the
program, so someone who takes it apart can forge any file. Each file
stands alone, so putting back an older copy of a file, deleting one, or
copying one from a friend's install looks like real play. Results that
must be trusted belong on a server that replays them. A game that sets
`IMPORTS_PLAIN_SAVES = true` on its `Game` gets `Legacy` for plain saves
an older, unsealed version left. The first time it seals, the vault
leaves a marker outside the folder listing the plain saves that were
there; from then on only those names can still be imported, and any
other plain file reads as `Tampered`. That marker is one small file,
named for the game: in the config folder for an installed copy, or beside the
executable for a portable one, so a portable copy leaves nothing in the
player's user folders. A player who deletes the marker too, or plays as a
new user, can still have a hand-written plain file imported; turn
`IMPORTS_PLAIN_SAVES` off once players have moved to sealed releases, and
no marker is written at all. Games that never import (the default) leave
no marker. This raises cheating from "open a text
editor" to "reverse engineer the program", which is as far as any offline
game can go.

### Soundtracks

A game can give players its music to listen to, a piece at a time as they
earn it. It lists the pieces from `Game::soundtrack` (or
`ctx.soundtrack.set_tracks` while running) and opens the menu with
`ctx.push(SoundtrackScreen::<MyGame>::new())`:

```rust
use hearken::{Stems, Track};

fn soundtrack() -> Vec<Track> {
    vec![
        // Unlocked the first time music/title plays.
        Track::clip("title", "Title theme", "music/title")
            .about("The title screen's theme.")
            .hint("Open the game."),
        // Layers recorded to loop together, arranged anew each play.
        Track::stems(
            "night-song",
            "The night song",
            Stems::new(8.0)
                .bed(["song/base"])
                .voices(["song/flute", "song/harp", "song/choir"]),
        )
        .unlocked_by(["music/night_complete"])
        .hint("Finish a night."),
        // Hidden, and not counted, until it is heard.
        Track::clip("waltz", "Midnight waltz", "music/waltz").secret(),
    ]
}
```

A track unlocks the first time any of its `unlocked_by` keys starts
playing, or when the game calls `ctx.unlock_track("night-song")` for a rule
the engine cannot hear. The game hears of new ones through
`ctx.soundtrack.take_new()` and announces them when it suits it. A screen
that plays sounds outside play, such as a sound test, calls
`ctx.soundtrack.listen(false)` while it is open; a key heard at any
moment listening was off never counts, even if the screen closed in the
same frame. Unlocks are saved to
`soundtrack.json` through `ctx.vault`, so a release with `seal = true`
seals them; a game that keeps progress elsewhere overrides
`Game::load_soundtrack` and `Game::save_soundtrack`. `SoundtrackScreen`
lists one game's tracks with `.album("My Game")` when a collection
groups them by `Track::album`. While it is open the game's music is
paused, and it goes on from the same place on Escape. Nothing played on
that screen unlocks a track, and playback stops when another screen is
pushed over it. A save keeps what `soundtrack.json` already holds and
writes nothing while the file cannot be read, so a file held for a moment
by a virus scanner or a sync client never costs the player their tracks:
a save that could not be made is tried again every few seconds, and once
more as the game closes. `Game::save_soundtrack` returns whether it saved.

The menu plays a track as the player heard it: a clip as recorded, and
`Stems` with the bed first and the voices joining one loop at a time in
the order listed, then all playing to the end. A track marked
`.varied()`, for music the game itself plays differently each time, is
instead a new performance on every play, from a fresh seed:

- `Stems` start every layer on the same sample (`Audio::start_together`),
  however long each takes to decode, so they never drift, and fade
  voices in and out at loop boundaries: which ones open, the order they
  join, a breakdown now and then, how it thins out, and how long it runs
  all change; a play runs up to a fifth longer or shorter than
  `length_s` (at most an hour). With no bed, no loop holds fewer than two
  voices. The arrangement keeps time by the wall clock as well as the
  frames, so a stalled frame does not put it behind the music. Arranging
  needs at least three voices with no bed, or two over a bed
  (`Stems::arranges`); with fewer, every loop is the full mix and a
  warning is logged. A piece whose stems do not all load (a damaged pack
  entry) does not play at all, and the screen says "Can't play that
  track." `tests/soundtrack.rs` checks that every voice is heard and that
  eight plays give at least six arrangements.
- `Variation` (by default `GENTLE`) moves the whole piece up or down a
  semitone, key and tempo together, never leaving it at the recorded
  pitch, and shades each voice's level and place. It moves at most an
  octave. `Variation::NONE` plays a track as recorded.
- `Track::live` hands the play to the game's own `Performer`, made from
  that play's `Variety`, for music the game composes as it goes.

#### Exporting an album

Export is off unless the game turns it on, an album at a time (`""` is a
game whose tracks have no album):

```rust
use hearken::soundtrack::AlbumOptions;

ctx.soundtrack.set_album("", AlbumOptions { export: true, vary_export: false });
```

The menu then shows an "Export album" row (or "Export Lantern Lake" for a
named album). It reads "Export album, locked: 2 of 5 heard" until every
track listed in that album is unlocked; a secret track counts once heard.
Export plays each track once on a mixer with no device, faster than real
time, and writes only its finished mix, never the stems, as a 16-bit
stereo 48 kHz WAV to `Music/<game>/<album>/01 <title>.wav`. When the
system has no Music folder it uses `Soundtrack` in the game's data folder,
and `ctx.soundtrack.set_export_dir` moves it anywhere else. A track that
would run on (a live piece that never ends) stops at 15 minutes. The game
keeps answering while it works; Escape stops it and leaves no half-written
file, and when it is done it says where the files are. With
`vary_export: true` each export is a fresh variation instead of the track
as heard.

In a developer build, `MY_GAME_ALL_TRACKS=1` opens every track to
audition, and saves nothing.

What this cannot protect, as no offline game can:

- **Recording the speakers.** A loopback recording of a play is that play.
  A voice's level and place hold for the whole play and the stems stay
  sample-aligned, so a patient listener can subtract one loop from the
  next to pull out the voices that changed, and undo the pitch step,
  which is a plain resample by a known amount. Any loop where every voice
  plays is the full mix. Varying each play makes that slow work; it does not
  make it impossible. A single-file `Track::clip` varies only in key and
  level, so a recording of it is close to the file.
- **Exports.** An exported file is the mix as it is, to share or keep. A
  player can export a varied album as many times as they like, and
  nothing limits that.
- **Copied saves.** The seal ties `soundtrack.json` to the game, not to
  the player. A sealed file copied from another player of the same game,
  or an old copy of one's own, is taken as it is.
- **The pack key.** Someone who digs it out of the program can read the
  stems (see `hearken-pack`).

## Optional features

`hearken`'s `kira` feature, which plays sound, and `testing`, which provides
`Headless`, are on by default. These are opt-in:

- `radio`: internet radio, as `hearken::radio::Stream`. On Windows and
  macOS, AAC goes through the system's own decoder (Media Foundation,
  AudioToolbox), so HE-AAC and HE-AAC v2 stations play in full stereo. On
  Linux, Symphonia decodes AAC, and an HE-AAC station plays its AAC-LC base
  layer only: half the bandwidth and, for v2, mono.
- `fdk-aac`: Fraunhofer's FDK AAC decodes radio's AAC in place of the
  system's decoder and Symphonia's, adding HE-AAC on Linux. It turns on
  `radio`. Fraunhofer's licence permits using FDK AAC only under AAC patent
  licences the game holds, and grants none itself: a game without them is
  not licensed to use it at all. Read the licence notes below first.
- `voice`: AI commentary voices, as `hearken::voice`. See Commentary voices.
  It adds ONNX Runtime to the build (downloaded prebuilt at build time).
- `voice-download`: `voice`, plus the in-game installer for the voice
  package. It adds `ureq` and rustls.
- `discord`: Discord Rich Presence. See Discord status.

## Discord status

A game can show its players' Discord friends what they are doing, as
Freight Fate does. Make an application at
<https://discord.com/developers/applications>, named after the game (Discord
shows that name as the game being played), and give its application ID:

```rust
impl Game for MyGame {
    // ...
    fn discord() -> Option<hearken::DiscordConfig> {
        Some(hearken::DiscordConfig::new("123456789012345678"))
    }

    // Shown when no screen reports one of its own.
    fn presence(ctx: &Ctx<Self>) -> Option<hearken::Presence> {
        Some(hearken::Presence::new("In the menus"))
    }
}
```

Turn on `hearken`'s `discord` feature in the game's `Cargo.toml`. Each screen
can report its own status with `State::presence` (or `Menu::presence`):

```rust
fn presence(&self, ctx: &Ctx<MyGame>) -> Option<hearken::Presence> {
    Some(hearken::Presence::new("Playing a round").detail(format!("Level {}", self.level)))
}
```

The engine asks from the top screen down and takes the first answer, then
falls back to `Game::presence`, so a help or Settings screen over play can
return `None` and the status stays on the game. Keep it to public game
content: never a save path, a typed name or debug text.

What the engine does with it:

- **Settings** gets a "Discord status" switch, on by default, saved in
  `settings.json`. Discord's own activity setting still applies.
- **Nothing on the game loop.** A worker thread owns Discord's pipe. A
  presence equal to the last one is dropped, changes go at most once every
  15 seconds (Discord's rate limit), and an elapsed-time counter runs from
  the start of the session (`DiscordConfig::without_elapsed` turns it off).
- **Silent when Discord is absent.** Not running, not installed, quitting
  mid-session or refusing the ID: the game plays on and says nothing. It
  tries to connect again every 30 seconds.
- **Idle players drop off.** After 30 minutes with no change the status
  comes down; the next change puts it back.
- **Quitting stays quick.** The status is cleared at quit, waiting at most
  two seconds, and only when Discord is connected.
- Tests (`Headless`) and agent sessions never touch Discord.

## Commentary voices

With `voice` on, a game speaks commentary in Kokoro's voices, synthesized
on the player's computer by [local-tts](https://github.com/GarAlex/local-tts)
and ONNX Runtime. No account, key or network is needed once the voice
package (`kokoro-en-q8`, about 92 MB) is installed.

```rust
use hearken::voice::{HasVoiceSettings, Role, Voice, VoiceConfig, VoiceSettings};
use hearken::Line;

const CREW: &[Role] = &[Role {
    id: "pbp",
    label: "Play-by-play",
    default_voice: "bf_emma",
    sample: "Welcome in, everybody.",
}];

// Once, at startup: the model loads on a worker thread.
let voice = Voice::new(&ctx, VoiceConfig::new(CREW));
ctx.set_commentary(Box::new(voice));

// On entering a screen, get its fixed lines ready.
ctx.prime_commentary(&[Line::new("pbp", "Tip-off!")]);

// Any time: returns at once, plays when ready.
ctx.commentate(Line::new("pbp", "Hawks lead 54-49 at the half.").key("score"));
```

The game's `Settings` type holds a `VoiceSettings` and implements
`HasVoiceSettings`, and its `Game::settings_rows` adds
`hearken::voice::settings_rows(CREW)`: who speaks commentary (AI voices,
screen reader, or off), a voice for each role (Enter plays its sample), and
the speed.

How lines are delivered:

- One line plays at a time on `Bus::VOICE`, so the crew never talks over
  itself. The mix ducks under it as it does under event speech, when the
  player has "Lower game sound under speech" on. F2 stops it.
- A line takes 1 to 6 seconds to synthesize on a modest computer, so lines
  are cached in memory and as WAV files under `voice-cache/` in the game's
  data folder (256 MB at most, oldest first). `prime` fills the cache
  ahead of time without saying anything. A cached line plays the same frame.
- The pacer's rules apply: the same line within 2.5 s is said once, a
  keyed line only when its text changes, and a line that would start more
  than 4 s (`Ambient`) or 8 s (`Important`) after it was asked for is
  dropped. A `Critical` line cuts off the voice and clears the queue.
- The player's verbosity level applies to each line's category, as it does
  to `say_event_with`.
- With no package installed, a model that will not load, or the player's
  setting on the screen reader, every line goes to the event voice through
  `ctx.say_line`, and Settings says why. A build without `voice` does the
  same, so `ctx.commentate` always works.

`hearken::voice::FakeVoice` stands in for `Headless` tests: it records each
line at once (`VoicedLog::texts`), following the voice setting and the
verbosity level, so tests assert commentary with no model.

### Getting the voice package to players

Either works, and a game can offer both:

- **In the release zip.** Install the package into the game's own `voices`
  folder and list `voices` in `[package.metadata.hearken] include`; `Voice`
  finds `voices/kokoro-en-q8` beside the program. The zip grows by about
  92 MB. To install, call `hearken::voice::install` (with `hearken`'s
  `voice-download` feature) from a small program of the game's own, or
  from Hearken's repository run
  `cargo run -p hearken-voice --example install --features download -- <game>/voices`.
- **Downloaded on first use.** With `hearken`'s `voice-download` feature,
  the Settings rows gain "Download the AI voices", which asks first
  (`install_confirm`) and then downloads into `voices` in the game's data
  folder, checking each file's pinned SHA-256, and switches to the AI
  voices when it is done.

`<PREFIX>_VOICES` points any build at another voices folder. A
`lexicon.txt` (CMU format) in the voices folder, or `VoiceConfig::lexicon`,
corrects how names are said; editing it re-synthesizes the cached lines.

### On Windows

ONNX Runtime links statically, but the prebuilt one includes Microsoft's
DirectML, so the program needs `DirectML.dll` beside it. The build puts it
in `target/<profile>`, and `hearken release` ships it with the other DLLs,
as it does the Visual C++ runtime, adding `DirectML-LICENSE.txt` and
`DirectML-ThirdPartyNotices.txt` beside it. Hearken's voices run on the CPU; DirectML
is only loaded. A game that wants no Microsoft DLL can build ONNX Runtime
itself without DirectML and point `ORT_LIB_LOCATION` at it.

## Working on Hearken

`cargo test --workspace` runs every crate's tests and the template's
walkthrough. CI also runs `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets -- -D warnings` on Windows and
Linux for every pull request, and on macOS for release tags. Set
`HEARKEN_TEST_NO_SPEECH=1` to run the tests without a screen reader.

The engine's design and the notes behind it are in `docs/superpowers/`,
starting with `specs/2026-09-26-hearken-engine-design.md`.

## Licence

Hearken is licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. Prism's screen reader clients
keep their own licences. Two decoders compile into every game with sound,
and the game's credits or licence notes carry what they ask:

- Symphonia (MPL-2.0): where its source is, when shipped unchanged.
- libopus (BSD-3-Clause): its copyright notice and licence text, from
  [opus-codec.org/license](https://opus-codec.org/license/). Xiph's,
  Broadcom's and Microsoft's Opus patents are licensed royalty-free; Vectis
  IP's Opus pool covers hardware devices, not software.

The opt-in features compile in more, all of it into the game, with no DLLs
but one (`voice` on Windows):

- `radio`: `ureq`, `icy-metadata` and `url` (each MIT or Apache-2.0),
  `m3u8-rs` and `mpeg2ts` (MIT), and Symphonia's AAC decoder (MPL-2.0, as
  the rest of Symphonia). On Windows and macOS, `windows` or
  `objc2-audio-toolbox` (each MIT or Apache-2.0) bind the system's AAC
  decoder, which ships with the operating system: Microsoft and Apple
  license the AAC patents for it, and the game carries no AAC decoder of
  its own there. `ureq`'s TLS is rustls, which is built on `ring`
  (Apache-2.0 and ISC) and brings in `rustls-webpki` and `untrusted` (ISC).
  `url` brings in ICU's Unicode data through `idna` (Unicode-3.0).
- `fdk-aac`: `symphonia-adapter-fdk-aac` ((MIT or Apache-2.0) and MPL-2.0),
  the `fdk-aac` and `fdk-aac-sys` crates (MIT), and Fraunhofer's FDK AAC
  library, under its own licence. That licence asks the game to keep its
  complete text with its credits or licence notes and to make the FDK AAC
  source available free of charge to anyone who gets the game. It permits
  use only under AAC patent licences the game holds, and grants none: see
  Optional features.
- `voice`: local-tts (MIT), ONNX Runtime (MIT) through `ort` and `ort-sys`
  (MIT or Apache-2.0), Hugging Face `tokenizers` (Apache-2.0), `ndarray`
  (MIT or Apache-2.0) and `rustfst` (MIT or Apache-2.0). The voice package
  is data, not code: the Kokoro-82M weights and voices (Apache-2.0) and the
  CMU Pronouncing Dictionary (BSD-2-Clause), whose notices the game's
  credits carry. On Windows the game also ships `DirectML.dll` (1.15.4),
  which is not open source: Microsoft's DirectML licence lets a developer
  distribute it inside games and applications for Windows and Xbox, but not
  on its own, and its notices must not be removed. `hearken release` puts
  the licence and its third-party notices (Microsoft GSL and half, MIT;
  Zstandard, BSD-3-Clause, whose notice must travel with the binary) beside
  the DLL. The game's own MIT or Apache-2.0 licence does not cover the DLL,
  so the game's licence notes say so. The licence also says DirectML may
  send usage data to Microsoft.

These lists name the notable licences, not every crate's. `cargo about`
builds the complete notice for what a game actually compiles in.
