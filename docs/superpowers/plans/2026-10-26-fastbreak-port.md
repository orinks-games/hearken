# Fastbreak on Hearken: port plan

- Date: 2026-10-26
- Status: approved 2026-10-26, awaiting execution
- Source game: `Orinks/fate-engine`, `examples/fastbreak` on `dev` at `d043a78`
- Target engine: `orinks-games/hearken`, `main` at `d826403`
- Models: this plan was written on Fable 5.1 at high effort. Each execution
  task below runs on Opus 5.5 at medium effort unless its row says otherwise.

## Summary

Fastbreak is the Fate Engine's flagship audio basketball game: eight invented
teams in the Continental Basketball League, a 14-game season with standings
and a two-round playoff, a 32-possession match the player paces by hand, and
a broadcast crew (Dana Wells on play-by-play, Marcus "Big M" Cole on color)
with pregame, halftime and postgame shows. It is authored entirely as data:
a Python build script writes JSON scenes, state machines and ~2,000 authored
lines, and the Fate runtime selects those lines at play time.

Hearken is a Rust engine, and its design spec already records the decision
that the Python `fate-engine` repository is archived, not reused. So this
port is a rewrite, not a translation: the game's content (teams, schedule,
simulation, lines, shows, achievements) moves across intact and verifiably
identical, and its mechanics are re-expressed as Rust on Hearken's screen
stack, menus, event voice, positioned audio and headless tests. The result
should feel like the same game with a better engine under it: real
positioned sound, a crowd that reacts, menus the player can rebind, and
commentary spoken by neural voices instead of the screen reader.

Three requirements from Orinks shape every task:

1. Sound and music are made with Genny, blal1's offline synthesizer
   (`github.com/blal1/genny`, Python, MIT with a Klang attribution clause).
2. Commentary is dynamic speech from a locally installable neural
   text-to-speech package with no API keys or accounts.
3. Planning runs on Fable 5.1 at high effort; execution on Opus 5.5 at
   medium, higher only where a task needs it.

## What the source game is

Read these before any task: `examples/fastbreak/README.md`, `DESIGN.md`,
`fastbreak_data.py`, `fastbreak_season.py`, `fastbreak_court.py`,
`fastbreak_shows.py`, and the journeys in
`packages/fate-core/tests/project/test_fastbreak_example.py`. `build.py`
(3,875 lines) is the authoritative record of every machine and line; read the
function for the piece you are porting, not the whole file.

| Piece | Where it lives in Fate | What it does |
|---|---|---|
| League data | `fastbreak_data.py` | 8 teams with offense/defense ratings (1 to 99) and an identity line; circle-method double round-robin (14 rounds); an LCG-seeded integer score simulation for every non-player game. Deterministic. |
| Season content | `fastbreak_season.py` | Per team: next-game lines (17, keyed by game index), schedule lines (14), East/West standings lines (17), around-the-league lines (17), record lines (15). Playoff opponents: the best simulated record in your conference, then the best in the other. Line budgets: 240 characters per readout. |
| Match content | `fastbreak_court.py` | 32 possessions, 8 per quarter; clock-check lines (32); score-check lines (61, margin -30..+30); margin buckets for breaks and the buzzer; line families for possession start, shot release, made basket, miss, proximity, gather, timeout, final buzzer. 120-character cap on lines. |
| Shows | `fastbreak_shows.py` | Center Court pregame reel: orientation, three desk frames, an opponent matchup line (8) and Big M's keys (8), skip line, hand-off. Postgame Wrap win/loss lines. Segment reels advance only on the player's press; Backspace skips to tip-off. |
| Variables | `variables.json` | home/away score, margin offset, possession count, ball-live, paused, game index (0..16), opponent (1..8), opponent tier, team (1..8), season wins/losses, last result, verbosity (0 essential, 1 standard, 2 full). |
| Achievements | `achievements.json` | First Win, Winning Record (8 wins), Perfect Season (14), Continental Champion. |
| Input | `input.json` | Move up/down/left/right, Choose, Go back, Help, Pause; Confirm or shoot (hold), Continue season, Commentary verbosity, My record, Score check, Clock check, Next game details, East/West standings, Around the league. Keyboard and controller. |
| Audio | `fastbreak_audio.py` | Ten integer-math placeholder WAVs: menu theme, UI move/select, buzzer, bounce, swish, rim, whistle, crowd bed, crowd roar. Each with a caption. All replaced by Genny in this port. |

### How the match works in Fate (the rules to keep)

- The player presses Activate to start each possession; nothing advances on
  a wall clock. Possession 32 arms the buzzer; the next press ends the game.
- Shooting is hold-to-release on Confirm: a tap releases a mid-range jumper
  after a 350 ms gather; still held at 350 ms goes to a deep gather, and at
  750 ms a three-point attempt fires. "You rise for the jumper" / "You let
  it fly from deep".
- In Fate the ball is a physics body and the hoop a trigger zone, so the
  made/missed outcome came from a ballistic arc. The defender is a body that
  drifts toward your hoop, chases the ball when within 60 units ("Defender
  closing"), rips the ball when within 20 units while the ball is dead
  ("They rise, live ball", and the ball flies at their hoop), and announces
  "Your man breaks free" past 160 units at verbosity 1 or more.
- Two referee machines add 2 to a score and move the margin offset when a
  hoop zone is occupied. (Three-point scoring is in the line families but
  the Fate referee only ever added 2; the port fixes this: deep makes score 3.)
- Quarter boards fire at possession 8, 16 and 24 with the bucketed margin
  line; the second one is The Halftime Report.
- Pause is an honest timeout: every machine parks in a huddle state, the
  ball stops, a held shot is waved off, score and clock checks stay open.
- The final buzzer branches on the margin into six result lines, adds a
  season win or loss, records last-result, advances the game index and
  transitions to the locker room. A tie goes to sudden death: next basket
  wins.
- The locker room speaks the Postgame Wrap, unlocks milestones, routes a
  winning record (8+) into the playoffs after game 14, and the champion
  line after the Continental Final.

## What Hearken gives us

Read `README.md`, `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`,
`docs/superpowers/notes/2026-10-17-freight-fate-on-hearken.md` and
`template/` before any task.

- `hearken::Game` with `Data`, `Settings` (serde, saved under `"game"` in
  `settings.json`), `Action` (rebindable keys and pad buttons), `Category`
  and `verbosity()` (a table of what each event category does at each level
  the player picks), `sound()` (buses, duck, earcons), `builtin_clips()`.
- `State` screens on a queued stack, `Menu` with help per row, built-in
  Settings, Help, Pause, Confirm, text entry and key binding screens.
- `ctx.say` for the screen-reader channel; `ctx.say_event_with(text,
  SayEvent::new().priority(..).category(..).key(..).terse(..))` for the
  gameplay channel with the pacer (repeat suppression, standing conditions,
  priorities Ambient/Important/Critical) and the duck.
- `ctx.audio`: `play`, `play_at(pos)`, `start_loop`, `start_voice` (a
  fundsp recipe with live parameters), `play_clip(key, bytes, bus, Start)`
  for audio the game made itself, `set_bed`, `play_music`, buses MASTER,
  UI, EFFECTS, AMBIENCE, MUSIC, VOICE plus the game's own.
- Sound files by key in a `sounds` folder next to the game
  (`sfx/swish.wav`); a built-in stand-in when a file is missing;
  `hearken release` packs the folder encrypted.
- `Headless` for tests: press keys, read `spoken()` and `played()`,
  advance frames. The `agent-server` feature lets an agent play the real
  game by ear.
- `hearken-cli`: `hearken play orinks-games/<repo>` and `hearken release`.

What Hearken does not have, and this plan adds:

- A neural voice service. New optional crate `hearken-voice` behind a
  `voice` feature on the facade (tasks 7 and 8). Hearken's design spec rules
  out sound-source integrations for files; a runtime voice is a different
  thing, and the football game on the roadmap wants commentary too.
- A save file helper. Fastbreak needs a season save beyond settings. The
  game writes `season.json` under `ctx.paths` itself; if a second game
  wants the same thing later, lift it into Hearken then, not now.

## Decisions

| Decision | Choice | Why |
|---|---|---|
| Where the game lives | A new repository `orinks-games/fastbreak`, started from `template/`, with `hearken` as a git dependency pinned to a commit. Approved by Orinks on 2026-10-26. | Matches Freight Fate and Echo Parlor; `hearken play orinks-games/fastbreak` works by name; Hearken's workspace stays engine-only. |
| Crate layout | `fastbreak-core` (engine-free rules and content, no Hearken dependency) and `fastbreak` (the game: `Game`, screens, sound, commentary). | The Freight Fate precedent: rules in a headless crate so they test without a window and the content parity tests stay fast. |
| Content parity | Teams, schedule, simulated scores, standings, playoff opponents and every authored line are ported verbatim and checked against a golden JSON generated from the Python modules. | The season's reproducibility is a feature of the design; a port that changes a score is a different game. |
| Match outcome model | Replace the 2D physics arc with an audio-first shot model: outcome from release timing, shot distance, defender proximity and team ratings, drawn from a per-game seeded RNG. Deep makes score 3. | Hearken has no 2D physics and the Fate arc existed to drive a sprite view. The player hears timing and proximity; a seeded model keeps tests deterministic. |
| Sound | Every clip is rendered offline with Genny from JSON specs committed beside the WAVs in the game repo; the crowd is a Hearken live voice (fundsp) fed by game state, with Genny crowd layers for roars and groans. | Orinks's requirement; Hearken takes files and the loose folder packs on release. The crowd must swell with the play, which is what live voices are for. |
| Commentary voice | `local-tts-core` (GarAlex/local-tts) running Kokoro-82M Q8 inside the game, wrapped by `hearken-voice`. Screen-reader commentary stays as the fallback and as a setting. | See the recommendation below. |
| Verbosity | Hearken's table replaces the Fate `verbosity` variable: three levels (Essential, Standard, Full broadcast) over the categories PlayByPlay, Color, Proximity, Crowd, Readout, Clock, Show. | Same three player-facing levels; the engine applies them, and Settings grows the row on its own. |
| Lines with numbers | Dynamic lines (scores, margins, records, possessions) are formatted at run time, not authored per value as Fate had to. The Fate line tables stay as the fixed vocabulary and as the parity fixture. | Fate authored 61 margin lines because its runtime could not format text; Rust can. |

## Commentary voice: research and recommendation

The requirement: dynamic commentary from a package that installs locally
into Hearken or the game, with no API keys or cloud accounts, in a Rust
game that ships under MIT or Apache-2.0.

| Option | What it is | Fit |
|---|---|---|
| **local-tts** (`GarAlex/local-tts`, crate `local-tts-core` 0.7.0, MIT) | Native Rust inference for Kokoro-82M (Apache-2.0 weights), VITS, Piper and Vosk voices on ONNX Runtime through the `ort` crate. Its English frontend is a BSD CMU dictionary plus native grapheme-to-phoneme, with NVIDIA NeMo normalization grammars compiled in: "The Lakers won 112-108" is spoken as scores, not digits. 28 English voices in the `kokoro-en-q8` package (92 MB, pinned SHA-256, fetched from Hugging Face with no account). Cancellable synthesis, a lexicon for custom pronunciations, no eSpeak, no Python at inference. | **Recommended.** Pure Rust dependency, permissive licences end to end, sports-aware text, voices of both sexes and two accents for the two commentators. |
| sherpa-onnx (`sherpa-rs`) | The k2-fsa C++ toolkit with many TTS models (Piper/VITS, Kokoro, Matcha), built with CMake and a downloaded ONNX Runtime. | Mature and fast, but its Piper and Kokoro paths phonemize through espeak-ng, which is GPL-3, so a game linking it is GPL. Heavy first build. Keep as the fallback if local-tts fails task 7. |
| piper-rs, kokoros | Rust wrappers for Piper and Kokoro that link espeak-ng for phonemes. | Same GPL-3 problem. |
| System speech through Prism (SAPI, NVDA, VoiceOver) | What Hearken already does. | Not an "AI voice" and it is the screen reader's channel; it stays as the fallback and as a player setting. |
| Genny `say` | Genny's Klatt-style formant voice for letters, numbers and short words. | A robotic PA voice, not commentary. Usable for arena announcer stings only. |

Risks with the recommendation, and what the tasks do about them:

- local-tts is young (0.7.0), pins `ort = 2.0.0-rc.10`, and its own docs say
  Linux and Windows packaging are unverified. Task 7 is a spike that proves
  it builds on Hearken's three CI platforms and reports latency before any
  game code depends on it. `ort` fetches prebuilt ONNX Runtime binaries at
  build time; the spike records whether the link is static on each platform
  or whether `hearken release` must ship a shared library.
- Kokoro on a laptop CPU runs at roughly a third of real time or better, so a
  one-second line costs a few hundred milliseconds. Commentary cannot wait on
  that during a possession. Task 8's service pre-synthesizes every fixed line
  into a cache (memory and a `voice-cache` folder under the game's data
  directory) when a screen opens, so play-by-play is instant after the first
  run, and only lines with fresh numbers are synthesized live, off the loop,
  and dropped if they would arrive stale.
- Invented names (Skyliners, Big M, Cascades) may be unknown to the
  dictionary, and local-tts refuses unknown words rather than guessing.
  Task 9 builds a lexicon for every team, city and broadcaster name and
  asserts every authored line synthesizes.
- The voice package is 92 MB. The game ships it in the release zip (listed
  in `[package.metadata.hearken] include`) or installs it on first run with
  local-tts's `download` feature behind a Confirm screen. Task 8 implements
  both paths; the release choice is Orinks's.

Voice casting to start with (task 9 may change it after a listen): Dana
Wells on `bf_emma` (UK female) and Big M Cole on `am_michael` (US male), so
the two are distinguishable at a glance.

## Sound: Genny

Genny (`pip install genny` or `python install.py --tool` from a clone) is a
Python command that renders WAVs from JSON specs, deterministically. It has
no place at run time in a Rust game; it is the authoring tool. The game repo
keeps `audio/genny/*.json` specs and the rendered `sounds/**/*.wav` side by
side, and a `audio/render.sh` (or `.ps1`) that re-renders the whole kit with
`genny render`. Genny also ships a Claude Code skill (`skill/genny`) that an
execution session can install; `AGENTS.md` in the Genny repo is the manual.

Catalog entries that matter for a basketball arena: `bounce` and `drop`
(contact models with restitution), `squeak` (friction), `impact`,
`crowd_murmur`, `applause`, `handclap`, `buzzer`, `car_horn` (for an arena
horn), `whistle_human`, `cyber_ui` and `ui_tick` for menus, the `compose`
and style loops for music, and `genny say` for a PA announcer.

Licensing: parts of Genny are adapted from the Klang library under the Klang
Open License 1.0, which asks interactive products for attribution. Task 6
adds that notice to the game's credits and licence notes, alongside
Hearken's own Symphonia and libopus notes.

## Sound keys the game will play

| Key | Source | Notes |
|---|---|---|
| `ui/menu_open`, `ui/menu_move`, `ui/menu_select`, `ui/menu_back`, `ui/tick`, `ui/error`, `ui/notify` | Genny `cyber_ui`, `ui_tick`, `beep` | Override Hearken's built-in earcons with a broadcast-desk flavour. |
| `sfx/dribble` | Genny `bounce` on hardwood | Played at the player's position while the ball is dead in your hands. |
| `sfx/gather`, `sfx/release_mid`, `sfx/release_deep` | Genny `whoosh`, `squeak` | Gather and release cues; the deep release is longer. |
| `sfx/swish`, `sfx/rim_in`, `sfx/rim_out`, `sfx/backboard` | Genny `impact`, `twang`, metal plate | Positioned at the hoop. A miss plays `rim_out` then a loose-ball `bounce`. |
| `sfx/steal`, `sfx/footsteps_defender` | Genny `squeak`, `footsteps` | Defender proximity: footsteps and squeaks at the defender's position, louder as it closes. |
| `sfx/whistle`, `sfx/buzzer`, `sfx/horn` | Genny `whistle_human`, `buzzer`, `car_horn` | Timeout, quarter end, final buzzer. |
| `amb/crowd` | Hearken live voice (fundsp babble) with Genny `crowd_murmur` and `applause` layers | Parameters `excitement` and `home_edge` from the match state. |
| `sfx/crowd_roar`, `sfx/crowd_groan`, `sfx/crowd_oh` | Genny `applause`, `crowd_murmur` with `scream`/`moan` layers | Made shots, misses, steals. |
| `music/menu`, `music/center_court`, `music/halftime`, `music/postgame`, `music/champion` | Genny `compose` and style loops, `loop: fold` | Broadcast stings and a looping title theme. |
| `sfx/pa_*` | Genny `say` through `telephone` and `reverb` | Optional arena PA flavour ("Three!", "Timeout") on Full broadcast only. |

## Verbosity table

| Category | Essential | Standard | Full broadcast |
|---|---|---|---|
| PlayByPlay (possession start, release, make, miss, steal) | Terse | Full | Full |
| Color (Big M's lines, runs, milestones) | Silent | FirstTime | Full |
| Proximity ("Defender closing", "breaks free") | Earcon `sfx/footsteps_defender` | Full | Full |
| Crowd (crowd callouts) | Silent | Silent | Full |
| Readout (score check, clock check, standings) | Full | Full | Full |
| Clock (quarter breaks, buzzer) | Full | Full | Full |
| Show (pregame, halftime, postgame segments) | Terse | Full | Full |

Readouts are always spoken through the screen reader (`ctx.say`), because
the player asked for them. Play-by-play, color and shows go to the
commentary voice when it is on, and to `say_event_with` otherwise. The
verbosity rule applies before either path.

## Execution tasks

Each task is one Opus 5.5 session at medium effort unless marked. Each ends
with `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --
-D warnings` and `cargo test --workspace` green, a commit, a push and a pull
request against the repository named, with this plan linked in the body.
Where a task says "golden" it means a fixture generated from the Python
modules in `fate-engine` (the project has that repository; run
`uv run python` from its root) and committed with the Rust test that reads
it.

Order and dependencies:

```
T0 repo ─┬─ T1 core data ── T3 menus/hub ─┬─ T5 shows ──┐
         │                                │             ├─ T10 polish/release
         ├─ T2 core match ── T4 court ────┴─ T9 commentary
         ├─ T6 Genny sound kit ──────────────────────────┘
         └─ T7 voice spike (hearken) ── T8 hearken-voice ── T9
```

T1, T2, T6 and T7 can run in parallel once T0 exists. T7 and T8 are pull
requests against `orinks-games/hearken`; everything else against the game
repository.

### T0. Create the game repository from the Hearken template

Repository: the one Orinks picks (default `orinks-games/fastbreak`).

- Copy `template/` into the new repo as the `fastbreak` package with
  `build.rs`, `help.md`, CI (`ci.yml` adapted: Windows and Linux on pull
  requests, macOS on tags; the `linux-deps` action copied), a `LICENSE-MIT`
  and `LICENSE-APACHE`, and a `README.md` naming the toolchain
  requirements from Hearken's README.
- `hearken = { git = "https://github.com/orinks-games/hearken", rev = "<commit>" }`
  pinned, with a `voice` feature mapped through once T8 lands.
- Add the `fastbreak-core` library crate (empty) and make the workspace
  `[workspace] members = ["fastbreak", "fastbreak-core"]`.
- `[package.metadata.hearken] portable = false, pack = true, include =
  ["sounds", "voices", "README.md", "LICENSE-MIT", "LICENSE-APACHE",
  "CREDITS.md"]`.
- `Game` constants: `TITLE = "Fastbreak"`, `DATA_DIR = "Fastbreak"`,
  `ENV_PREFIX = "FASTBREAK"`.
- Accept: the template walkthrough passes under the new names;
  `hearken play .` builds a window; CI green.

### T1. `fastbreak-core`: league, schedule, simulation, season lines

- Port `fastbreak_data.py` and `fastbreak_season.py` verbatim: `Team`,
  `TEAMS` (same order), `double_round_robin`, the LCG (`state * 1103515245
  + 12345 mod 2^31`) and `simulated_score`, `opponent_and_home`,
  `other_games`, `simulated_records_after`, `playoff_opponents`, and the
  five line generators. Keep integer math exact; use `u64`/`i64` and the
  same modulus.
- Golden: generate `tests/golden/season.json` from Python with, for every
  team and game index, the opponent, home flag, the three other games with
  scores, both conference standings lines, the around-the-league line, and
  the playoff opponents; plus every next-game, schedule and record line.
  The Rust test asserts byte equality.
- `SeasonState` (team, game index, wins, losses, last result, playoff stage,
  unlocked achievements) with `serde` and the transitions the locker room
  applies: record result, advance, enter playoffs at 8+ wins after game 14,
  champion after the final, season complete.
- Accept: golden test green; a property test that every team plays every
  other twice, once at home.

### T2. `fastbreak-core`: the match engine

- A `Match` struct advanced by `MatchInput` (StartPossession, ShootPress,
  ShootRelease, Timeout, Resume, ScoreCheck, ClockCheck) and `tick(dt)`,
  emitting `MatchEvent`s that carry the category, priority and the line
  text from the ported `fastbreak_court.py` families (`possession_start`,
  `shot_release`, `made_basket`, `miss`, `proximity`, `gather`, `pause`,
  `final_buzzer`, the 32 clock-check and 61 score-check lines kept as a
  golden fixture even though the port formats them at run time).
- Rules as listed under "How the match works in Fate", with the shot model
  from the Decisions table: gather at 350 ms, deep at 750 ms, release
  timing window around each, make probability from distance, defender
  proximity, release accuracy and the opponent's defense rating; defender
  movement as a 1D/2D position that drifts, closes and rips with the same
  thresholds (60, 20, 160 units) scaled to court units; opponent
  possessions resolved by the same model from their offense rating, so a
  Beacons game feels different from a Tide game.
- Positions for audio: player, ball, defender and both hoops as `Vec3` on
  NVGT axes (x east along the court, y north, z up), exposed for the court
  screen.
- Seeded RNG (`rand` with a fixed seed per game index and opponent) so a
  test can replay a game.
- Accept: unit tests for the Python journeys: court entry resets, a quick
  tap scores two and announces once, a quarter break announces the bucketed
  margin, sudden death records a walk-off win, a timeout holds and resumes
  honestly, deep make scores three; the fixture of line families equals
  Python's.

### T3. Screens: title, team select, season hub, save and continue

- `Title` menu: New season, Continue season (hidden when no save),
  Commentary verbosity (an adjustable row mirroring Settings), Settings,
  Help, Quit. Escape says how to leave, as the template does.
- `TeamSelect`: a wrapping list of eight franchises; Left/Right jump to the
  other conference; Enter speaks the confirm text and a second Enter (or a
  hold on Confirm, as Fate had) commits. Back returns to the title.
- `SeasonHub` menu, one screen for all teams (Fate needed eight because
  its runtime could not index; Rust indexes): Play next game, Next game
  details, My record, Schedule (Up/Down browse), East standings, West
  standings, Around the league, Back to menu. Every readout is one bounded
  line from T1.
- Save: `season.json` under `ctx.paths` (`Paths::at`-style sibling of
  `settings_file`), written after every result; Continue season loads it
  and lands on the hub.
- Actions enum with the Fate defaults (arrows, Enter, Escape, F1, P for
  timeout, Space for Confirm/shoot, S score check, C clock check, N next
  game, R record, E/W standings, A around the league) and pad buttons.
- Accept: headless tests for the menu announcing itself, team select
  carrying the team into the hub, back from team select, continue after a
  recorded result; `lines()` renders each screen for the agent server.

### T4. The court screen

- `Court` state owning a `Match` from T2 and the sound: crowd live voice
  started on enter, listener at the player, dribble at the player, hoop
  sounds at the hoops, defender footsteps and squeaks at the defender's
  position, each `MatchEvent` mapped to a sound key and a spoken line
  through the commentary service (T9) or `say_event_with` with its category.
- Timeout pushes a `TimeoutScreen` (score check, clock check, help, resume)
  over the court; the match is paused underneath; a held shot is waved off.
- Quarter breaks push a short `BreakScreen` that speaks the bucketed line
  (the second one is The Halftime Report from T5) and resumes on Enter.
- The final buzzer records the result into `SeasonState`, saves, and
  replaces the court with the locker room (T5).
- Accept: headless journeys from T2 driven by keys with `spoken()` and
  `played()` assertions, including that a miss plays `sfx/rim_out` at the
  hoop position and the crowd voice's `excitement` rises after a make.

### T5. Shows: Center Court, The Halftime Report, Postgame Wrap, playoffs

- `CenterCourt` reel: orientation line, three frames, the opponent matchup
  line, Big M's key, hand-off; Enter advances, Backspace skips to tip-off,
  a held Enter pages (400 ms dwell as Fate had). Then the court.
- Halftime: the bucket line plus a Big M observation chosen from the margin
  and the opponent identity.
- `LockerRoom`: Postgame Wrap win/loss line, milestone board (achievement
  unlock texts from `achievements.json`, persisted in the save), the
  playoff desk (semifinal with a winning record, "missed the postseason"
  otherwise, the Continental Final, the champion line), then back to the
  hub or the title when the season is complete.
- Accept: headless tests mirroring the Python show tests: reel plays
  through, skip goes straight to tip-off, postgame speaks the result on
  arrival, playoff desk routes the semifinal, a missed postseason is
  answered honestly, milestones unlock and the champion takes the title.

### T6. The Genny sound kit and the crowd

- Install Genny (`git clone https://github.com/blal1/genny && python
  install.py --tool`, or `pip install genny`) and its Claude Code skill;
  read `AGENTS.md` and `docs/catalog.md`.
- Write `audio/genny/kit.json` (a batch) producing every key in the "Sound
  keys" table, with a one-line description per sound kept in
  `audio/genny/descriptions.toml` for a Learn Sounds screen; render into
  `sounds/`; commit specs and WAVs; add `audio/render.sh` and `.ps1`.
- Music: a looping title theme, four show stings, a champion fanfare, each
  with `bpm` and `loop: fold`; pick styles from Genny's palette
  (`orchestral_heroic` or `funk` for the broadcast desk is a reasonable
  start).
- The crowd: a fundsp recipe in `fastbreak/src/sound.rs` (start from
  Hearken's babble idea in the design spec and the template's engine
  recipe) with `excitement` and `home_edge` parameters, plus Genny roar,
  groan and "oh" clips layered on events.
- `CREDITS.md` with the Klang attribution, Symphonia and libopus notes.
- Accept: a headless full-season run reports `missing_clips()` empty;
  `genny info` shows every clip peaks at or below -1 dBFS; the Learn Sounds
  descriptions cover every key.

### T7. Hearken: voice spike (may need high effort)

Repository: `orinks-games/hearken`.

- Add `crates/hearken-voice` depending on `local-tts-core` (git or
  crates.io, pinned) with a `Kokoro` loader behind a `voice` feature on
  `hearken` (off by default, like `radio`), and a `voices/` lookup next to
  the game or under `<PREFIX>_VOICES`.
- Prove it builds in `ci.yml` on Windows, Linux and macOS with the feature
  on; record in `docs/superpowers/notes/` whether ONNX Runtime links
  statically on each platform, the first-build time, and the synthesis
  latency of ten sample lines on the CI runner.
- If local-tts cannot be made to build on all three, stop and report;
  the fallback is `sherpa-rs` with the licence question put to Orinks.
- Accept: `cargo test -p hearken-voice --features voice` synthesizes a
  line into WAV bytes that `hearken-audio` decodes, on all three
  platforms; the note is committed.

### T8. Hearken: the `hearken-voice` service

Repository: `orinks-games/hearken`.

- `Voice` service: loads a model once on a worker thread; `speak(Line {
  text, speaker, priority, key, category })` returns at once; synthesis
  runs on the worker with a cache keyed by (speaker, text) in memory and
  under `voice-cache/` in the game's data folder as WAV; playback through
  `ctx.audio.play_clip` on `Bus::VOICE`; a queue with the pacer's rules
  (drop a repeat within 2.5 s, drop a line that would start stale for its
  priority, Critical interrupts); `prime(&[Line])` to pre-synthesize a
  screen's fixed lines; `stop()` wired to F2 beside speech; `is_speaking`.
- Duck: while the voice plays, lower the game's duck buses as event speech
  does (reuse `SoundConfig::duck_buses` and `duck_level`).
- Fallback: when no model is installed or loading fails, `speak` routes
  to `say_event_with` and Settings says why. A `VoiceSettings` row group:
  Commentary voice (AI voices, screen reader, off), speaker per role, rate.
- Installer: `Voice::install(package)` using local-tts's `download`
  feature with its pinned SHA-256, behind a Confirm screen the game shows;
  and a documented path for shipping `voices/` in the release zip.
- Testing: a `FakeVoice` that records `voiced()` lines for `Headless`,
  so a game's transcripts assert commentary text without a model.
- Accept: tests for cache hits, queue dropping, fallback, duck, and the
  fake; `template/` gains nothing (the template stays voice-free); README
  gains a Voice section and the licence notes for Kokoro (Apache-2.0),
  local-tts (MIT) and ONNX Runtime (MIT).

### T9. Commentary: the broadcast crew speaks

- `fastbreak-core::commentary`: line pools per event with variation
  (three to five phrasings each, chosen by a seeded rotation so no line
  repeats twice running), dynamic templates with numbers and names (score
  after every make, margin and possessions at breaks, the opponent's
  identity, runs of three makes, milestones such as the first lead), the
  two speakers' roles (Dana on play-by-play, Big M on color), and the
  Fate lines kept as the first phrasing of each pool so the golden tests
  still hold.
- A lexicon file (`voices/lexicon.txt`, CMU format) for every team, city
  and broadcaster name; a test synthesizes every pooled line through the
  real model when `FASTBREAK_VOICES` points at one (skipped otherwise).
- Wire the court, breaks and shows to `hearken::voice::Voice` with
  `prime` on screen enter and the verbosity table above; readouts stay
  on `ctx.say`.
- Accept: headless tests with `FakeVoice` showing every match event
  produces one commentary line in the right category, that Essential
  silences color, and that the screen-reader path carries the same text
  when the voice is off.

### T10. Polish, accessibility pass and release

- `help.md` sections for every screen; `Learn Sounds` from T6's
  descriptions; README for players (install, voices, controls) and for
  developers (Genny, lexicon, tests, agent server).
- An agent-server playtest session (`--features agent-server`, `lockstep`
  on) through a full game, recorded with `record_start`; fix what it finds
  a player cannot hear.
- `hearken release` zip with sounds packed and voices included or the
  first-run installer; a GitHub release workflow on `v*` tags.
- Optional: a `fastbreak.yml` downstream workflow in Hearken, like
  `freight-fate.yml`, so a Hearken change can be proved against Fastbreak.
- Accept: a human verification checklist in `docs/` with the journeys a
  screen-reader user should play through, and the release artifact built
  by CI.

## Out of scope

- Localization (Hearken has none yet); Fate's Spanish catalog is not ported.
- Sprite art or any visual presentation beyond `lines()`.
- Multiplayer, online services, cloud saves.
- Replacing Genny renders with recordings; a layer is a file and can come
  later with no code change.

## Decisions taken by Orinks

- 2026-10-26: the recommendations stand as written, including the new
  `orinks-games/fastbreak` repository and Kokoro through local-tts.

## Open questions for Orinks

1. Ship the 92 MB voice package in the release zip, or download on first
   run? The plan builds both; the release choice is yours.
2. Voice casting after a listen (task 9 proposes Dana on `bf_emma`, Big M
   on `am_michael`).
