# Hearken M1 Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Hearken workspace whose crates boot a menu-driven, screen-reader-first game: BASS bindings (with callback streams), Prism speech, and an app shell with a screen stack, menus, settings, help and pause, proven by a template game with a headless walkthrough test.

**Architecture:** Three library crates extracted from Freight Fate (FF, `c5d3ef8d`) and Lucky Break (LB, `a101970`): `hearken-bass-sys` (LB's bindings plus FF's music/HLS items plus a new `STREAMPROC` binding), `hearken-speech` (FF's speech stack on `prismer`, minus trucking couplings), and `hearken-app` (LB's shell, queued screen stack and menu, generalised over a `Game` trait; FF's key/input model and single-instance guard). A `template` crate is the first game on it.

**Tech Stack:** Rust 1.98.0 (edition 2021), `prismer` 0.1.3 (static), `sdl2` 0.38 (vendored SDL2 on Windows), BASS 2.4 loaded at run time via `libloading`, `serde`/`serde_json`, `log`/`env_logger`.

**Spec:** `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`. Research notes with verbatim source: `docs/superpowers/notes/m1-bass-sys.md`, `m1-speech.md`, `m1-lb-app.md`, `m1-ff-menus-input.md`.

## Global Constraints

- Toolchain pinned to `1.98.0`, `profile = "minimal"`, components `rustfmt` + `clippy`; `edition = "2021"`, `rust-version = "1.95"`, `resolver = "2"`. Do not move to edition 2024 (breaks `env::set_var` in tests and trips `unsafe_op_in_unsafe_fn`).
- Every crate: `license = "MIT OR Apache-2.0"`, `repository = "https://github.com/Orinks/hearken"`, `authors = ["Joshua Tubbs (Orinks)"]`.
- Source repos are read-only. Extract with `git -C <repo> show <commit>:<path>` so the pinned commit is what gets copied: FF = `C:/Users/joshu/Freight-fate` at `c5d3ef8d`, LB = `C:/Users/joshu/gh-projects/lucky-break` at `a101970`.
- No `FREIGHT_FATE_*`, `LUCKY_BREAK_*`, trucking or casino names anywhere in Hearken code or docs. Engine-level environment variables are `HEARKEN_*`; per-game ones are `<Game::ENV_PREFIX>_*`.
- Nothing may block the game loop. Workers get an explicit, idempotent, time-bounded shutdown. No network, IPC or thread joins in `Drop`.
- Spoken text is for players: no jargon, and silence over redundant speech.
- Code files stay at or under 1000 lines.
- Gates, each separately: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; the tests. `actionlint` after any workflow change.
- Exactly one `cargo test` run in flight on the machine at a time. Background any single command expected to take over 30 seconds. Never pipe a test run to `tail`; redirect to a file and read the result.
- Focused tests while iterating (`cargo test -p <crate> <filter>`); the full `cargo test --workspace --locked` once, in Task 8.
- CI triggers: `pull_request`, `push: tags: ["v*"]`, `workflow_dispatch`. Windows runner only.
- Work on branch `m1-skeleton` in `C:/Users/joshu/gh-projects/hearken`. One commit per task. Every commit message ends with:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`

## Deferred out of M1 (do not build)

- **M1b Input:** rebindable actions (`bindings.rs`, `names.rs`, the rebinding screen), the controller manager (hot-plug, repeat), held-key tracking, the text-entry screen, the timed-message screen.
- **M2 Sound:** `hearken-audio`, menu sounds, volumes in settings, Learn Sounds, and the event channel's priority ladder, pacer and audio duck (they need audio). In M1 `Ctx::say_event` goes straight to the sink's event voice.
- **M4 Playtest:** `--smoke`, `--playtest`, the agent-server MCP.

## Review Focus

1. **A settings file the game cannot read** (hand-edited, truncated by a crash). Expected: the game starts on defaults, the bad file is kept beside it as `settings.json.bad` rather than overwritten, and the player hears one line saying the defaults are in use. Pinned in Task 4 and Task 5.
2. **No screen reader, no SAPI, or `<PREFIX>_NO_SPEECH` set.** Expected: the game starts, menus work, and the transcript still records every line. Pinned in Task 3 (disabled sink never panics) and Task 5 (transcript records without a voice).
3. **A second copy launched while one is running.** Expected: the second copy says the game is already running and exits instead of fighting for the window. Pinned in Task 7.
4. **Screen changes that end or loop** (a screen pops the last screen; a screen whose `enter` keeps pushing). Expected: popping the last screen ends the run; pop-then-push in one handler keeps running; a runaway queue stops after 16 rounds without hanging. Pinned in Task 5.
5. **Menus at their edges** (no rows, F1 on a row without help, a letter no row starts with, Right on a row that cannot adjust). Expected: "No options" for an empty menu, the menu's own help when a row has none, silence for a missed letter or a row that cannot adjust, and never a panic. Pinned in Task 6.

---

### Task 1: Workspace, `hearken-bass-sys`, BASS fetch, CI

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`, `LICENSE-MIT`, `LICENSE-APACHE`, `README.md`
- Create: `crates/hearken-bass-sys/{Cargo.toml,build.rs,src/lib.rs,src/loader.rs,src/fx.rs,src/safe.rs,tests/smoke.rs,vendor/licenses/README.md}`
- Create: `tools/fetch_bass.py`
- Create: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: package `hearken-bass-sys` with `[lib] name = "bass_sys"` (consumers keep writing `use bass_sys::...`). `bass_sys::LIBRARY_PATH_ENV == "HEARKEN_BASS_PATH"`. Everything LB's `bass-sys` exports, plus FF's `BASS_MUSIC_PRESCAN`, `BASS_MUSIC_RAMPS`, `BASS_MUSIC_SINCINTER`, `BASS_MUSIC_STOPBACK`, `BASS_TAG_HLS_EXTINF`, `FnMusicLoad`, `FnMusicFree`, `safe::music_load_mem_shared`, `safe::tags_string`.

- [ ] **Step 1: Branch and root files**

```bash
cd C:/Users/joshu/gh-projects/hearken
git checkout -b m1-skeleton
```

`Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.95"
license = "MIT OR Apache-2.0"
repository = "https://github.com/Orinks/hearken"
authors = ["Joshua Tubbs (Orinks)"]

[workspace.dependencies]
libloading = "0.8"
log = "0.4"
env_logger = "0.11"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"
dirs = "6"
sdl2 = "0.38"
prismer = { version = "0.1.3", features = ["static"] }
windows-sys = { version = "0.59", features = ["Win32_Foundation", "Win32_System_Threading", "Win32_System_Diagnostics_ToolHelp"] }

[profile.release]
opt-level = 3
lto = "thin"
strip = true

[profile.dev]
opt-level = 0

[profile.test]
debug = 0
opt-level = 1

[profile.dev.package."*"]
opt-level = 2
```

`rust-toolchain.toml`:

```toml
[toolchain]
# Keep CI and local builds on the same compiler; bump deliberately.
channel = "1.98.0"
# A user's complete profile requests components stable Windows lacks.
profile = "minimal"
components = ["rustfmt", "clippy"]
```

`.gitignore`:

```
/target
# BASS is proprietary: fetched by tools/fetch_bass.py, never committed.
crates/hearken-bass-sys/vendor/*/*bass*
!crates/hearken-bass-sys/vendor/licenses/
```

Licences: copy the Apache text from any crate in the cargo registry, and write the MIT text with this repository's copyright line.

```bash
cp "$(ls -d ~/.cargo/registry/src/*/serde-1.* | head -1)/LICENSE-APACHE" LICENSE-APACHE
```

`LICENSE-MIT`:

```
MIT License

Copyright (c) 2026 Joshua Tubbs (Orinks)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

`README.md` (Task 8 expands it):

```markdown
# Hearken

Rust crates for audio-first, screen-reader-first games. Work in progress;
see `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`.

Licensed under either of MIT or Apache-2.0, at your option.
```

- [ ] **Step 2: Extract LB's bass-sys**

```bash
LB=C:/Users/joshu/gh-projects/lucky-break; D=crates/hearken-bass-sys
mkdir -p $D/src $D/tests $D/vendor/licenses
for f in Cargo.toml build.rs src/lib.rs src/loader.rs src/fx.rs src/safe.rs tests/smoke.rs; do
  git -C $LB show a101970:crates/bass-sys/$f > $D/$f
done
```

Replace `crates/hearken-bass-sys/Cargo.toml` with:

```toml
[package]
name = "hearken-bass-sys"
description = "Raw FFI bindings to the BASS audio library, loaded at run time"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
build = "build.rs"

[lib]
# Consumers ported from Freight Fate and Lucky Break keep `use bass_sys::...`.
name = "bass_sys"

[dependencies]
libloading.workspace = true
log.workspace = true
```

- [ ] **Step 3: Port what LB dropped from FF**

Section "LB is NOT a strict superset of FF" in `docs/superpowers/notes/m1-bass-sys.md` lists each item with FF line numbers. Read each from FF with `git -C C:/Users/joshu/Freight-fate show c5d3ef8d:crates/bass-sys/<file>` and add it to the matching Hearken file:

1. `src/lib.rs`: the four `BASS_MUSIC_*` constants (FF `lib.rs:231-243`), `BASS_TAG_HLS_EXTINF` (FF `:332-334`), `FnMusicLoad` and `FnMusicFree` (FF `:475-484`), and the test asserts for the music constants (FF `:599-603`), added to the existing constants test.
2. `src/loader.rs`: fields `music_load: FnMusicLoad`, `music_free: FnMusicFree` in `Api`, and `music_load: sym!("BASS_MusicLoad")`, `music_free: sym!("BASS_MusicFree")` in `resolve` (FF `:150-151`, `:279-280`).
3. `src/safe.rs`: `Stream`'s `music: bool` field and `free_handle` (FF `:396-397`, `:432-444`), so `Drop` calls `BASS_MusicFree` for music handles; `music_load_mem_shared` (FF `:488`); `tags_string` (FF `:765`), with `tags_meta` delegating to it as FF does. Keep LB's `is_plugin_file_name` (it excludes `bass_fx`).
4. `tests/smoke.rs`: FF's `tiny_mod()`, `looping_mod()` and the two tests `a_tracker_module_loads_reports_its_length_and_frees_as_music` and `stopback_ends_a_module_that_jumps_back_to_the_start` (FF `tests/smoke.rs:299-382`), using the existing `bass()` guard.

- [ ] **Step 4: One override variable, no game names**

In `src/loader.rs` replace both constants with:

```rust
/// Environment variable naming the BASS library to load: a file, or a
/// directory containing it. A game sets it (for example in
/// `.cargo/config.toml` `[env]`) to wherever it fetched BASS.
pub const LIBRARY_PATH_ENV: &str = "HEARKEN_BASS_PATH";
```

In `search_paths()` change `for var in [LIBRARY_PATH_ENV, LIBRARY_PATH_ENV_LEGACY] {` to `for var in [LIBRARY_PATH_ENV] {`. In `src/lib.rs` change the re-export to `pub use loader::{library_file_name, Api, LoadError, LIBRARY_PATH_ENV};`. Replace the test `both_override_env_vars_are_honoured_in_order` with:

```rust
    #[test]
    fn the_override_env_var_is_searched_first() {
        let _guard = env_lock();
        std::env::set_var(LIBRARY_PATH_ENV, "/tmp/hearken-bass-lib");
        let paths = search_paths();
        std::env::remove_var(LIBRARY_PATH_ENV);
        assert_eq!(
            paths.first(),
            Some(&PathBuf::from("/tmp/hearken-bass-lib")),
            "the override outranks every discovered location"
        );
    }
```

In `build.rs` change the missing-library warning to name the crate and the command:

```rust
        println!(
            "cargo:warning=hearken-bass-sys: {file_name} is missing for \
             {target_os}-{target_arch}. Run `python tools/fetch_bass.py` \
             -- BASS is fetched, not committed. Until then the game runs \
             without audio unless the library is installed system-wide."
        );
```

Rename the test `flag_values_match_pybass` to `flag_values_match_bass_h`. In `src/fx.rs` change the log line to `log::warn!("bass_fx not found; effects that need it are unavailable");`. Rewrite every doc comment listed under "Doc comments only" and "Design-level game specifics" in the notes so it describes the behaviour without naming a game, a language port or a casino. Keep the behaviour itself unchanged.

Gate:

```bash
grep -rniwE 'freight|fate|lucky|casino|python|pybass|sound_lib|sustain|radio|ignition' crates/hearken-bass-sys
```

Expected: no output.

- [ ] **Step 5: The licence note**

`crates/hearken-bass-sys/vendor/licenses/README.md`:

```markdown
# BASS is not in this repository

BASS and bass_fx are proprietary libraries by un4seen Developments. They
are fetched by `tools/fetch_bass.py`, checked against pinned sha256 hashes,
and ignored by git.

BASS is free for non-commercial use. A game sold or otherwise used
commercially needs its own licence from https://www.un4seen.com/. The
licence covers finished products, not components, so Hearken can never
bundle it: each game fetches it for itself.

Point `HEARKEN_BASS_PATH` at a file or folder to use a copy installed
somewhere else.
```

- [ ] **Step 6: The fetch script**

```bash
git -C C:/Users/joshu/gh-projects/lucky-break show a101970:tools/fetch_bass.py > tools/fetch_bass.py
```

Edits:
1. `VENDOR = REPO_ROOT / "crates" / "bass-sys" / "vendor"` becomes `VENDOR = REPO_ROOT / "crates" / "hearken-bass-sys" / "vendor"`.
2. Add a `--dest DIR` option to the `argparse` parser: when given, files go to `DIR/<os>-<arch>/` instead of `VENDOR/<os>-<arch>/`, so a game can fetch into its own folder. Thread it through every place `VENDOR` is used for output, and through the "missing" message.
3. Every `LUCKY_BREAK_BASS_PATH` becomes `HEARKEN_BASS_PATH`; every `uv run python tools/fetch_bass.py` becomes `python tools/fetch_bass.py`.
4. Rewrite the module docstring and the comments that name a game so they describe Hearken; keep the pin tables and the hashing logic unchanged.
5. The `licenses/README.md` existence check points at `VENDOR / "licenses" / "README.md"` (unchanged path shape, new `VENDOR`).

Gate: `grep -niwE 'lucky|casino|freight' tools/fetch_bass.py` gives no output.

- [ ] **Step 7: Run it and the crate's tests**

```bash
python tools/fetch_bass.py
mkdir -p target
cargo test -p hearken-bass-sys > target/t1.log 2>&1; echo exit=$?
```

Expected: `exit=0`; `target/t1.log` shows the smoke tests including both tracker-module tests passing (not skipped: `grep -c "library not loadable" target/t1.log` prints `0`).

- [ ] **Step 8: CI**

`.github/workflows/ci.yml`:

```yaml
name: CI

on:
  pull_request:
  push:
    tags: ["v*"]
  workflow_dispatch:

jobs:
  rust:
    name: Rust (Windows)
    runs-on: windows-latest
    timeout-minutes: 45
    env:
      CARGO_TERM_COLOR: always
    steps:
      - uses: actions/checkout@v6

      - name: Install the pinned toolchain
        run: rustup show

      - name: Keep Defender off the build tree
        shell: pwsh
        run: |
          try {
            Add-MpPreference -ExclusionPath "${{ github.workspace }}", "$env:USERPROFILE\.cargo", "$env:USERPROFILE\.rustup" -ErrorAction Stop
            Write-Host "Defender exclusions added."
          } catch {
            Write-Host "Could not add Defender exclusions ($_); building without them."
          }

      - uses: Swatinem/rust-cache@v2
        with:
          cache-on-failure: true

      - name: Cache BASS
        uses: actions/cache@v4
        with:
          path: crates/hearken-bass-sys/vendor
          key: bass-${{ runner.os }}-${{ hashFiles('tools/fetch_bass.py') }}

      - name: Fetch BASS
        run: python tools/fetch_bass.py

      - name: cargo fmt --check
        run: cargo fmt --all --check

      - name: Clippy
        run: cargo clippy --workspace --all-targets --locked -- -D warnings

      - name: Tests
        run: cargo test --workspace --locked
```

```bash
actionlint
```

Expected: no output.

- [ ] **Step 9: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings
git add -A
git commit -m "feat(bass-sys): hearken-bass-sys from Lucky Break's bindings plus Freight Fate's music and HLS items

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Callback streams in `hearken-bass-sys`

A live voice (M2) is a BASS stream whose samples come from a Rust closure. This task binds `BASS_StreamCreate` with a `STREAMPROC` and `BASS_ChannelGetData`, and wraps them safely.

**Files:**
- Modify: `crates/hearken-bass-sys/src/lib.rs`, `src/loader.rs`, `src/safe.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `safe::lib()`, `BassError`, `Api::stream_free` from Task 1.
- Produces:
  - `bass_sys::BASS_STREAMPROC_END: DWORD`, `bass_sys::BASS_DATA_FLOAT: DWORD`, `bass_sys::STREAMPROC`, `bass_sys::FnStreamCreate`, `bass_sys::FnChannelGetData`
  - `bass_sys::safe::callback_stream(freq: u32, chans: u32, flags: u32, fill: impl FnMut(&mut [f32]) -> usize + Send + 'static) -> Result<CallbackStream, BassError>`
  - `bass_sys::safe::CallbackStream` with `fn handle(&self) -> u32`; dropping it frees the stream
  - `bass_sys::safe::channel_get_data_float(handle: u32, out: &mut [f32]) -> Result<usize, BassError>`

- [ ] **Step 1: Write the failing tests**

Append to `crates/hearken-bass-sys/tests/smoke.rs`:

```rust
#[test]
fn a_callback_stream_is_pulled_and_ends_when_the_callback_says_so() {
    let Some(_g) = bass() else { return };
    let mut left = 1000usize;
    let stream = callback_stream(44100, 1, BASS_STREAM_DECODE, move |out| {
        let n = out.len().min(left);
        out[..n].fill(0.5);
        left -= n;
        n
    })
    .expect("create a decoding callback stream");
    let mut buf = vec![0.0f32; 4096];
    let got = channel_get_data_float(stream.handle(), &mut buf).expect("first read");
    assert_eq!(got, 1000, "exactly what the callback wrote");
    assert!(buf[..got].iter().all(|&s| s == 0.5));
    let after = channel_get_data_float(stream.handle(), &mut buf);
    assert!(
        matches!(after, Ok(0)) || after.as_ref().is_err_and(|e| e.is(BASS_ERROR_ENDED)),
        "a stream whose callback ended gives nothing more, got {after:?}"
    );
}

#[test]
fn a_panicking_callback_ends_the_stream_instead_of_unwinding_into_bass() {
    let Some(_g) = bass() else { return };
    let stream = callback_stream(44100, 1, BASS_STREAM_DECODE, |_out| panic!("boom"))
        .expect("create a decoding callback stream");
    let mut buf = vec![1.0f32; 512];
    let got = channel_get_data_float(stream.handle(), &mut buf);
    assert!(
        matches!(got, Ok(0)) || got.as_ref().is_err_and(|e| e.is(BASS_ERROR_ENDED)),
        "the stream ends quietly, got {got:?}"
    );
}

#[test]
fn dropping_a_callback_stream_frees_it() {
    let Some(_g) = bass() else { return };
    let handle = {
        let stream = callback_stream(44100, 2, BASS_STREAM_DECODE, |out| out.len())
            .expect("create");
        stream.handle()
    };
    let mut buf = vec![0.0f32; 64];
    let after = channel_get_data_float(handle, &mut buf);
    assert!(after.as_ref().is_err_and(|e| e.is(BASS_ERROR_HANDLE)), "got {after:?}");
}
```

Add to the unit tests in `src/lib.rs`:

```rust
    #[test]
    fn stream_callback_constants_match_bass_h() {
        assert_eq!(BASS_STREAMPROC_END, 0x8000_0000);
        assert_eq!(BASS_DATA_FLOAT, 0x4000_0000);
        assert_eq!(BASS_STREAM_DECODE, 0x20_0000);
        assert_eq!(BASS_SAMPLE_FLOAT, 256);
    }
```

- [ ] **Step 2: Run to confirm they fail**

`cargo test -p hearken-bass-sys callback > target/t2.log 2>&1; echo exit=$?` — expected: compile errors naming `callback_stream`, `channel_get_data_float`, `BASS_STREAMPROC_END`.

- [ ] **Step 3: Declarations**

In `src/lib.rs`, beside the other stream flags and callback types:

```rust
/// Returned by a `STREAMPROC` (OR-ed with the byte count) when the stream has
/// ended; BASS plays what was written and then stops the channel.
pub const BASS_STREAMPROC_END: DWORD = 0x8000_0000;
/// `BASS_ChannelGetData` flag: `length` is in bytes of 32-bit float samples.
pub const BASS_DATA_FLOAT: DWORD = 0x4000_0000;

/// `DWORD CALLBACK STREAMPROC(HSTREAM handle, void *buffer, DWORD length, void *user)`.
///
/// Called on a BASS thread to fill up to `length` bytes of `buffer`. It must
/// be quick and must not block on anything the mixer may be waiting for.
/// Returns the bytes written, OR-ed with `BASS_STREAMPROC_END` at the end.
pub type STREAMPROC = unsafe extern "system" fn(
    handle: HSTREAM,
    buffer: *mut c_void,
    length: DWORD,
    user: *mut c_void,
) -> DWORD;

/// `HSTREAM BASS_StreamCreate(DWORD freq, DWORD chans, DWORD flags, STREAMPROC *proc, void *user)`
pub type FnStreamCreate = unsafe extern "system" fn(
    freq: DWORD,
    chans: DWORD,
    flags: DWORD,
    proc_: Option<STREAMPROC>,
    user: *mut c_void,
) -> HSTREAM;

/// `DWORD BASS_ChannelGetData(DWORD handle, void *buffer, DWORD length)`
pub type FnChannelGetData =
    unsafe extern "system" fn(handle: DWORD, buffer: *mut c_void, length: DWORD) -> DWORD;
```

In `src/loader.rs`, add to `Api` beside `stream_free`:

```rust
    pub stream_create: FnStreamCreate,
    pub channel_get_data: FnChannelGetData,
```

and to `resolve`:

```rust
            stream_create: sym!("BASS_StreamCreate"),
            channel_get_data: sym!("BASS_ChannelGetData"),
```

- [ ] **Step 4: The safe wrapper**

Append to `src/safe.rs`:

```rust
// --- callback streams ----------------------------------------------------------

/// The closure a callback stream pulls from: fill `out` (interleaved 32-bit
/// float frames) and return how many samples were written. Fewer than
/// `out.len()` ends the stream once those samples have played.
type FillFn = dyn FnMut(&mut [f32]) -> usize + Send;

/// Shared between the stream and BASS's thread. `None` once the stream has
/// been dropped, so a call BASS makes while it is still freeing the stream
/// finds nothing to run.
type FillSlot = std::sync::Mutex<Option<Box<FillFn>>>;

/// A stream whose samples come from a Rust closure (`BASS_StreamCreate` with a
/// `STREAMPROC`), always 32-bit float. Dropping it frees the stream.
pub struct CallbackStream {
    handle: HSTREAM,
    slot: *const FillSlot,
}

// SAFETY: the slot is a `Mutex` around a `Send` closure, and the raw pointer
// is only dereferenced as `&FillSlot`.
unsafe impl Send for CallbackStream {}

impl CallbackStream {
    pub fn handle(&self) -> HSTREAM {
        self.handle
    }
}

unsafe extern "system" fn fill_trampoline(
    _handle: HSTREAM,
    buffer: *mut c_void,
    length: DWORD,
    user: *mut c_void,
) -> DWORD {
    // SAFETY: `user` is the `FillSlot` leaked by `callback_stream`, which is
    // never freed (see `Drop`), so it outlives every call BASS makes.
    let slot = unsafe { &*(user as *const FillSlot) };
    let samples = length as usize / std::mem::size_of::<f32>();
    // SAFETY: BASS hands a writable buffer of `length` bytes, aligned for
    // the float format the stream was created with.
    let out = unsafe { std::slice::from_raw_parts_mut(buffer.cast::<f32>(), samples) };
    let Ok(mut guard) = slot.lock() else {
        return BASS_STREAMPROC_END;
    };
    let Some(fill) = guard.as_mut() else {
        return BASS_STREAMPROC_END;
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(out))) {
        Ok(written) if written >= samples => length,
        Ok(written) => (written * std::mem::size_of::<f32>()) as DWORD | BASS_STREAMPROC_END,
        Err(_) => {
            log::error!("bass: a stream callback panicked; ending the stream");
            *guard = None;
            BASS_STREAMPROC_END
        }
    }
}

/// Create a float stream fed by `fill`. `flags` may add `BASS_SAMPLE_3D`,
/// `BASS_STREAM_DECODE` and the like; `BASS_SAMPLE_FLOAT` is always set.
pub fn callback_stream(
    freq: u32,
    chans: u32,
    flags: u32,
    fill: impl FnMut(&mut [f32]) -> usize + Send + 'static,
) -> Result<CallbackStream, BassError> {
    let a = lib()?;
    let fill: Box<FillFn> = Box::new(fill);
    // ponytail: the slot (a mutex and a pointer) is leaked per stream, because
    // BASS may free a stream asynchronously and call the STREAMPROC once more;
    // free it from a BASS_SYNC_FREE sync if games ever create thousands.
    let slot: *const FillSlot = Box::into_raw(Box::new(std::sync::Mutex::new(Some(fill))));
    // SAFETY: `fill_trampoline` matches `STREAMPROC`; `slot` is never freed.
    let handle = unsafe {
        (a.stream_create)(
            freq,
            chans,
            flags | BASS_SAMPLE_FLOAT,
            Some(fill_trampoline),
            slot as *mut c_void,
        )
    };
    if handle == 0 {
        return Err(BassError::last());
    }
    Ok(CallbackStream { handle, slot })
}

impl Drop for CallbackStream {
    fn drop(&mut self) {
        if let Ok(a) = lib() {
            // SAFETY: our own handle, freed once.
            unsafe { (a.stream_free)(self.handle) };
        }
        // SAFETY: the slot is leaked and valid; emptying it drops the closure
        // (and whatever graph it owns) while the slot itself stays readable.
        let slot = unsafe { &*self.slot };
        if let Ok(mut guard) = slot.lock() {
            *guard = None;
        }
    }
}

/// `BASS_ChannelGetData(handle, out, bytes | BASS_DATA_FLOAT)` on a decoding
/// channel: fills `out` with float samples and returns how many were written.
pub fn channel_get_data_float(handle: u32, out: &mut [f32]) -> Result<usize, BassError> {
    let a = lib()?;
    let bytes = (out.len() * std::mem::size_of::<f32>()) as DWORD;
    // SAFETY: `out` is a writable buffer of exactly `bytes` bytes.
    let got = unsafe { (a.channel_get_data)(handle, out.as_mut_ptr().cast(), bytes | BASS_DATA_FLOAT) };
    if got == u32::MAX {
        Err(BassError::last())
    } else {
        Ok(got as usize / std::mem::size_of::<f32>())
    }
}
```

Use the names the file already imports for `HSTREAM`, `DWORD`, `c_void` and `BASS_SAMPLE_FLOAT`; add any missing to its `use` lines.

- [ ] **Step 5: Run the tests**

`cargo test -p hearken-bass-sys > target/t2.log 2>&1; echo exit=$?` — expected `exit=0`, the three new smoke tests and the constants test pass (not skipped).

- [ ] **Step 6: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings
git add -A
git commit -m "feat(bass-sys): callback streams fed by a Rust closure

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: `hearken-speech`

**Files:**
- Create: `crates/hearken-speech/{Cargo.toml,build.rs}`
- Create: `crates/hearken-speech/src/{lib.rs,backend.rs,live.rs,threaded.rs,capture.rs,fakes.rs,spoken.rs}`
- Create: `crates/hearken-speech/src/backend/recovery_tests.rs`, `src/threaded/{recovery_tests.rs,tests.rs}`
- Create: `crates/hearken-speech/tests/{speech.rs,speech_live.rs}`
- Modify: root `Cargo.toml` (`[workspace.dependencies]`)

**Interfaces:**
- Produces (all re-exported from the crate root):
  - `trait SpeechSink` exactly as FF's (22 methods, no `Send` supertrait; verbatim in `m1-speech.md` §2a)
  - `struct SpeechEnv { pub no_speech: String, pub backend: String, pub stream_transcript: String }` with `fn with_prefix(prefix: &str) -> SpeechEnv` and `fn disabled(&self) -> bool`
  - `ThreadedSpeech::spawn(env: SpeechEnv) -> ThreadedSpeech`; `Speech::new(env: &SpeechEnv)`
  - `struct SpeechSettings { separate_event_voice: bool, event_backend: String, rate: f64, pitch: f64, volume: f64, voice: String, braille_only: bool }` (serde, `#[serde(default)]`), `Default` = `true, "SAPI", 0.5, 0.5, 1.0, "", false`
  - `fn apply_speech_settings(sink: &mut dyn SpeechSink, settings: &mut SpeechSettings)`
  - `CaptureSpeech`, `NullSpeech`, `SpeechChannel`, `SpokenEntry`, `CaptureProfile`, `ConfigureCall`, and new `#[derive(Clone)] pub struct SharedCapture(pub Rc<RefCell<CaptureSpeech>>)` implementing `SpeechSink`
  - `SpokenMessage`, `terse_silent`
  - Build metadata: `links = "hearken_speech"`, publishing `DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS` to direct dependents

- [ ] **Step 1: Extract**

```bash
FF=C:/Users/joshu/Freight-fate; D=crates/hearken-speech; S=crates/freight-fate/src
mkdir -p $D/src/backend $D/src/threaded $D/tests
git -C $FF show c5d3ef8d:$S/speech.rs > $D/src/lib.rs
for f in backend live threaded capture fakes; do git -C $FF show c5d3ef8d:$S/speech/$f.rs > $D/src/$f.rs; done
git -C $FF show c5d3ef8d:$S/speech/backend/recovery_tests.rs > $D/src/backend/recovery_tests.rs
git -C $FF show c5d3ef8d:$S/speech/threaded/recovery_tests.rs > $D/src/threaded/recovery_tests.rs
git -C $FF show c5d3ef8d:$S/speech/threaded/tests.rs > $D/src/threaded/tests.rs
git -C $FF show c5d3ef8d:crates/ff-core/src/speech_text.rs | sed -n 68,177p > $D/src/spoken.rs
git -C $FF show c5d3ef8d:crates/freight-fate/tests/it/speech.rs > $D/tests/speech.rs
git -C $FF show c5d3ef8d:crates/freight-fate/tests/it/speech_live.rs > $D/tests/speech_live.rs
```

Add to root `Cargo.toml` `[workspace.dependencies]`: `hearken-speech = { path = "crates/hearken-speech" }`.

`crates/hearken-speech/Cargo.toml`:

```toml
[package]
name = "hearken-speech"
description = "Screen reader, braille and text-to-speech output for audio games, through Prism"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
# Publishes Prism's delay-load list to games (see build.rs).
links = "hearken_speech"
build = "build.rs"

[dependencies]
prismer.workspace = true
log.workspace = true
serde.workspace = true

[target.'cfg(windows)'.dependencies]
windows-sys.workspace = true

[dev-dependencies]
serde_json.workspace = true
```

- [ ] **Step 2: build.rs**

```rust
//! Pass Prism's delay-load list on to games, and link Prism's system
//! libraries on macOS and Linux.
//!
//! Cargo hands a `links` crate's metadata to its direct dependents only.
//! prismer publishes the Windows screen-reader DLLs that must be
//! delay-loaded; a game depends on this crate, not on prismer, so the list
//! is re-published here as `DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS`. The game's
//! own build.rs turns it into `/DELAYLOAD` link arguments: a link flag only
//! takes effect on the final link, which a library cannot do for a game.

use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_PRISMER_DELAY_LOAD_DLLS");
    if let Ok(dlls) = env::var("DEP_PRISMER_DELAY_LOAD_DLLS") {
        println!("cargo:delay_load_dlls={dlls}");
    }
    link_prism_system_libraries();
}
```

Then append FF's `link_prism_system_libraries` and `link_libstdcxx_statically` verbatim from `git -C C:/Users/joshu/Freight-fate show c5d3ef8d:crates/freight-fate/build.rs` (lines 65-149, both functions with their doc comments), adding `use std::path::PathBuf;` if they need it.

- [ ] **Step 3: Cut the couplings**

Apply the cut list at the end of `docs/superpowers/notes/m1-speech.md`:

1. `src/lib.rs`: the module declarations become `pub mod backend; pub mod capture; pub mod fakes; pub mod live; pub mod spoken; pub mod threaded;`. Delete `use ff_core::settings::Settings;` and the `pub use ff_core::speech_pacing::...` line. Delete `NO_SPEECH_ENV` and `SPEECH_BACKEND_ENV`. Add `pub use spoken::{terse_silent, SpokenMessage};` and `pub use capture::SharedCapture;` beside the existing re-exports. Add:

```rust
/// The environment overrides a game's speech reads, named after the game's
/// prefix so two games on one machine never share them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechEnv {
    /// Any non-empty value: start with speech off, loading no voice.
    pub no_speech: String,
    /// A backend name tried first, for testing one voice.
    pub backend: String,
    /// Any non-empty value: print every line a voice spoke to stdout.
    pub stream_transcript: String,
}

impl SpeechEnv {
    pub fn with_prefix(prefix: &str) -> SpeechEnv {
        SpeechEnv {
            no_speech: format!("{prefix}_NO_SPEECH"),
            backend: format!("{prefix}_SPEECH_BACKEND"),
            stream_transcript: format!("{prefix}_STREAM_TRANSCRIPT"),
        }
    }

    /// Whether `no_speech` is set to something non-empty.
    pub fn disabled(&self) -> bool {
        std::env::var_os(&self.no_speech).is_some_and(|v| !v.is_empty())
    }
}

/// The speech settings a game saves: which voices speak and how.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpeechSettings {
    /// Speak gameplay events through a separate software voice, so the
    /// screen reader cannot talk over them.
    pub separate_event_voice: bool,
    /// The voice that speaks events when `separate_event_voice` is on.
    pub event_backend: String,
    /// 0.0 to 1.0.
    pub rate: f64,
    /// 0.0 to 1.0.
    pub pitch: f64,
    /// 0.0 to 1.0.
    pub volume: f64,
    /// An installed voice's name; empty for the voice's default.
    pub voice: String,
    /// Send every line to the braille display and speak nothing.
    pub braille_only: bool,
}

impl Default for SpeechSettings {
    fn default() -> Self {
        SpeechSettings {
            separate_event_voice: true,
            event_backend: EVENT_BACKEND.to_string(),
            rate: 0.5,
            pitch: 0.5,
            volume: 1.0,
            voice: String::new(),
            braille_only: false,
        }
    }
}
```

   Rewrite `apply_speech_settings` with the same body but the new types: `settings.sapi_events` becomes `settings.separate_event_voice`, `speech_voice` becomes `voice`, `speech_rate`/`speech_pitch`/`speech_volume` become `rate`/`pitch`/`volume`, and the signature becomes `pub fn apply_speech_settings(sink: &mut dyn SpeechSink, settings: &mut SpeechSettings)`.
2. `src/live.rs`: `Speech::new()` and `Speech::new_after_wedge()` take `env: &SpeechEnv` and store a clone. Each read of `super::NO_SPEECH_ENV` reads `env.no_speech` (keep FF's non-empty test), `SPEECH_BACKEND_ENV` reads `env.backend`, and the literal `"FREIGHT_FATE_STREAM_TRANSCRIPT"` reads `env.stream_transcript`. The log line at FF `live.rs:90` names the variable from `env.no_speech`.
3. `src/threaded.rs`: `spawn()` becomes `pub fn spawn(env: SpeechEnv) -> Self`, whose factory moves `env` in and calls `Speech::new(&env)` or `Speech::new_after_wedge(&env)`, keeping FF's `after_wedge` switch.
4. `src/capture.rs`: `use ff_core::speech_text::SpokenMessage;` becomes `use crate::spoken::SpokenMessage;`. Append:

```rust
/// A `CaptureSpeech` the test keeps a handle on after handing the sink to a
/// game: the app owns the `Box<dyn SpeechSink>`, the test reads the lines.
#[derive(Clone, Debug, Default)]
pub struct SharedCapture(pub std::rc::Rc<std::cell::RefCell<CaptureSpeech>>);

impl SpeechSink for SharedCapture {
    fn say(&mut self, text: &str, interrupt: bool) { self.0.borrow_mut().say(text, interrupt) }
    fn say_event(&mut self, text: &str, interrupt: bool) { self.0.borrow_mut().say_event(text, interrupt) }
    fn stop_main(&mut self) { self.0.borrow_mut().stop_main() }
    fn stop_event(&mut self) { self.0.borrow_mut().stop_event() }
    fn stop(&mut self) { self.0.borrow_mut().stop() }
    fn poll(&mut self, dt: f64) { self.0.borrow_mut().poll(dt) }
    fn request_refresh(&mut self) { self.0.borrow_mut().request_refresh() }
    fn available(&self) -> bool { self.0.borrow().available() }
    fn backend_name(&self) -> String { self.0.borrow().backend_name() }
    fn has_separate_event_voice(&self) -> bool { self.0.borrow().has_separate_event_voice() }
    fn event_backend_name(&self) -> String { self.0.borrow().event_backend_name() }
    fn supports_rate(&self) -> bool { self.0.borrow().supports_rate() }
    fn supports_pitch(&self) -> bool { self.0.borrow().supports_pitch() }
    fn supports_volume(&self) -> bool { self.0.borrow().supports_volume() }
    fn event_supports_rate(&self) -> bool { self.0.borrow().event_supports_rate() }
    fn event_backend_options(&self) -> Vec<String> { self.0.borrow().event_backend_options() }
    fn select_event_backend(&mut self, name: Option<&str>) { self.0.borrow_mut().select_event_backend(name) }
    fn voice_names(&self) -> Vec<String> { self.0.borrow().voice_names() }
    fn configure(&mut self, rate: Option<f64>, pitch: Option<f64>, volume: Option<f64>, voice: Option<&str>) {
        self.0.borrow_mut().configure(rate, pitch, volume, voice)
    }
    fn set_braille_only(&mut self, on: bool) { self.0.borrow_mut().set_braille_only(on) }
    fn supports_braille(&self) -> bool { self.0.borrow().supports_braille() }
    fn say_adjustment_preview(&mut self, setting: &str, text: &str, interrupt: bool) -> bool {
        self.0.borrow_mut().say_adjustment_preview(setting, text, interrupt)
    }
    fn refresh(&mut self, announce: bool) -> bool { self.0.borrow_mut().refresh(announce) }
    fn shutdown(&mut self) { self.0.borrow_mut().shutdown() }
}
```

   (`cargo fmt` will reflow it.)
5. `src/spoken.rs`: add `use std::fmt;` and whatever else the extracted lines need at the top; it must compile with no `ff_core` import.
6. Every remaining `crate::speech::` path becomes `crate::`; every `super::` that pointed at `speech.rs` becomes `crate::`.
7. Rewrite doc comments that name a game, a place, a tester, a date, or the Python original, so they describe behaviour. Keep FF's Windows knowledge (`"UIA"`, `narrator.exe`, SAPI, OneCore) unchanged.

Gate:

```bash
grep -rniwE 'freight|fate|lucky|casino|truck|trucking|driving|road|python|pygame|ff_core' crates/hearken-speech
```

Expected: no output.

- [ ] **Step 4: Port the tests**

In `tests/speech.rs`, keep the tests listed under "Tests exercising the speech stack" in `m1-speech.md` §7 that exercise backends, capture, fakes, threading and `apply_speech_settings`. Delete tests that need FF's `GameContext`, `TestApp` or `ff_core`. Change imports from `freight_fate::speech::...` to `hearken_speech::...`, and port the four `apply_speech_settings` tests to `SpeechSettings` field names. In `tests/speech_live.rs` replace every `FREIGHT_FATE_*` variable with the matching `SpeechEnv::with_prefix("HEARKEN_TEST")` field and pass that env to `Speech::new`.

Add to `tests/speech.rs`:

```rust
#[test]
fn env_names_follow_the_game_prefix() {
    let env = hearken_speech::SpeechEnv::with_prefix("HELLO_GAME");
    assert_eq!(env.no_speech, "HELLO_GAME_NO_SPEECH");
    assert_eq!(env.backend, "HELLO_GAME_SPEECH_BACKEND");
    assert_eq!(env.stream_transcript, "HELLO_GAME_STREAM_TRANSCRIPT");
}

#[test]
fn a_disabled_live_sink_starts_and_takes_every_call_quietly() {
    let env = hearken_speech::SpeechEnv::with_prefix("HEARKEN_TEST_DISABLED");
    std::env::set_var(&env.no_speech, "1");
    let mut speech = hearken_speech::Speech::new(&env);
    assert!(!speech.available());
    speech.say("hello", true);
    speech.say_event("event", false);
    speech.poll(0.016);
    speech.stop();
    speech.shutdown();
    speech.shutdown();
    std::env::remove_var(&env.no_speech);
}

#[test]
fn speech_settings_default_and_partial_json() {
    let s: hearken_speech::SpeechSettings = serde_json::from_str(r#"{"rate": 0.7}"#).unwrap();
    assert_eq!(s.rate, 0.7);
    assert_eq!(s.pitch, 0.5);
    assert!(s.separate_event_voice);
    assert_eq!(s.event_backend, "SAPI");
}

#[test]
fn a_shared_capture_is_read_through_its_handle() {
    use hearken_speech::{CaptureSpeech, SharedCapture, SpeechSink};
    let handle = std::rc::Rc::new(std::cell::RefCell::new(CaptureSpeech::new()));
    let mut sink: Box<dyn SpeechSink> = Box::new(SharedCapture(handle.clone()));
    sink.say("one", true);
    sink.say_event("two", false);
    assert_eq!(handle.borrow().main_lines(), vec!["one".to_string()]);
    assert_eq!(handle.borrow().event_lines(), vec!["two".to_string()]);
}
```

- [ ] **Step 5: Run**

`cargo test -p hearken-speech > target/t3.log 2>&1; echo exit=$?`. The first build compiles Prism from source (CMake and a C++23 compiler are required), so run it in the background. Expected `exit=0`.

- [ ] **Step 6: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings
git add -A
git commit -m "feat(speech): hearken-speech from Freight Fate's speech stack on prismer

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: `hearken-app` foundations: input types, paths, settings

**Files:**
- Create: `crates/hearken-app/{Cargo.toml}`, `src/{lib.rs,input.rs,text.rs,paths.rs,settings.rs}`
- Test: `crates/hearken-app/tests/foundations.rs`
- Modify: root `Cargo.toml` (`hearken-app = { path = "crates/hearken-app" }` in `[workspace.dependencies]`)

**Interfaces:**
- Consumes: `hearken_speech::SpeechSettings` (Task 3).
- Produces:
  - `hearken_app::{Key, Mods, InputEvent, ControllerButton, ControllerAxis}`: FF's definitions and impls verbatim (`m1-ff-menus-input.md` §1 "Key", "Mods", "InputEvent"; §5 `ControllerButton`, `ControllerAxis`). `Key::from_char(ch) -> Key`; `InputEvent::{key, key_text, key_mods, key_up, typed, button, button_up, axis, is_controller_event, key_down, key_repeat}`
  - `hearken_app::text::{end_sentence, spoken_char}` (FF `states/base.rs:50-75`)
  - `hearken_app::Paths { pub data_dir: PathBuf }` with `discover(dir_name: &str, env_prefix: &str) -> Paths`, `at(dir: impl Into<PathBuf>) -> Paths`, `settings_file() -> PathBuf`, `log_file() -> PathBuf`
  - `hearken_app::Settings<GS> { pub speech: SpeechSettings, pub game: GS }` with `load(&Paths) -> (Settings<GS>, Option<String>)`, `save(&self, &Paths) -> io::Result<()>`
  - `hearken_app::settings::UNREADABLE_NOTICE: &str`

- [ ] **Step 1: Write the failing tests**

`crates/hearken-app/tests/foundations.rs`:

```rust
use hearken_app::settings::UNREADABLE_NOTICE;
use hearken_app::{InputEvent, Key, Mods, Paths, Settings};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
struct GameBits {
    difficulty: u8,
}

fn temp_paths() -> (Paths, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (Paths::at(dir.path()), dir)
}

#[test]
fn typed_characters_land_on_their_keys() {
    assert_eq!(Key::from_char('Q'), Key::Q);
    assert_eq!(Key::from_char('5'), Key::Num5);
    assert!(matches!(Key::from_char('$'), Key::Other(_)));
    assert_eq!(
        InputEvent::typed('a'),
        InputEvent::KeyDown { key: Key::A, mods: Mods::NONE, text: Some('a'), repeat: false }
    );
    assert!(!InputEvent::key(Key::Down).key_repeat());
}

#[test]
fn spoken_text_helpers() {
    assert_eq!(hearken_app::text::end_sentence("Sleep"), "Sleep.");
    assert_eq!(hearken_app::text::end_sentence("Done."), "Done.");
    assert_eq!(hearken_app::text::spoken_char(' '), "space");
    assert_eq!(hearken_app::text::spoken_char('J'), "cap j");
}

#[test]
fn the_data_dir_override_wins_and_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let wanted = dir.path().join("elsewhere");
    std::env::set_var("HEARKEN_TEST_FOUNDATIONS_DATA_DIR", &wanted);
    let paths = Paths::discover("HearkenFoundationsTest", "HEARKEN_TEST_FOUNDATIONS");
    std::env::remove_var("HEARKEN_TEST_FOUNDATIONS_DATA_DIR");
    assert_eq!(paths.data_dir, wanted);
    assert!(wanted.is_dir());
    assert_eq!(paths.settings_file(), wanted.join("settings.json"));
    assert_eq!(paths.log_file(), wanted.join("session.log"));
}

#[test]
fn missing_settings_load_as_defaults_without_a_notice() {
    let (paths, _dir) = temp_paths();
    let (s, notice) = Settings::<GameBits>::load(&paths);
    assert_eq!(s, Settings::default());
    assert_eq!(notice, None);
}

#[test]
fn settings_round_trip() {
    let (paths, _dir) = temp_paths();
    let mut s = Settings::<GameBits>::default();
    s.speech.rate = 0.8;
    s.game.difficulty = 3;
    s.save(&paths).unwrap();
    assert_eq!(Settings::<GameBits>::load(&paths), (s, None));
}

#[test]
fn partial_and_unknown_fields_load() {
    let (paths, _dir) = temp_paths();
    std::fs::write(paths.settings_file(), r#"{"speech": {"rate": 0.3}, "extra": 1}"#).unwrap();
    let (s, notice) = Settings::<GameBits>::load(&paths);
    assert_eq!(s.speech.rate, 0.3);
    assert_eq!(s.speech.pitch, 0.5);
    assert_eq!(s.game, GameBits::default());
    assert_eq!(notice, None);
}

#[test]
fn an_unreadable_settings_file_is_kept_aside_and_reported() {
    let (paths, _dir) = temp_paths();
    std::fs::write(paths.settings_file(), "{ not json").unwrap();
    let (s, notice) = Settings::<GameBits>::load(&paths);
    assert_eq!(s, Settings::default());
    assert_eq!(notice.as_deref(), Some(UNREADABLE_NOTICE));
    assert!(!paths.settings_file().exists(), "the bad file is moved, not left to be overwritten");
    let bad = paths.data_dir.join("settings.json.bad");
    assert_eq!(std::fs::read_to_string(bad).unwrap(), "{ not json");
}
```

- [ ] **Step 2: Run to confirm they fail**

`cargo test -p hearken-app --test foundations > target/t4.log 2>&1; echo exit=$?` — expected: compile errors (crate does not exist yet).

- [ ] **Step 3: The crate**

`crates/hearken-app/Cargo.toml`:

```toml
[package]
name = "hearken-app"
description = "The shell of an audio-first game: screens, menus, input, settings, speech"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true

[dependencies]
hearken-speech.workspace = true
log.workspace = true
serde.workspace = true
serde_json.workspace = true
dirs.workspace = true
tempfile.workspace = true

```

`src/lib.rs`:

```rust
//! The shell of an audio-first, screen-reader-first game.

pub mod input;
pub mod paths;
pub mod settings;
pub mod text;

pub use input::{ControllerAxis, ControllerButton, InputEvent, Key, Mods};
pub use paths::Paths;
pub use settings::Settings;
```

`src/input.rs`: a module doc line (`//! Keyboard and controller input as screens see it.`), then FF's `Key` (with `impl Key { from_char }`), `Mods` (with its consts), `InputEvent` (with its whole `impl`) copied verbatim from `m1-ff-menus-input.md` §1, and `ControllerButton` and `ControllerAxis` copied verbatim from §5. Rewrite the doc comments that mention pygame to describe the behaviour.

`src/text.rs`: `//! Small helpers for spoken text.` then FF's `end_sentence` and `spoken_char` verbatim from §1.

`src/paths.rs`:

```rust
//! Where a game keeps its settings and its session log.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    /// `<env_prefix>_DATA_DIR` when it is set and non-empty, otherwise
    /// `dir_name` under the platform's local data folder. Created if missing.
    pub fn discover(dir_name: &str, env_prefix: &str) -> Paths {
        let dir = std::env::var_os(format!("{env_prefix}_DATA_DIR"))
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|d| d.join(dir_name)))
            .unwrap_or_else(|| PathBuf::from(dir_name));
        Paths::at(dir)
    }

    /// A data folder at `dir`, created if missing. A folder that cannot be
    /// created is logged, not fatal: saving later reports its own error.
    pub fn at(dir: impl Into<PathBuf>) -> Paths {
        let data_dir = dir.into();
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            log::warn!("could not create {}: {e}", data_dir.display());
        }
        Paths { data_dir }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("session.log")
    }
}
```

`src/settings.rs`:

```rust
//! The saved settings: the engine's own, and the game's, in one
//! `settings.json`.

use std::io;

use hearken_speech::SpeechSettings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// What the player hears when their settings file could not be read.
pub const UNREADABLE_NOTICE: &str =
    "Your settings file could not be read, so the game is using its default settings.";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(bound(
    serialize = "GS: Serialize",
    deserialize = "GS: DeserializeOwned + Default"
))]
pub struct Settings<GS> {
    #[serde(default)]
    pub speech: SpeechSettings,
    #[serde(default)]
    pub game: GS,
}

impl<GS: Serialize + DeserializeOwned + Default> Settings<GS> {
    /// The saved settings. A missing file gives the defaults quietly. A file
    /// that cannot be read or parsed gives the defaults too, is moved aside
    /// to `settings.json.bad` so the next save cannot destroy it, and returns
    /// a notice for the player.
    pub fn load(paths: &Paths) -> (Self, Option<String>) {
        let file = paths.settings_file();
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => {
                log::warn!("could not read {}: {e}; using defaults", file.display());
                return (Self::default(), Some(UNREADABLE_NOTICE.to_string()));
            }
        };
        match serde_json::from_str(&text) {
            Ok(settings) => (settings, None),
            Err(e) => {
                log::warn!("could not parse {}: {e}; using defaults", file.display());
                let bad = file.with_extension("json.bad");
                if let Err(e) = std::fs::rename(&file, &bad) {
                    log::warn!("could not keep the bad settings file aside: {e}");
                }
                (Self::default(), Some(UNREADABLE_NOTICE.to_string()))
            }
        }
    }
}

impl<GS: Serialize> Settings<GS> {
    /// Write the settings as pretty JSON, through a temporary file, so a
    /// crash mid-write never leaves half a file behind.
    pub fn save(&self, paths: &Paths) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let file = paths.settings_file();
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &file)
    }
}
```

- [ ] **Step 4: Run the tests**

`cargo test -p hearken-app --test foundations > target/t4.log 2>&1; echo exit=$?` — expected `exit=0`, 7 passed.

- [ ] **Step 5: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings
git add -A
git commit -m "feat(app): input types, data folder and settings with a safe fallback

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: `hearken-app` core: `Game`, `Ctx`, `State`, the stack and the loop

**Files:**
- Create: `crates/hearken-app/src/{game.rs,ctx.rs,state.rs,app.rs,shell.rs,testing.rs}`
- Modify: `crates/hearken-app/src/lib.rs`
- Test: `crates/hearken-app/tests/common/mod.rs`, `crates/hearken-app/tests/app.rs`

**Interfaces:**
- Consumes: `Paths`, `Settings`, `InputEvent`, `Key` (Task 4); `SpeechSink`, `SharedCapture`, `CaptureSpeech`, `apply_speech_settings` (Task 3).
- Produces:
  - `trait Game: Sized + 'static { const TITLE: &'static str; const DATA_DIR: &'static str; const ENV_PREFIX: &'static str; type Event; type Data; type Settings: Serialize + DeserializeOwned + Default + Clone; fn pump(ctx: &mut Ctx<Self>, dt: f64) -> Vec<Self::Event>; fn observe(ctx: &mut Ctx<Self>, event: &Self::Event); fn on_exit(ctx: &mut Ctx<Self>); }`
  - `struct Ctx<G: Game>` with pub fields `speech, settings, paths, game, running, transcript, notices, frame, clock` and methods `new, say, say_queued, say_event, stop_speech, apply_speech_settings, push, pop, replace, reset_to, quit`; consts `TRANSCRIPT_TARGET = "hearken.transcript"`, `TRANSCRIPT_CAP = 2000`
  - `trait State<G: Game>` (hooks `name, enter, exit, resume, handle_input, on_event, update, lines, help(&self, &Ctx<G>)`), `enum StackOp<G>`
  - `struct App<G: Game> { pub ctx: Ctx<G>, .. }` with `new(Ctx<G>, Box<dyn State<G>>)`, `step(Vec<InputEvent>, f64)`, `run(&mut dyn Shell)`, `top_name`, `depth`, `top_lines`
  - `trait Shell { fn poll(&mut self) -> Vec<InputEvent>; fn render(&mut self, lines: &[String]); fn frame(&mut self) -> f64; }`, `HeadlessShell`, `FRAME_SECONDS`
  - `struct Headless<G: Game> { pub app: App<G>, pub speech: Rc<RefCell<CaptureSpeech>>, .. }` with `new(G::Data, impl State<G> + 'static)`, `press(Key)`, `send(InputEvent)`, `frame(f64)`, `spoken() -> Vec<String>`, `clear()`

- [ ] **Step 1: The shared test game**

`crates/hearken-app/tests/common/mod.rs`:

```rust
#![allow(dead_code)]

use hearken_app::{Ctx, Game, InputEvent, Key, State};

pub struct TestGame;

#[derive(Default)]
pub struct TestData {
    pub log: Vec<String>,
    pub queued_events: Vec<u32>,
}

impl Game for TestGame {
    const TITLE: &'static str = "Test Game";
    const DATA_DIR: &'static str = "HearkenTestGame";
    const ENV_PREFIX: &'static str = "HEARKEN_TEST_GAME";
    type Event = u32;
    type Data = TestData;
    type Settings = ();

    fn pump(ctx: &mut Ctx<Self>, _dt: f64) -> Vec<u32> {
        std::mem::take(&mut ctx.game.queued_events)
    }

    fn observe(ctx: &mut Ctx<Self>, event: &u32) {
        ctx.game.log.push(format!("observe {event}"));
    }
}

/// A screen that writes every hook it sees into `ctx.game.log`, and runs
/// `on_return` when Return is pressed.
pub struct Probe {
    pub name: &'static str,
    on_return: Option<Box<dyn FnMut(&mut Ctx<TestGame>)>>,
}

impl Probe {
    pub fn new(name: &'static str) -> Self {
        Probe { name, on_return: None }
    }

    pub fn on_return(mut self, f: impl FnMut(&mut Ctx<TestGame>) + 'static) -> Self {
        self.on_return = Some(Box::new(f));
        self
    }
}

impl State<TestGame> for Probe {
    fn name(&self) -> &'static str {
        self.name
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.game.log.push(format!("{} enter", self.name));
    }
    fn exit(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.game.log.push(format!("{} exit", self.name));
    }
    fn resume(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.game.log.push(format!("{} resume", self.name));
    }
    fn handle_input(&mut self, ctx: &mut Ctx<TestGame>, event: &InputEvent) {
        if let Some((key, _, _)) = event.key_down() {
            ctx.game.log.push(format!("{} key {key:?}", self.name));
            if key == Key::Return {
                if let Some(f) = self.on_return.as_mut() {
                    f(ctx);
                }
            }
        }
    }
    fn on_event(&mut self, ctx: &mut Ctx<TestGame>, event: &u32) {
        ctx.game.log.push(format!("{} event {event}", self.name));
    }
    fn update(&mut self, ctx: &mut Ctx<TestGame>, _dt: f64) {
        ctx.game.log.push(format!("{} update", self.name));
    }
    fn lines(&self, _ctx: &Ctx<TestGame>) -> Vec<String> {
        vec![self.name.to_string()]
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        format!("{} help", self.name)
    }
}

/// The log since the last call, without the per-frame update lines.
pub fn take_log(ctx: &mut Ctx<TestGame>) -> Vec<String> {
    std::mem::take(&mut ctx.game.log)
        .into_iter()
        .filter(|line| !line.ends_with(" update"))
        .collect()
}
```

- [ ] **Step 2: Write the failing tests**

`crates/hearken-app/tests/app.rs`:

```rust
mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{take_log, Probe, TestData, TestGame};
use hearken_app::{App, Ctx, Headless, HeadlessShell, InputEvent, Key, Paths, Settings, State};
use hearken_speech::{CaptureSpeech, SharedCapture};

fn headless(root: Probe) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), root);
    take_log(&mut h.app.ctx);
    h
}

#[test]
fn push_enters_after_the_handler_returns() {
    let mut h = headless(Probe::new("root").on_return(|ctx| ctx.push(Probe::new("b"))));
    h.press(Key::Return);
    assert_eq!(take_log(&mut h.app.ctx), ["root key Return", "b enter"]);
    assert_eq!((h.app.top_name(), h.app.depth()), ("b", 2));
}

#[test]
fn pop_exits_and_resumes_the_screen_below() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b").on_return(|ctx| ctx.pop()));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.press(Key::Return);
    assert_eq!(take_log(&mut h.app.ctx), ["b key Return", "b exit", "root resume"]);
    assert_eq!(h.app.depth(), 1);
}

#[test]
fn replace_exits_then_enters_without_resuming_the_screen_below() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b").on_return(|ctx| ctx.replace(Probe::new("c"))));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.press(Key::Return);
    assert_eq!(take_log(&mut h.app.ctx), ["b key Return", "b exit", "c enter"]);
    assert_eq!((h.app.top_name(), h.app.depth()), ("c", 2));
}

#[test]
fn reset_exits_every_screen_top_down_then_enters() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b"));
    h.app.ctx.push(Probe::new("c").on_return(|ctx| ctx.reset_to(Probe::new("d"))));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.press(Key::Return);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["c key Return", "c exit", "b exit", "root exit", "d enter"]
    );
    assert_eq!(h.app.depth(), 1);
}

#[test]
fn popping_the_root_and_pushing_in_the_same_handler_keeps_running() {
    let mut h = headless(Probe::new("root").on_return(|ctx| {
        ctx.pop();
        ctx.push(Probe::new("next"));
    }));
    h.press(Key::Return);
    assert!(h.app.ctx.running);
    assert_eq!(h.app.top_name(), "next");
}

#[test]
fn popping_the_last_screen_ends_the_run() {
    let mut h = headless(Probe::new("root").on_return(|ctx| ctx.pop()));
    h.press(Key::Return);
    assert!(!h.app.ctx.running);
    assert_eq!(h.app.depth(), 0);
}

struct Runaway;

impl State<TestGame> for Runaway {
    fn name(&self) -> &'static str {
        "runaway"
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.push(Runaway);
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
}

#[test]
fn a_screen_that_keeps_pushing_itself_is_cut_off_after_sixteen_rounds() {
    let h = Headless::new(TestData::default(), Runaway);
    assert!(h.app.ctx.running);
    assert_eq!(h.app.depth(), 17, "the first screen plus sixteen rounds");
}

#[test]
fn a_quit_event_ends_the_run() {
    let mut h = headless(Probe::new("root"));
    h.send(InputEvent::Quit);
    assert!(!h.app.ctx.running);
}

#[test]
fn f2_stops_speech_and_the_screen_never_sees_it() {
    let mut h = headless(Probe::new("root"));
    h.press(Key::F2);
    assert_eq!(h.speech.borrow().stop_calls(), 1);
    assert!(take_log(&mut h.app.ctx).is_empty());
}

#[test]
fn f1_speaks_the_top_screen_help_and_the_screen_never_sees_it() {
    let mut h = headless(Probe::new("root"));
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), ["root help"]);
    assert!(take_log(&mut h.app.ctx).is_empty());
}

#[test]
fn game_events_are_observed_then_reach_only_the_top_screen() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b"));
    h.frame(0.0);
    take_log(&mut h.app.ctx);
    h.app.ctx.game.queued_events = vec![7, 8];
    h.frame(0.0);
    assert_eq!(
        take_log(&mut h.app.ctx),
        ["observe 7", "b event 7", "observe 8", "b event 8"]
    );
}

#[test]
fn only_the_top_screen_updates() {
    let mut h = headless(Probe::new("root"));
    h.app.ctx.push(Probe::new("b"));
    h.frame(0.0);
    h.app.ctx.game.log.clear();
    h.frame(0.016);
    assert_eq!(h.app.ctx.game.log, ["b update"]);
}

#[test]
fn speech_is_recorded_in_the_transcript_and_blank_lines_are_dropped() {
    let mut h = headless(Probe::new("root"));
    h.clear();
    h.app.ctx.say("Hello.");
    h.app.ctx.say("   ");
    h.app.ctx.say_event("Watch out.", false);
    assert_eq!(h.spoken(), ["Hello."]);
    assert_eq!(h.speech.borrow().event_lines(), ["Watch out."]);
    let tail: Vec<_> = h.app.ctx.transcript.iter().rev().take(2).rev().cloned().collect();
    assert_eq!(tail, ["Hello.", "[event] Watch out."]);
}

#[test]
fn the_transcript_keeps_the_newest_lines() {
    let mut h = headless(Probe::new("root"));
    for i in 0..(hearken_app::TRANSCRIPT_CAP + 5) {
        h.app.ctx.say(format!("line {i}"));
    }
    assert_eq!(h.app.ctx.transcript.len(), hearken_app::TRANSCRIPT_CAP);
    assert_eq!(
        h.app.ctx.transcript.last().map(String::as_str),
        Some(format!("line {}", hearken_app::TRANSCRIPT_CAP + 4).as_str())
    );
}

struct Announcer;

impl State<TestGame> for Announcer {
    fn name(&self) -> &'static str {
        "announcer"
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.say("Main menu.");
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
}

#[test]
fn notices_are_spoken_after_the_first_screen_announces_itself() {
    let capture = Rc::new(RefCell::new(CaptureSpeech::new()));
    let dir = tempfile::tempdir().unwrap();
    let mut ctx: Ctx<TestGame> = Ctx::new(
        Box::new(SharedCapture(capture.clone())),
        Settings::default(),
        Paths::at(dir.path()),
        TestData::default(),
    );
    ctx.notices.push("Your settings were reset.".into());
    let _app = App::new(ctx, Box::new(Announcer));
    assert_eq!(
        capture.borrow().calls(hearken_speech::SpeechChannel::Main),
        [("Main menu.".to_string(), true), ("Your settings were reset.".to_string(), false)]
    );
}

#[test]
fn run_ends_when_a_screen_quits_then_saves_settings_and_shuts_speech_down() {
    let mut h = headless(Probe::new("root").on_return(|ctx| ctx.quit()));
    let mut shell = HeadlessShell::new();
    shell.push(InputEvent::key(Key::Return));
    h.app.run(&mut shell);
    assert!(!h.app.ctx.running);
    assert!(h.app.ctx.paths.settings_file().is_file());
    assert_eq!(h.speech.borrow().shutdown_calls(), 1);
    assert_eq!(shell.rendered.last().map(Vec::as_slice), Some(&["root".to_string()][..]));
}
```

- [ ] **Step 3: Run to confirm they fail**

`cargo test -p hearken-app --test app > target/t5.log 2>&1; echo exit=$?` — expected: compile errors naming `Game`, `Ctx`, `Headless`.

- [ ] **Step 4: `game.rs`, `state.rs`, `ctx.rs`**

`src/game.rs`:

```rust
//! What the engine needs to know about one game.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::ctx::Ctx;

pub trait Game: Sized + 'static {
    /// The window title, and the name the game is known by.
    const TITLE: &'static str;
    /// The folder under the platform's local data folder that holds the
    /// settings and the session log. Also names the single-instance lock.
    const DATA_DIR: &'static str;
    /// Prefix of the game's environment overrides: `<PREFIX>_DATA_DIR`,
    /// `<PREFIX>_LOG_FILE`, `<PREFIX>_NO_SPEECH`, `<PREFIX>_SPEECH_BACKEND`,
    /// `<PREFIX>_STREAM_TRANSCRIPT`.
    const ENV_PREFIX: &'static str;

    /// What the game's rules emit. The top screen receives each one in
    /// `State::on_event`.
    type Event;
    /// The game's own state and services, at `ctx.game`.
    type Data;
    /// The game's settings, saved under `"game"` in `settings.json`.
    type Settings: Serialize + DeserializeOwned + Default + Clone;

    /// The events the game's rules produced this frame. Called once per
    /// frame, after input.
    fn pump(_ctx: &mut Ctx<Self>, _dt: f64) -> Vec<Self::Event> {
        Vec::new()
    }

    /// Sees every pumped event before the top screen does: bookkeeping that
    /// must happen whichever screen is open.
    fn observe(_ctx: &mut Ctx<Self>, _event: &Self::Event) {}

    /// Called once when the run ends, before the settings are saved and
    /// speech shuts down.
    fn on_exit(_ctx: &mut Ctx<Self>) {}
}
```

`src/state.rs`:

```rust
//! Screens, and the changes to the screen stack they ask for.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::InputEvent;

/// One screen. Only `name` and `help` are required.
pub trait State<G: Game> {
    /// A stable identifier for logs and tests.
    fn name(&self) -> &'static str;
    /// Pushed onto the stack.
    fn enter(&mut self, _ctx: &mut Ctx<G>) {}
    /// Popped or replaced.
    fn exit(&mut self, _ctx: &mut Ctx<G>) {}
    /// Revealed again when the screen above it closes. Announces afresh by
    /// default.
    fn resume(&mut self, ctx: &mut Ctx<G>) {
        self.enter(ctx)
    }
    fn handle_input(&mut self, _ctx: &mut Ctx<G>, _event: &InputEvent) {}
    /// An event from the game's rules, after `Game::observe`.
    fn on_event(&mut self, _ctx: &mut Ctx<G>, _event: &G::Event) {}
    /// Once per frame, top screen only.
    fn update(&mut self, _ctx: &mut Ctx<G>, _dt: f64) {}
    /// What the window shows a sighted helper. The player gets speech.
    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        Vec::new()
    }
    /// Spoken on F1.
    fn help(&self, ctx: &Ctx<G>) -> String;
}

/// A change to the stack, queued on the context and applied once the
/// current handler has returned, so a screen never changes the stack it is
/// running on.
pub enum StackOp<G: Game> {
    Push(Box<dyn State<G>>),
    Pop,
    Replace(Box<dyn State<G>>),
    Reset(Box<dyn State<G>>),
}
```

`src/ctx.rs`:

```rust
//! What every screen is handed.

use hearken_speech::SpeechSink;

use crate::game::Game;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::state::{StackOp, State};

/// The `log` target every spoken line is written under, so a session log
/// reads as a transcript of the run.
pub const TRANSCRIPT_TARGET: &str = "hearken.transcript";

/// How many spoken lines `Ctx::transcript` keeps.
pub const TRANSCRIPT_CAP: usize = 2000;

pub struct Ctx<G: Game> {
    pub speech: Box<dyn SpeechSink>,
    pub settings: Settings<G::Settings>,
    pub paths: Paths,
    pub game: G::Data,
    pub running: bool,
    /// Every line spoken this run, newest last. Event lines start `[event] `.
    pub transcript: Vec<String>,
    /// Lines for the player once the first screen has announced itself.
    pub notices: Vec<String>,
    pub frame: u64,
    /// Seconds since the run started, the sum of every frame's `dt`.
    pub clock: f64,
    pending: Vec<StackOp<G>>,
}

impl<G: Game> Ctx<G> {
    pub fn new(
        speech: Box<dyn SpeechSink>,
        settings: Settings<G::Settings>,
        paths: Paths,
        game: G::Data,
    ) -> Self {
        Ctx {
            speech,
            settings,
            paths,
            game,
            running: true,
            transcript: Vec::new(),
            notices: Vec::new(),
            frame: 0,
            clock: 0.0,
            pending: Vec::new(),
        }
    }

    /// Speak on the main channel now, cutting off what it was saying.
    pub fn say(&mut self, text: impl Into<String>) {
        self.speak(text.into(), true);
    }

    /// Speak on the main channel after what it is saying.
    pub fn say_queued(&mut self, text: impl Into<String>) {
        self.speak(text.into(), false);
    }

    /// Speak on the gameplay-event channel, which a separate voice can carry
    /// so the screen reader never talks over it.
    pub fn say_event(&mut self, text: impl Into<String>, interrupt: bool) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        self.speech.say_event(&text, interrupt);
        self.record(format!("[event] {text}"));
    }

    fn speak(&mut self, text: String, interrupt: bool) {
        if text.trim().is_empty() {
            return;
        }
        self.speech.say(&text, interrupt);
        self.record(text);
    }

    fn record(&mut self, line: String) {
        log::info!(target: TRANSCRIPT_TARGET, "{line}");
        self.transcript.push(line);
        if self.transcript.len() > TRANSCRIPT_CAP {
            let excess = self.transcript.len() - TRANSCRIPT_CAP;
            self.transcript.drain(..excess);
        }
    }

    pub fn stop_speech(&mut self) {
        self.speech.stop();
    }

    /// Push the saved speech settings to the voices.
    pub fn apply_speech_settings(&mut self) {
        hearken_speech::apply_speech_settings(self.speech.as_mut(), &mut self.settings.speech);
    }

    pub fn push(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Push(Box::new(screen)));
    }

    pub fn pop(&mut self) {
        self.pending.push(StackOp::Pop);
    }

    pub fn replace(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Replace(Box::new(screen)));
    }

    pub fn reset_to(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Reset(Box::new(screen)));
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub(crate) fn take_pending(&mut self) -> Vec<StackOp<G>> {
        std::mem::take(&mut self.pending)
    }

    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
    }
}
```

- [ ] **Step 5: `shell.rs`, `app.rs`, `testing.rs`**

`src/shell.rs`:

```rust
//! The window and event pump, behind a trait so tests need no display.

use std::collections::VecDeque;

use crate::input::InputEvent;

pub const FRAME_SECONDS: f64 = 1.0 / 60.0;

pub trait Shell {
    fn poll(&mut self) -> Vec<InputEvent>;
    fn render(&mut self, lines: &[String]);
    /// Sleep out the rest of the frame and return the seconds since the
    /// previous call.
    fn frame(&mut self) -> f64;
}

/// No window: events come from a queue, one per frame, and frames take no
/// time.
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
```

`src/app.rs`:

```rust
//! The screen stack and the frame loop that drives it.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::shell::Shell;
use crate::state::{StackOp, State};

/// Rounds of stack changes applied at once. An `enter` may queue more, and a
/// screen that always does would otherwise hang the game.
const MAX_STACK_ROUNDS: usize = 16;

pub struct App<G: Game> {
    pub ctx: Ctx<G>,
    states: Vec<Box<dyn State<G>>>,
}

impl<G: Game> App<G> {
    /// Enter the first screen, then speak any notices behind its
    /// announcement.
    pub fn new(mut ctx: Ctx<G>, mut first: Box<dyn State<G>>) -> Self {
        first.enter(&mut ctx);
        for notice in std::mem::take(&mut ctx.notices) {
            ctx.say_queued(notice);
        }
        let mut app = App { ctx, states: vec![first] };
        app.apply_pending();
        app
    }

    /// One frame: input to the top screen, the game's events, the top
    /// screen's update, speech upkeep, and the stack changes asked for.
    pub fn step(&mut self, events: Vec<InputEvent>, dt: f64) {
        for event in events {
            match &event {
                InputEvent::Quit => {
                    self.ctx.running = false;
                    continue;
                }
                // Coming back to the window re-checks which screen reader
                // is running; the screen still sees the event.
                InputEvent::WindowFocusGained => self.ctx.speech.request_refresh(),
                // F2 silences speech everywhere, so no screen can hold it.
                InputEvent::KeyDown { key: Key::F2, .. } => {
                    self.ctx.stop_speech();
                    continue;
                }
                // F1 reads the top screen's help.
                InputEvent::KeyDown { key: Key::F1, .. } => {
                    let help = self.states.last().map(|top| top.help(&self.ctx));
                    if let Some(help) = help {
                        self.ctx.say(help);
                    }
                    continue;
                }
                _ => {}
            }
            if let Some(top) = self.states.last_mut() {
                top.handle_input(&mut self.ctx, &event);
            }
        }
        // A screen that pushed or popped from a key must not also get this
        // frame's update.
        self.apply_pending();

        for event in G::pump(&mut self.ctx, dt) {
            G::observe(&mut self.ctx, &event);
            if let Some(top) = self.states.last_mut() {
                top.on_event(&mut self.ctx, &event);
            }
        }
        if let Some(top) = self.states.last_mut() {
            top.update(&mut self.ctx, dt);
        }
        self.ctx.speech.poll(dt);

        self.apply_pending();
        self.ctx.frame += 1;
        self.ctx.clock += dt;
    }

    /// Run until something quits, then let the game finish, save the
    /// settings and shut speech down.
    pub fn run(&mut self, shell: &mut dyn Shell) {
        while self.ctx.running {
            let events = shell.poll();
            let dt = shell.frame();
            self.step(events, dt);
            let lines = self.top_lines();
            shell.render(&lines);
        }
        G::on_exit(&mut self.ctx);
        if let Err(e) = self.ctx.settings.save(&self.ctx.paths) {
            log::error!("could not save settings: {e}");
        }
        self.ctx.speech.shutdown();
    }

    pub fn top_name(&self) -> &'static str {
        self.states.last().map(|s| s.name()).unwrap_or("none")
    }

    pub fn depth(&self) -> usize {
        self.states.len()
    }

    pub fn top_lines(&self) -> Vec<String> {
        self.states
            .last()
            .map(|s| s.lines(&self.ctx))
            .unwrap_or_default()
    }

    fn apply_pending(&mut self) {
        for _ in 0..MAX_STACK_ROUNDS {
            if !self.ctx.has_pending() {
                break;
            }
            for op in self.ctx.take_pending() {
                match op {
                    StackOp::Push(mut next) => {
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                    }
                    StackOp::Pop => {
                        if let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        // An empty stack is only judged once the whole queue
                        // has drained: popping the root and pushing its
                        // replacement in one handler is allowed.
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
        if self.ctx.has_pending() {
            log::error!("screen changes kept queueing themselves; dropping the rest");
            self.ctx.clear_pending();
        }
        if self.states.is_empty() {
            self.ctx.running = false;
        }
    }
}
```

`src/testing.rs`:

```rust
//! A headless app for tests: captured speech, a throwaway data folder, no
//! window.

use std::cell::RefCell;
use std::rc::Rc;

use hearken_speech::{CaptureSpeech, SharedCapture};

use crate::app::App;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::paths::Paths;
use crate::settings::Settings;
use crate::state::State;

pub struct Headless<G: Game> {
    pub app: App<G>,
    pub speech: Rc<RefCell<CaptureSpeech>>,
    _dir: tempfile::TempDir,
}

impl<G: Game> Headless<G> {
    pub fn new(game: G::Data, first: impl State<G> + 'static) -> Self {
        let dir = tempfile::tempdir().expect("a temporary data folder");
        let speech = Rc::new(RefCell::new(CaptureSpeech::new()));
        let ctx = Ctx::new(
            Box::new(SharedCapture(speech.clone())),
            Settings::default(),
            Paths::at(dir.path()),
            game,
        );
        Headless {
            app: App::new(ctx, Box::new(first)),
            speech,
            _dir: dir,
        }
    }

    /// One key press, then the rest of that frame.
    pub fn press(&mut self, key: Key) {
        self.send(InputEvent::key(key));
    }

    pub fn send(&mut self, event: InputEvent) {
        self.app.step(vec![event], 0.0);
    }

    pub fn frame(&mut self, dt: f64) {
        self.app.step(Vec::new(), dt);
    }

    /// Main-channel lines spoken since the last `clear`.
    pub fn spoken(&self) -> Vec<String> {
        self.speech.borrow().main_lines()
    }

    pub fn clear(&mut self) {
        self.speech.borrow_mut().clear();
    }
}
```

`src/lib.rs` becomes:

```rust
//! The shell of an audio-first, screen-reader-first game.

pub mod app;
pub mod ctx;
pub mod game;
pub mod input;
pub mod paths;
pub mod settings;
pub mod shell;
pub mod state;
pub mod testing;
pub mod text;

pub use app::App;
pub use ctx::{Ctx, TRANSCRIPT_CAP, TRANSCRIPT_TARGET};
pub use game::Game;
pub use input::{ControllerAxis, ControllerButton, InputEvent, Key, Mods};
pub use paths::Paths;
pub use settings::Settings;
pub use shell::{HeadlessShell, Shell, FRAME_SECONDS};
pub use state::{StackOp, State};
pub use testing::Headless;
```

Add `hearken-speech.workspace = true` and `tempfile.workspace = true` to `[dev-dependencies]` of `hearken-app` only if the tests fail to resolve them through the normal dependencies (they are normal dependencies already, so this should not be needed).

- [ ] **Step 6: Run the tests**

`cargo test -p hearken-app --test app > target/t5.log 2>&1; echo exit=$?` — expected `exit=0`, 16 passed. Then `cargo test -p hearken-app --test foundations > target/t5b.log 2>&1; echo exit=$?` — still `exit=0`.

- [ ] **Step 7: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings
git add -A
git commit -m "feat(app): Game trait, context, queued screen stack and frame loop

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Menus, and the Settings, Help and Pause screens

**Files:**
- Create: `crates/hearken-app/src/menu.rs`, `src/screens/{mod.rs,settings.rs,help.rs,pause.rs}`
- Modify: `crates/hearken-app/src/lib.rs`
- Test: `crates/hearken-app/tests/menu.rs`, `crates/hearken-app/tests/screens.rs`

**Interfaces:**
- Consumes: `Ctx`, `Game`, `State`, `InputEvent`, `Key`, `Headless` (Task 5).
- Produces:
  - `Label<S, G>`, `MenuItem<S, G>` (`new`, `inert`, `help`, `adjustable`, `text`, `help_text`), `MenuCore<S, G>` (`new`, `set_items`, `with_intro_help`; pub fields `title, intro_help, items, index, revision`), `DEFAULT_INTRO_HELP`
  - `trait Menu<G: Game>: Sized + 'static` with required `name, menu, menu_mut, build_items` and overridable `enter, exit, resume, handle_input, on_event, update, lines, help, on_escape`; helpers `refresh, announce, current_text, current_help, speak_current, move_by, jump, letter_jump, activate, adjust`
  - `fn menu_input<G: Game, S: Menu<G>>(state: &mut S, ctx: &mut Ctx<G>, event: &InputEvent)`
  - Blanket `impl<G: Game, M: Menu<G>> State<G> for M` (no macro)
  - `screens::SettingsScreen<G>::new()`, `screens::settings::rate_label(f64) -> String`
  - `screens::HelpScreen<G>::new(doc: &'static str)`, `screens::help::{sections, Section}`
  - `screens::PauseScreen<G>::new(help_doc: &'static str, to_title: fn(&mut Ctx<G>))`

- [ ] **Step 1: Write the failing menu tests**

`crates/hearken-app/tests/menu.rs`:

```rust
mod common;

use common::{Probe, TestData, TestGame};
use hearken_app::menu::{Label, Menu, MenuCore, MenuItem, DEFAULT_INTRO_HELP};
use hearken_app::{Ctx, Headless, InputEvent, Key};

/// Alpha adjusts, Beta cannot, Gamma rebuilds the rows with a fourth.
struct TestMenu {
    menu: MenuCore<Self, TestGame>,
    activated: u32,
    volume: i32,
    grown: bool,
}

impl TestMenu {
    fn new() -> Self {
        TestMenu { menu: MenuCore::new("Test menu"), activated: 0, volume: 70, grown: false }
    }
}

impl Menu<TestGame> for TestMenu {
    fn name(&self) -> &'static str {
        "test_menu"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        let mut items = vec![
            MenuItem::new(
                Label::dynamic(|s: &TestMenu, _| {
                    if s.volume == 70 { "Alpha".to_string() } else { format!("Alpha: {}", s.volume) }
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

struct Empty {
    menu: MenuCore<Self, TestGame>,
}

impl Menu<TestGame> for Empty {
    fn name(&self) -> &'static str {
        "empty"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        Vec::new()
    }
}

/// The test menu open over a probe root, with its announcement flushed.
fn open() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(TestMenu::new());
    h.frame(0.0);
    h
}

#[test]
fn opening_announces_the_title_the_count_and_the_first_row() {
    let h = open();
    assert_eq!(h.spoken().last().map(String::as_str), Some("Test menu. 3 items. Alpha"));
}

#[test]
fn up_and_down_speak_the_row_and_wrap() {
    let mut h = open();
    h.clear();
    for key in [Key::Down, Key::Down, Key::Down, Key::Up] {
        h.press(key);
    }
    assert_eq!(h.spoken(), ["Beta", "Gamma", "Alpha", "Gamma"]);
}

#[test]
fn home_and_end_jump_to_the_edges() {
    let mut h = open();
    h.clear();
    h.press(Key::End);
    h.press(Key::Home);
    assert_eq!(h.spoken(), ["Gamma", "Alpha"]);
}

#[test]
fn enter_runs_the_row_and_an_action_that_rebuilds_keeps_its_rows() {
    let mut h = open();
    h.press(Key::End);
    h.press(Key::Return);
    assert!(h.app.top_lines().contains(&"  Delta".to_string()));
}

#[test]
fn left_and_right_adjust_and_read_back_and_a_row_without_an_adjuster_is_silent() {
    let mut h = open();
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["Alpha: 80"]);
    h.press(Key::Down);
    h.clear();
    h.press(Key::Right);
    assert!(h.spoken().is_empty());
}

#[test]
fn a_letter_jumps_to_the_next_matching_row_and_a_miss_is_silent() {
    let mut h = open();
    h.clear();
    h.send(InputEvent::typed('g'));
    assert_eq!(h.spoken(), ["Gamma"]);
    h.clear();
    h.send(InputEvent::typed('z'));
    assert!(h.spoken().is_empty());
}

#[test]
fn f1_reads_the_row_help_or_the_menu_help_when_the_row_has_none() {
    let mut h = open();
    h.clear();
    h.press(Key::F1);
    h.press(Key::End);
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), [DEFAULT_INTRO_HELP]);
}

#[test]
fn f1_on_the_first_row_reads_its_help() {
    let mut h = open();
    h.clear();
    h.press(Key::F1);
    assert_eq!(h.spoken(), ["Alpha help"]);
}

#[test]
fn escape_closes_the_menu_and_the_screen_below_resumes() {
    let mut h = open();
    h.press(Key::Escape);
    assert_eq!((h.app.top_name(), h.app.depth()), ("root", 1));
}

#[test]
fn coming_back_keeps_the_cursor_and_announces_without_the_count() {
    let mut h = open();
    h.press(Key::Down);
    h.app.ctx.push(Probe::new("above"));
    h.frame(0.0);
    h.app.ctx.pop();
    h.clear();
    h.frame(0.0);
    assert_eq!(h.spoken(), ["Test menu. Beta"]);
}

#[test]
fn an_empty_menu_says_so_and_ignores_keys() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(Empty { menu: MenuCore::new("Nothing here") });
    h.frame(0.0);
    assert_eq!(h.spoken().last().map(String::as_str), Some("Nothing here. 0 items. No options"));
    h.clear();
    for key in [Key::Down, Key::Up, Key::Home, Key::End, Key::Return, Key::Right] {
        h.press(key);
    }
    assert!(h.spoken().is_empty());
    h.press(Key::F1);
    assert_eq!(h.spoken(), [DEFAULT_INTRO_HELP]);
}
```

- [ ] **Step 2: Write the failing screen tests**

`crates/hearken-app/tests/screens.rs`:

```rust
mod common;

use common::{Probe, TestData, TestGame};
use hearken_app::screens::help::sections;
use hearken_app::screens::{HelpScreen, PauseScreen, SettingsScreen};
use hearken_app::{Ctx, Headless, Key};

const DOC: &str = "# Test Game\n\nIgnored preamble.\n\n## Getting around\n\nUp and Down move.\nEnter chooses.\n\n## Quitting\n\nPick Quit.\n";

fn over_root(screen: impl hearken_app::State<TestGame> + 'static) -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(screen);
    h.frame(0.0);
    h
}

#[test]
fn help_sections_split_on_level_two_headings() {
    let s = sections(DOC);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].title, "Getting around");
    assert_eq!(s[0].body, "Up and Down move. Enter chooses.");
    assert_eq!(s[1].body, "Pick Quit.");
}

#[test]
fn the_help_screen_lists_topics_and_reads_one_on_enter() {
    let mut h = over_root(HelpScreen::<TestGame>::new(DOC));
    assert_eq!(h.spoken().last().map(String::as_str), Some("Help. 2 items. Getting around"));
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), ["Up and Down move. Enter chooses."]);
}

#[test]
fn speech_rate_moves_by_five_percent_clamps_and_reaches_the_voice() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    assert_eq!(h.spoken().last().map(String::as_str), Some("Settings. 1 item. Speech rate: 50 percent"));
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.spoken(), ["Speech rate: 55 percent"]);
    assert_eq!(h.app.ctx.settings.speech.rate, 0.55);
    assert_eq!(
        h.speech.borrow().configure_calls().last().and_then(|c| c.0),
        Some(0.55),
        "the new rate reached the voice"
    );
    for _ in 0..20 {
        h.press(Key::Left);
    }
    assert_eq!(h.app.ctx.settings.speech.rate, 0.0);
}

#[test]
fn escape_on_settings_saves_them_and_goes_back() {
    let mut h = over_root(SettingsScreen::<TestGame>::new());
    h.press(Key::Right);
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "root");
    let saved = std::fs::read_to_string(h.app.ctx.paths.settings_file()).unwrap();
    assert!(saved.contains("0.55"), "saved: {saved}");
}

fn back_to_title(ctx: &mut Ctx<TestGame>) {
    ctx.reset_to(Probe::new("title"));
}

#[test]
fn pause_lists_its_rows_and_each_one_works() {
    let mut h = over_root(PauseScreen::<TestGame>::new(DOC, back_to_title));
    assert_eq!(h.spoken().last().map(String::as_str), Some("Paused. 5 items. Resume"));
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "root", "Resume closes the pause menu");

    h.app.ctx.push(PauseScreen::<TestGame>::new(DOC, back_to_title));
    h.frame(0.0);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "settings");
    h.press(Key::Escape);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "help");
    h.press(Key::Escape);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!((h.app.top_name(), h.app.depth()), ("title", 1));

    h.app.ctx.push(PauseScreen::<TestGame>::new(DOC, back_to_title));
    h.frame(0.0);
    h.press(Key::End);
    h.press(Key::Return);
    assert!(!h.app.ctx.running, "Quit ends the run");
}
```

- [ ] **Step 3: Run to confirm they fail**

`cargo test -p hearken-app --test menu --test screens > target/t6.log 2>&1; echo exit=$?` — expected: compile errors naming `menu` and `screens`.

- [ ] **Step 4: `menu.rs`**

This is LB's `states/menu.rs` (lines 1-452, verbatim in `m1-lb-app.md` §6.1) made generic over the game, with the interface sounds removed (they return with audio in M2), F1 handled by the app, keypad Enter accepted, and the macro replaced by a blanket impl.

```rust
//! The accessible menu: a list of rows navigated by ear. Every move speaks
//! the row it lands on, Enter runs the row's action, Left and Right adjust a
//! row that has an adjuster and speak the new value, and F1 (handled by the
//! app) reads the row's help.
//!
//! A screen embeds a [`MenuCore`] and implements [`Menu`]; every `Menu` is a
//! [`State`] through the blanket impl at the bottom of this file.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::state::State;

/// A label computed while the menu is open. Rows whose text depends on
/// state that changes under them must be read through this, never off a
/// string captured when the menu was built.
pub type DynamicLabel<S, G> = Box<dyn Fn(&S, &Ctx<G>) -> String>;
/// What activating a row does.
pub type MenuAction<S, G> = Box<dyn FnMut(&mut S, &mut Ctx<G>)>;
/// What Left and Right do to a row, given -1 or +1.
pub type MenuAdjust<S, G> = Box<dyn FnMut(&mut S, &mut Ctx<G>, i32)>;

/// A row's label or help: fixed text, or text computed on demand.
pub enum Label<S, G: Game> {
    Text(String),
    Dynamic(DynamicLabel<S, G>),
}

impl<S, G: Game> Label<S, G> {
    pub fn dynamic(f: impl Fn(&S, &Ctx<G>) -> String + 'static) -> Self {
        Label::Dynamic(Box::new(f))
    }

    pub fn text(&self, state: &S, ctx: &Ctx<G>) -> String {
        match self {
            Label::Text(text) => text.clone(),
            Label::Dynamic(f) => f(state, ctx),
        }
    }
}

impl<S, G: Game> From<&str> for Label<S, G> {
    fn from(text: &str) -> Self {
        Label::Text(text.to_string())
    }
}

impl<S, G: Game> From<String> for Label<S, G> {
    fn from(text: String) -> Self {
        Label::Text(text)
    }
}

/// One row of a menu.
pub struct MenuItem<S, G: Game> {
    label: Label<S, G>,
    help: Option<Label<S, G>>,
    action: MenuAction<S, G>,
    /// Set when Left and Right change this row instead of doing nothing.
    pub adjust: Option<MenuAdjust<S, G>>,
}

impl<S: 'static, G: Game> MenuItem<S, G> {
    pub fn new(
        label: impl Into<Label<S, G>>,
        action: impl FnMut(&mut S, &mut Ctx<G>) + 'static,
    ) -> Self {
        MenuItem {
            label: label.into(),
            help: None,
            action: Box::new(action),
            adjust: None,
        }
    }

    /// A row that does nothing when activated: a heading or a readout.
    pub fn inert(label: impl Into<Label<S, G>>) -> Self {
        MenuItem::new(label, |_: &mut S, _: &mut Ctx<G>| {})
    }

    pub fn help(mut self, help: impl Into<Label<S, G>>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Give the row a Left/Right adjuster.
    pub fn adjustable(mut self, f: impl FnMut(&mut S, &mut Ctx<G>, i32) + 'static) -> Self {
        self.adjust = Some(Box::new(f));
        self
    }

    pub fn text(&self, state: &S, ctx: &Ctx<G>) -> String {
        self.label.text(state, ctx)
    }

    pub fn help_text(&self, state: &S, ctx: &Ctx<G>) -> Option<String> {
        self.help.as_ref().map(|h| h.text(state, ctx))
    }
}

/// The reusable half of a menu screen: the title, the rows, the cursor.
pub struct MenuCore<S, G: Game> {
    pub title: String,
    pub intro_help: String,
    pub items: Vec<MenuItem<S, G>>,
    pub index: usize,
    /// Bumped whenever the rows are replaced. `activate` and `adjust` lend
    /// the rows to a callback and compare this afterwards to tell a
    /// callback that installed its own rows from one that did not. Anything
    /// that assigns `items` directly must bump it; `set_items` does both.
    pub revision: u64,
}

pub const DEFAULT_INTRO_HELP: &str =
    "Up and Down move, Enter chooses, Escape goes back, F2 stops speech.";

impl<S, G: Game> MenuCore<S, G> {
    pub fn new(title: impl Into<String>) -> Self {
        MenuCore {
            title: title.into(),
            intro_help: DEFAULT_INTRO_HELP.to_string(),
            items: Vec::new(),
            index: 0,
            revision: 0,
        }
    }

    /// Replace the rows and mark them changed.
    pub fn set_items(&mut self, items: Vec<MenuItem<S, G>>) {
        self.items = items;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn with_intro_help(mut self, help: impl Into<String>) -> Self {
        self.intro_help = help.into();
        self
    }
}

impl<S, G: Game> Default for MenuCore<S, G> {
    fn default() -> Self {
        MenuCore::new("Menu")
    }
}

/// A vertically navigated, fully spoken menu. Implement `name`, the two
/// accessors and `build_items`; everything else has a default the screen
/// may override.
pub trait Menu<G: Game>: Sized + 'static {
    fn name(&self) -> &'static str;
    fn menu(&self) -> &MenuCore<Self, G>;
    fn menu_mut(&mut self) -> &mut MenuCore<Self, G>;
    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>>;

    // -- State hooks ----------------------------------------------------------

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        self.refresh(ctx, false);
        self.announce(ctx);
    }

    fn exit(&mut self, _ctx: &mut Ctx<G>) {}

    /// Revealed again: rebuild the rows (a setting may have changed) and
    /// re-announce, keeping the cursor.
    fn resume(&mut self, ctx: &mut Ctx<G>) {
        self.refresh(ctx, true);
        self.announce(ctx);
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        menu_input(self, ctx, event)
    }

    fn on_event(&mut self, _ctx: &mut Ctx<G>, _event: &G::Event) {}

    fn update(&mut self, _ctx: &mut Ctx<G>, _dt: f64) {}

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let core = self.menu();
        let mut out = vec![core.title.clone()];
        for (i, item) in core.items.iter().enumerate() {
            let marker = if i == core.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out
    }

    /// F1: the focused row's help, or the menu's own help when the row has
    /// none.
    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx)
            .unwrap_or_else(|| self.menu().intro_help.clone())
    }

    /// What Escape does. Closing the screen is the default, so a menu at
    /// the root of the stack must override this: popping the only screen
    /// ends the run.
    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        ctx.pop()
    }

    // -- menu behaviour --------------------------------------------------------

    /// Rebuild the rows from `build_items`, keeping the cursor when
    /// `keep_index` is set.
    fn refresh(&mut self, ctx: &mut Ctx<G>, keep_index: bool) {
        let items = self.build_items(ctx);
        let core = self.menu_mut();
        core.set_items(items);
        if !keep_index || core.index >= core.items.len() {
            core.index = 0;
        }
    }

    /// `"Title. N items. First row"` on a fresh open, `"Title. Row"` when
    /// the cursor is already somewhere.
    fn announce(&mut self, ctx: &mut Ctx<G>) {
        let title = self.menu().title.clone();
        let n = self.menu().items.len();
        let label = self.current_text(ctx);
        let text = if self.menu().index == 0 {
            let items = if n == 1 { "1 item".to_string() } else { format!("{n} items") };
            format!("{title}. {items}. {label}")
        } else {
            format!("{title}. {label}")
        };
        ctx.say(text);
    }

    fn current_text(&self, ctx: &Ctx<G>) -> String {
        let core = self.menu();
        match core.items.get(core.index) {
            Some(item) => item.text(self, ctx),
            None => "No options".to_string(),
        }
    }

    fn current_help(&self, ctx: &Ctx<G>) -> Option<String> {
        let core = self.menu();
        core.items.get(core.index)?.help_text(self, ctx)
    }

    fn speak_current(&mut self, ctx: &mut Ctx<G>) {
        let text = self.current_text(ctx);
        ctx.say(text);
    }

    /// Move the cursor by `delta`, wrapping at both ends.
    fn move_by(&mut self, ctx: &mut Ctx<G>, delta: i32) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        let here = self.menu().index as i32;
        self.menu_mut().index = (here + delta).rem_euclid(n as i32) as usize;
        self.speak_current(ctx);
    }

    fn jump(&mut self, ctx: &mut Ctx<G>, index: usize) {
        let n = self.menu().items.len();
        if n == 0 {
            return;
        }
        self.menu_mut().index = index.min(n - 1);
        self.speak_current(ctx);
    }

    /// The next row after the cursor whose label starts with `c`, wrapping.
    /// Silent when nothing matches.
    fn letter_jump(&mut self, ctx: &mut Ctx<G>, c: char) {
        let n = self.menu().items.len();
        let start = self.menu().index;
        let wanted = c.to_ascii_lowercase();
        for offset in 1..=n {
            let i = (start + offset) % n;
            let text = self.menu().items[i].text(self, ctx);
            if text.chars().next().is_some_and(|f| f.to_ascii_lowercase() == wanted) {
                self.menu_mut().index = i;
                self.speak_current(ctx);
                return;
            }
        }
    }

    fn activate(&mut self, ctx: &mut Ctx<G>) {
        let i = self.menu().index;
        if i >= self.menu().items.len() {
            return;
        }
        // The action needs `&mut self`, so the rows are lent to it and handed
        // back afterwards -- unless the action installed rows of its own.
        let revision = self.menu().revision;
        let mut items = std::mem::take(&mut self.menu_mut().items);
        (items[i].action)(self, ctx);
        if self.menu().revision == revision {
            self.menu_mut().items = items;
        }
    }

    /// Left and Right: change the row in place and read it back. A row with
    /// no adjuster does and says nothing.
    fn adjust(&mut self, ctx: &mut Ctx<G>, delta: i32) {
        let i = self.menu().index;
        if !self.menu().items.get(i).is_some_and(|item| item.adjust.is_some()) {
            return;
        }
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

/// The navigation every menu shares. A screen with extra keys overrides
/// `handle_input` and calls this for everything it does not claim.
pub fn menu_input<G: Game, S: Menu<G>>(state: &mut S, ctx: &mut Ctx<G>, event: &InputEvent) {
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
        Key::Return | Key::KpEnter | Key::Space => state.activate(ctx),
        Key::Left => state.adjust(ctx, -1),
        Key::Right => state.adjust(ctx, 1),
        Key::Escape => state.on_escape(ctx),
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
/// work when the shell reported no typed text.
fn letter_of(key: Key) -> Option<char> {
    ('a'..='z').chain('0'..='9').find(|&c| Key::from_char(c) == key)
}

impl<G: Game, M: Menu<G>> State<G> for M {
    fn name(&self) -> &'static str {
        Menu::name(self)
    }
    fn enter(&mut self, ctx: &mut Ctx<G>) {
        Menu::enter(self, ctx)
    }
    fn exit(&mut self, ctx: &mut Ctx<G>) {
        Menu::exit(self, ctx)
    }
    fn resume(&mut self, ctx: &mut Ctx<G>) {
        Menu::resume(self, ctx)
    }
    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        Menu::handle_input(self, ctx, event)
    }
    fn on_event(&mut self, ctx: &mut Ctx<G>, event: &G::Event) {
        Menu::on_event(self, ctx, event)
    }
    fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
        Menu::update(self, ctx, dt)
    }
    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        Menu::lines(self, ctx)
    }
    fn help(&self, ctx: &Ctx<G>) -> String {
        Menu::help(self, ctx)
    }
}
```

- [ ] **Step 5: The screens**

`src/screens/mod.rs`:

```rust
//! Screens every game gets.

pub mod help;
pub mod pause;
pub mod settings;

pub use help::HelpScreen;
pub use pause::PauseScreen;
pub use settings::SettingsScreen;
```

`src/screens/settings.rs`:

```rust
//! The engine's settings. Every change is heard at once; Escape saves
//! `settings.json` and goes back.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Label, Menu, MenuCore, MenuItem};

/// A speech-rate step: five percent of the voice's range.
const RATE_STEP: f64 = 0.05;

/// The rate as the player hears it: a whole percentage.
pub fn rate_label(rate: f64) -> String {
    format!("Speech rate: {} percent", (rate * 100.0).round() as i64)
}

pub struct SettingsScreen<G: Game> {
    menu: MenuCore<Self, G>,
}

impl<G: Game> SettingsScreen<G> {
    pub fn new() -> Self {
        SettingsScreen {
            menu: MenuCore::new("Settings").with_intro_help(
                "Up and Down move, Left and Right change the setting, Escape saves and goes back.",
            ),
        }
    }
}

impl<G: Game> Default for SettingsScreen<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Menu<G> for SettingsScreen<G> {
    fn name(&self) -> &'static str {
        "settings"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        vec![MenuItem::inert(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
            rate_label(ctx.settings.speech.rate)
        }))
        .help("Left and Right change how fast the game speaks.")
        .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| {
            let next = ctx.settings.speech.rate + f64::from(delta) * RATE_STEP;
            // Rounded to whole percent so repeated steps never drift.
            ctx.settings.speech.rate = ((next * 100.0).round() / 100.0).clamp(0.0, 1.0);
            ctx.apply_speech_settings();
        })]
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        if let Err(e) = ctx.settings.save(&ctx.paths) {
            log::error!("could not save settings: {e}");
        }
        ctx.pop();
    }
}
```

`src/screens/help.rs`:

```rust
//! A game's help document read out a section at a time. The document is
//! compiled into the game, so help works before any data folder exists.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Menu, MenuCore, MenuItem};

/// One `## ` heading and the prose under it, joined into one line.
pub struct Section {
    pub title: String,
    pub body: String,
}

/// Split a Markdown document on `## ` headings. Anything before the first
/// one (the document's own title) is not a section.
pub fn sections(doc: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in doc.lines() {
        match line.strip_prefix("## ") {
            Some(title) => out.push(Section {
                title: title.trim().to_string(),
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

pub struct HelpScreen<G: Game> {
    menu: MenuCore<Self, G>,
    doc: &'static str,
}

impl<G: Game> HelpScreen<G> {
    pub fn new(doc: &'static str) -> Self {
        HelpScreen {
            menu: MenuCore::new("Help").with_intro_help(
                "Up and Down move between help topics, Enter reads one, Escape goes back.",
            ),
            doc,
        }
    }
}

impl<G: Game> Menu<G> for HelpScreen<G> {
    fn name(&self) -> &'static str {
        "help"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        sections(self.doc)
            .into_iter()
            .map(|section| {
                let body = section.body;
                MenuItem::new(section.title, move |_s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.say(body.clone())
                })
                .help("Press Enter to hear this topic.")
            })
            .collect()
    }
}
```

`src/screens/pause.rs`:

```rust
//! The pause menu: the way out of a game in progress.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Menu, MenuCore, MenuItem};
use crate::screens::{HelpScreen, SettingsScreen};

pub struct PauseScreen<G: Game> {
    menu: MenuCore<Self, G>,
    help_doc: &'static str,
    to_title: fn(&mut Ctx<G>),
}

impl<G: Game> PauseScreen<G> {
    /// `to_title` is what "Return to the title" does: usually saving and
    /// `ctx.reset_to(...)` the game's title screen.
    pub fn new(help_doc: &'static str, to_title: fn(&mut Ctx<G>)) -> Self {
        PauseScreen {
            menu: MenuCore::new("Paused").with_intro_help(
                "Up and Down move, Enter chooses, Escape goes back to the game.",
            ),
            help_doc,
            to_title,
        }
    }
}

impl<G: Game> Menu<G> for PauseScreen<G> {
    fn name(&self) -> &'static str {
        "pause"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        vec![
            MenuItem::new("Resume", |_s: &mut Self, ctx: &mut Ctx<G>| ctx.pop())
                .help("Go back to where you were."),
            MenuItem::new("Settings", |_s: &mut Self, ctx: &mut Ctx<G>| {
                ctx.push(SettingsScreen::<G>::new())
            })
            .help("Speech rate and other settings."),
            MenuItem::new("Help", |s: &mut Self, ctx: &mut Ctx<G>| {
                ctx.push(HelpScreen::<G>::new(s.help_doc))
            })
            .help("How the game works."),
            MenuItem::new("Return to the title", |s: &mut Self, ctx: &mut Ctx<G>| {
                (s.to_title)(ctx)
            })
            .help("Leave the game and go back to the title."),
            MenuItem::new("Quit", |_s: &mut Self, ctx: &mut Ctx<G>| ctx.quit())
                .help("Close the game."),
        ]
    }
}
```

Add `pub mod menu;` and `pub mod screens;` to `src/lib.rs`.

- [ ] **Step 6: Run the tests**

`cargo test -p hearken-app --test menu --test screens > target/t6.log 2>&1; echo exit=$?` — expected `exit=0`, 11 + 5 passed. Then re-run `--test app` and `--test foundations` once; still green.

- [ ] **Step 7: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings
git add -A
git commit -m "feat(app): spoken menus, and settings, help and pause screens

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: The window, logging, single instance, and `run`

**Files:**
- Create: `crates/hearken-app/build.rs`, `src/{sdl_shell.rs,logging.rs,single_instance.rs,run.rs}`
- Create: `vendor/sdl2/{LICENSE.txt,README.md,windows-x86_64/SDL2.dll,windows-x86_64/SDL2.lib}`
- Modify: `crates/hearken-app/Cargo.toml`, `src/lib.rs`
- Test: `crates/hearken-app/tests/platform.rs`

**Interfaces:**
- Consumes: everything above; `ThreadedSpeech::spawn`, `SpeechEnv::with_prefix` (Task 3).
- Produces:
  - `hearken_app::SdlShell::new(title: &str) -> Result<SdlShell, String>`, `hearken_app::sdl_shell::{translate_events, mods_from, key_from_keycode}`
  - `hearken_app::logging::init(paths: &Paths, env_prefix: &str)`
  - `hearken_app::single_instance::{SingleInstanceGuard, MutexApi, WinMutexApi, mutex_name, ERROR_ALREADY_EXISTS}`; `SingleInstanceGuard::new(name: &str)`, `with_api(name, api, windows)`, `acquire() -> bool`, `release()`, `acquired() -> bool`
  - `hearken_app::run::<G>(game: G::Data, first: impl FnOnce() -> Box<dyn State<G>>) -> i32`, `hearken_app::already_running_line::<G>() -> String`

- [ ] **Step 1: Vendor SDL2 and wire the dependency**

```bash
LB=C:/Users/joshu/gh-projects/lucky-break
mkdir -p vendor/sdl2/windows-x86_64
for f in LICENSE.txt README.md windows-x86_64/SDL2.dll windows-x86_64/SDL2.lib; do
  git -C $LB show a101970:vendor/sdl2/$f > vendor/sdl2/$f
done
```

Add to `crates/hearken-app/Cargo.toml`:

```toml
build = "build.rs"

[target.'cfg(windows)'.dependencies]
sdl2.workspace = true
windows-sys.workspace = true

[target.'cfg(target_os = "macos")'.dependencies]
sdl2 = { workspace = true, features = ["bundled", "static-link"] }

[target.'cfg(target_os = "linux")'.dependencies]
sdl2 = { workspace = true, features = ["bundled", "static-link"] }
```

and `env_logger.workspace = true` under `[dependencies]`. (`build = "build.rs"` goes in `[package]`.)

`crates/hearken-app/build.rs`: FF's SDL2 block (`m1-bass-sys.md` §7, FF `main` lines 832-860) as the whole `main`, with the warning text changed to `hearken-app: no vendored SDL2 for {os}-{arch} under vendor/sdl2; expecting a system SDL2` and the header:

```rust
//! Put the vendored SDL2 import library on the link path and stage its DLL
//! beside the build output. macOS and Linux compile SDL2 in statically
//! instead (see Cargo.toml), so they have nothing vendored.
```

- [ ] **Step 2: Write the failing tests**

`crates/hearken-app/tests/platform.rs`:

```rust
mod common;

use std::cell::Cell;

use common::TestGame;
use hearken_app::sdl_shell::translate_events;
use hearken_app::single_instance::{MutexApi, SingleInstanceGuard, ERROR_ALREADY_EXISTS};
use hearken_app::{already_running_line, InputEvent, Key, Mods};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};

#[test]
fn a_key_press_carries_the_text_typed_with_it_and_its_repeat_flag() {
    let raw = vec![
        Event::KeyDown {
            timestamp: 0,
            window_id: 0,
            keycode: Some(Keycode::A),
            scancode: None,
            keymod: Mod::LSHIFTMOD,
            repeat: true,
        },
        Event::TextInput { timestamp: 0, window_id: 0, text: "A".to_string() },
    ];
    assert_eq!(
        translate_events(raw),
        [InputEvent::KeyDown { key: Key::A, mods: Mods::SHIFT, text: Some('A'), repeat: true }]
    );
}

#[test]
fn window_and_quit_events_translate() {
    let raw = vec![
        Event::Window { timestamp: 0, window_id: 0, win_event: WindowEvent::FocusLost },
        Event::Window { timestamp: 0, window_id: 0, win_event: WindowEvent::FocusGained },
        Event::Quit { timestamp: 0 },
    ];
    assert_eq!(
        translate_events(raw),
        [InputEvent::WindowFocusLost, InputEvent::WindowFocusGained, InputEvent::Quit]
    );
}

struct FakeKernel {
    exists: bool,
    closed: Cell<u32>,
}

impl MutexApi for FakeKernel {
    fn create_mutex(&self, _name: &str) -> (usize, u32) {
        (42, if self.exists { ERROR_ALREADY_EXISTS } else { 0 })
    }
    fn close_handle(&self, _handle: usize) {
        self.closed.set(self.closed.get() + 1);
    }
}

#[test]
fn a_second_copy_is_refused_and_closes_its_handle() {
    let mut guard = SingleInstanceGuard::with_api("x", FakeKernel { exists: true, closed: Cell::new(0) }, true);
    assert!(!guard.acquire());
    assert!(!guard.acquired());
}

#[test]
fn the_first_copy_holds_the_lock_until_released() {
    let kernel = FakeKernel { exists: false, closed: Cell::new(0) };
    let mut guard = SingleInstanceGuard::with_api("x", kernel, true);
    assert!(guard.acquire());
    guard.release();
    assert!(!guard.acquired());
}

#[cfg(windows)]
#[test]
fn two_real_guards_on_one_name_exclude_each_other() {
    let name = hearken_app::single_instance::mutex_name("HearkenPlatformTest");
    let mut first = SingleInstanceGuard::new(&name);
    assert!(first.acquire());
    let mut second = SingleInstanceGuard::new(&name);
    assert!(!second.acquire());
    first.release();
    let mut third = SingleInstanceGuard::new(&name);
    assert!(third.acquire());
}

#[test]
fn the_already_running_line_names_the_game() {
    assert_eq!(already_running_line::<TestGame>(), "Test Game is already running.");
}

#[test]
fn the_log_file_goes_to_the_data_folder() {
    let dir = tempfile::tempdir().unwrap();
    let paths = hearken_app::Paths::at(dir.path());
    hearken_app::logging::init(&paths, "HEARKEN_TEST_PLATFORM");
    log::info!(target: hearken_app::TRANSCRIPT_TARGET, "a spoken line");
    log::logger().flush();
    let text = std::fs::read_to_string(paths.log_file()).unwrap();
    assert!(text.contains("a spoken line"), "log: {text}");
}
```

Add `sdl2.workspace = true`, `log.workspace = true` and `tempfile.workspace = true` to `[dev-dependencies]` of `hearken-app` if they are not resolvable from the tests.

- [ ] **Step 3: Run to confirm they fail**

`cargo test -p hearken-app --test platform > target/t7.log 2>&1; echo exit=$?` — expected: compile errors naming `sdl_shell`, `single_instance`, `already_running_line`, `logging`.

- [ ] **Step 4: `sdl_shell.rs`**

```rust
//! The SDL window: a focus target for the keyboard and a mirror of the
//! top screen's last line in its title for a sighted helper.

use std::time::{Duration, Instant};

use sdl2::controller::{Axis, Button};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::pixels::Color;
use sdl2::render::WindowCanvas;
use sdl2::EventPump;

use crate::input::{ControllerAxis, ControllerButton, InputEvent, Key, Mods};
use crate::shell::{Shell, FRAME_SECONDS};

pub struct SdlShell {
    _sdl: sdl2::Sdl,
    canvas: WindowCanvas,
    pump: EventPump,
    title: String,
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
        Ok(SdlShell { _sdl: sdl, canvas, pump, title: title.to_string(), last: Instant::now() })
    }
}

impl Shell for SdlShell {
    fn poll(&mut self) -> Vec<InputEvent> {
        translate_events(self.pump.poll_iter().collect())
    }

    fn render(&mut self, lines: &[String]) {
        self.canvas.set_draw_color(Color::RGB(12, 24, 16));
        self.canvas.clear();
        self.canvas.present();
        if let Some(last) = lines.last() {
            let _ = self.canvas.window_mut().set_title(&format!("{} - {last}", self.title));
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
```

Then append, verbatim from FF (`git -C C:/Users/joshu/Freight-fate show c5d3ef8d:crates/freight-fate/src/app/sdl_shell.rs`): `translate_events` (in `m1-ff-menus-input.md` §5 "SDL keyboard/controller -> InputEvent translation"), `mods_from`, and `key_from_keycode` (FF lines 355-454); and from `git ... show c5d3ef8d:crates/freight-fate/src/controller/sdl.rs` (verbatim in the notes §5): `button_from_sdl` and `axis_from_sdl`. Make all five `pub`.

- [ ] **Step 5: `logging.rs` and `single_instance.rs`**

`src/logging.rs`: LB's `init_logging` and `Tee` (`m1-lb-app.md` §5.9) with these changes: the function is `pub fn init(paths: &Paths, env_prefix: &str)`; the override is read from `format!("{env_prefix}_LOG_FILE")`; the `LOG_FILE_ENV` constant is dropped; the doc comment reads:

```rust
/// Every log record goes to stderr and, when it can be opened, to the log
/// file (`<env_prefix>_LOG_FILE`, else `paths.log_file()`), so a finished
/// session leaves its transcript behind. Safe to call twice: the second
/// call finds a logger already set and changes nothing.
```

`src/single_instance.rs`: FF's file verbatim (`m1-ff-menus-input.md` §7), with these changes: module doc `//! One running copy of a game per Windows session. Everything fails open: a mutex the OS will not create never keeps a player out.`; delete `SINGLE_INSTANCE_MUTEX_NAME` and the `Default` impl; `new` becomes:

```rust
impl SingleInstanceGuard<WinMutexApi> {
    /// The guard a game uses: the real kernel32 on Windows, a no-op
    /// elsewhere.
    pub fn new(mutex_name: &str) -> Self {
        Self::with_api(mutex_name, WinMutexApi, cfg!(windows))
    }
}

/// The session-local mutex name for a game's data folder name.
pub fn mutex_name(data_dir: &str) -> String {
    format!("Local\\{data_dir}.SingleInstance")
}
```

and rewrite the doc comments that mention Python or Freight Fate.

- [ ] **Step 6: `run.rs`**

```rust
//! Boot a game: data folder, log, single instance, settings, speech,
//! window, then the loop.

use hearken_speech::{SpeechEnv, SpeechSink, ThreadedSpeech};

use crate::app::App;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::paths::Paths;
use crate::sdl_shell::SdlShell;
use crate::settings::Settings;
use crate::single_instance::{mutex_name, SingleInstanceGuard};
use crate::state::State;

/// What a second copy says before it closes.
pub fn already_running_line<G: Game>() -> String {
    format!("{} is already running.", G::TITLE)
}

fn start_speech<G: Game>() -> Box<dyn SpeechSink> {
    Box::new(ThreadedSpeech::spawn(SpeechEnv::with_prefix(G::ENV_PREFIX)))
}

/// Run `G` until the player quits. Returns the process exit code.
pub fn run<G: Game>(game: G::Data, first: impl FnOnce() -> Box<dyn State<G>>) -> i32 {
    let paths = Paths::discover(G::DATA_DIR, G::ENV_PREFIX);
    crate::logging::init(&paths, G::ENV_PREFIX);

    let mut guard = SingleInstanceGuard::new(&mutex_name(G::DATA_DIR));
    if !guard.acquire() {
        let line = already_running_line::<G>();
        log::warn!("{line}");
        let mut speech = start_speech::<G>();
        speech.say(&line, true);
        speech.shutdown();
        return 0;
    }

    let (settings, notice) = Settings::load(&paths);
    let mut ctx = Ctx::new(start_speech::<G>(), settings, paths, game);
    ctx.notices.extend(notice);
    ctx.apply_speech_settings();

    let mut shell = match SdlShell::new(G::TITLE) {
        Ok(shell) => shell,
        Err(e) => {
            log::error!("could not open a window: {e}");
            ctx.speech.shutdown();
            return 2;
        }
    };
    let mut app = App::new(ctx, first());
    app.run(&mut shell);
    guard.release();
    0
}
```

Add to `src/lib.rs`: `pub mod logging; pub mod run; pub mod sdl_shell; pub mod single_instance;` and `pub use run::{already_running_line, run}; pub use sdl_shell::SdlShell;`.

- [ ] **Step 7: Run the tests**

`cargo test -p hearken-app --test platform > target/t7.log 2>&1; echo exit=$?` — expected `exit=0`, 7 passed (6 off Windows).

- [ ] **Step 8: Gates and commit**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings
git add -A
git commit -m "feat(app): SDL window, session log, single instance and run

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The template game, README, and the full sweep

**Files:**
- Create: `template/{Cargo.toml,build.rs,help.md,src/main.rs,src/lib.rs,src/title.rs,src/practice.rs,tests/walkthrough.rs}`
- Modify: root `Cargo.toml` (`members = ["crates/*", "template"]`), `README.md`
- Modify: `docs/superpowers/specs/2026-09-26-hearken-engine-design.md` (only if anything in this plan's "Deferred" list is not reflected there)

**Interfaces:**
- Consumes: everything from Tasks 3-7.
- Produces: package `hello-hearken` (binary `hello-hearken`, library `hello_hearken` with `HelloGame`, `Title`, `Practice`, `HELP`).

- [ ] **Step 1: Write the failing walkthrough**

`template/tests/walkthrough.rs`:

```rust
use hearken_app::{Headless, Key};
use hello_hearken::{HelloGame, Title};

fn start() -> Headless<HelloGame> {
    Headless::new(Default::default(), Title::new())
}

#[test]
fn the_title_announces_itself() {
    let h = start();
    assert_eq!(h.spoken(), ["Hello Hearken. 4 items. Play"]);
}

#[test]
fn settings_open_and_the_title_comes_back_where_it_was() {
    let mut h = start();
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.spoken().last().map(String::as_str), Some("Settings. 1 item. Speech rate: 50 percent"));
    h.press(Key::Escape);
    assert_eq!(h.spoken().last().map(String::as_str), Some("Hello Hearken. Settings"));
}

#[test]
fn practice_counts_and_pauses_and_returns_to_the_title() {
    let mut h = start();
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "practice");
    h.clear();
    h.press(Key::Space);
    h.press(Key::Space);
    assert_eq!(h.spoken(), ["1", "2"]);
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "pause");
    for _ in 0..3 {
        h.press(Key::Down);
    }
    h.press(Key::Return);
    assert_eq!((h.app.top_name(), h.app.depth()), ("title", 1));
}

#[test]
fn escape_on_the_title_says_how_to_leave_and_quit_leaves() {
    let mut h = start();
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), ["Press Enter on Quit to leave."]);
    assert!(h.app.ctx.running);
    h.press(Key::End);
    h.press(Key::Return);
    assert!(!h.app.ctx.running);
}
```

- [ ] **Step 2: The game**

`template/Cargo.toml`:

```toml
[package]
name = "hello-hearken"
description = "The smallest complete Hearken game: a title menu, settings, help, and a practice room"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true
publish = false
build = "build.rs"

[dependencies]
hearken-app.workspace = true
# A direct dependency, so build.rs sees Prism's delay-load list.
hearken-speech.workspace = true
```

`template/build.rs`:

```rust
//! Delay-load the screen reader DLLs behind Prism's Windows backends.
//!
//! Without this every screen reader's client DLL becomes a hard import and
//! the game will not start on a machine missing any of them. A link flag
//! only applies to the final link, which is this crate's, so every game
//! needs this build script.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let Ok(dlls) = std::env::var("DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS") else {
        return;
    };
    for dll in dlls.split(';').filter(|dll| !dll.is_empty()) {
        println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
    }
}
```

`template/help.md`:

```markdown
# Hello Hearken

## Getting around

Up and Down move through a menu, Enter chooses, and Escape goes back. F1
reads help for wherever you are, and F2 stops speech.

## The practice room

Press Space to count out loud. Press Escape for the pause menu.

## Leaving

Choose Quit on the title menu, or Quit from the pause menu.
```

`template/src/lib.rs`:

```rust
//! Hello Hearken: the smallest complete Hearken game.

mod practice;
mod title;

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
}
```

`template/src/title.rs`:

```rust
use hearken_app::menu::{Menu, MenuCore, MenuItem};
use hearken_app::screens::{HelpScreen, SettingsScreen};
use hearken_app::Ctx;

use crate::{HelloGame, Practice, HELP};

pub struct Title {
    menu: MenuCore<Self, HelloGame>,
}

impl Title {
    pub fn new() -> Self {
        Title { menu: MenuCore::new("Hello Hearken") }
    }
}

impl Default for Title {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu<HelloGame> for Title {
    fn name(&self) -> &'static str {
        "title"
    }

    fn menu(&self) -> &MenuCore<Self, HelloGame> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, HelloGame> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<HelloGame>) -> Vec<MenuItem<Self, HelloGame>> {
        vec![
            MenuItem::new("Play", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| ctx.push(Practice::new()))
                .help("Go to the practice room."),
            MenuItem::new("Settings", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| {
                ctx.push(SettingsScreen::<HelloGame>::new())
            })
            .help("Speech rate."),
            MenuItem::new("Help", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| {
                ctx.push(HelpScreen::<HelloGame>::new(HELP))
            })
            .help("How to play."),
            MenuItem::new("Quit", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| ctx.quit())
                .help("Close the game."),
        ]
    }

    /// The root of the stack never pops: that would end the run with no
    /// warning.
    fn on_escape(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.say("Press Enter on Quit to leave.");
    }
}
```

`template/src/practice.rs`:

```rust
use hearken_app::screens::PauseScreen;
use hearken_app::{Ctx, InputEvent, Key, State};

use crate::{HelloGame, Title, HELP};

/// A room with one thing to do: count out loud.
pub struct Practice {
    count: u32,
}

impl Practice {
    pub fn new() -> Self {
        Practice { count: 0 }
    }
}

impl Default for Practice {
    fn default() -> Self {
        Self::new()
    }
}

fn to_title(ctx: &mut Ctx<HelloGame>) {
    ctx.reset_to(Title::new());
}

impl State<HelloGame> for Practice {
    fn name(&self) -> &'static str {
        "practice"
    }

    fn enter(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.say("The practice room. Space counts, Escape pauses.");
    }

    fn resume(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.say(format!("The practice room. The count is {}.", self.count));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<HelloGame>, event: &InputEvent) {
        let Some((key, _, _)) = event.key_down() else {
            return;
        };
        match key {
            Key::Space => {
                self.count += 1;
                ctx.say(self.count.to_string());
            }
            Key::Escape => ctx.push(PauseScreen::<HelloGame>::new(HELP, to_title)),
            _ => {}
        }
    }

    fn lines(&self, _ctx: &Ctx<HelloGame>) -> Vec<String> {
        vec![format!("Practice room. Count: {}", self.count)]
    }

    fn help(&self, _ctx: &Ctx<HelloGame>) -> String {
        "Space counts out loud. Escape opens the pause menu.".to_string()
    }
}
```

`template/src/main.rs`:

```rust
#![cfg_attr(windows, windows_subsystem = "windows")]
//! `hello-hearken`: the game's entry point.

fn main() {
    std::process::exit(hearken_app::run::<hello_hearken::HelloGame>((), || {
        Box::new(hello_hearken::Title::new())
    }));
}
```

In root `Cargo.toml`: `members = ["crates/*", "template"]`.

- [ ] **Step 3: Run the walkthrough**

`cargo test -p hello-hearken > target/t8.log 2>&1; echo exit=$?` — expected `exit=0`, 4 passed.

- [ ] **Step 4: README**

Replace `README.md` with:

```markdown
# Hearken

Rust crates for audio-first, screen-reader-first games, extracted from
Freight Fate and Lucky Break Casino. Work in progress: this is milestone 1
of `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`.

## What is here

- `hearken-app`: the window, the screen stack, spoken menus, settings,
  help and pause screens, and a headless harness for tests.
- `hearken-speech`: screen reader, braille and text-to-speech output
  through Prism.
- `hearken-bass-sys`: bindings to the BASS audio library, loaded at run
  time, including streams fed by a Rust closure.
- `template/`: Hello Hearken, the smallest complete game.

## Building

1. Install Rust with rustup; `rust-toolchain.toml` picks the version.
2. Install CMake 3.24 or newer and a C++23 compiler (Visual Studio 2022 on
   Windows). Prism is compiled from source the first time. To skip that,
   set `PRISM_LIB_DIR` to a folder holding a prebuilt Prism library.
3. `python tools/fetch_bass.py` downloads BASS. BASS is free for
   non-commercial use; a commercial game needs its own licence from
   un4seen. Hearken never bundles it.
4. `cargo run -p hello-hearken` starts the template game. Turn your screen
   reader on first; without one, Windows' SAPI voices speak instead.

Every game needs a `build.rs` like `template/build.rs`, so the screen reader
DLLs are delay-loaded and the game still starts on machines that lack some
of them.

## Licence

MIT or Apache-2.0, at your option. BASS and Prism's screen reader clients
keep their own licences.
```

- [ ] **Step 5: The full sweep, once**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked > target/full.log 2>&1; echo exit=$?
actionlint
```

Expected: fmt and clippy silent; `exit=0` with every test binary green in `target/full.log`; actionlint silent. Run the test command in the background.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(template): Hello Hearken, the first game on the engine

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: The owner's listening pass**

Build it (`cargo build -p hello-hearken`), then ask Josh to run `cargo run -p hello-hearken` with NVDA on and listen for: the title announcement, arrowing through the four rows, F1 on a row, Settings with Left and Right on the speech rate (the voice should speed up and slow down), the help topics, Space in the practice room, the pause menu, and Quit. Recorded speech in tests does not establish how it sounds; only this pass does. Report what he hears; fix anything that is wrong before calling M1 done.
```

## Self-review notes (for the plan author, resolved)

- Spec coverage for M1: workspace, `hearken-bass-sys` (Tasks 1-2), `hearken-speech` (Task 3), `hearken-app` shell/stack/menus/built-in screens (Tasks 4-7), bare template with one headless test (Task 8). Key rebinding, controller support, held keys and text entry moved to M1b; Learn Sounds, menu sounds and the event ladder/pacer/duck to M2, stated under "Deferred".
- Type consistency checked: `Ctx::say_event(text, interrupt)`, `State::help(&self, &Ctx<G>)`, `Menu<G>` with `name`, `Headless::{press, send, frame, spoken, clear}`, `Settings<GS>::{load, save}`, `SpeechSettings.rate`.
