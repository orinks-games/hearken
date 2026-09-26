# M1 research: extracting `hearken-bass-sys` from Lucky Break's `bass-sys`

Status: COMPLETE (2026-09-26). Research only; no source repo was built, tested or edited.

- Source: `C:\Users\joshu\gh-projects\lucky-break\crates\bass-sys` @ `a1019704bfaf1285705c07492ca13305292aef72` (branch `main`, HEAD).
- Ancestor: `C:\Users\joshu\Freight-fate\crates\bass-sys` @ `c5d3ef8d0cc70b43cb74589fa92b05424a088dad` (branch `dev`).
- All paths below are relative to the LB crate root unless prefixed.

## 1. Files and Cargo.toml

### LB `crates/bass-sys` (git-tracked files)

| Path | Lines | Purpose |
|---|---|---|
| `Cargo.toml` | 14 | Package manifest (verbatim below). |
| `build.rs` | 115 | Copies everything in `vendor/<os>-<arch>/` (minus `.lib/.txt/.md/.h`) into `target/<profile>/`; warns if the core DLL is missing and deletes stale staged copies. Links nothing. |
| `src/lib.rs` | 758 | Primitive aliases (`DWORD`, `HSTREAM`...), all constants (errors, config, flags, attribs, 3D, BFX, sync, pos, tags, ctype), `#[repr(C)]` structs (`BASS_DEVICEINFO`, `BASS_CHANNELINFO`, `BASS_3DVECTOR`, `BASS_BFX_BQF`), callback types (`SYNCPROC`, `DOWNLOADPROC`), one `Fn*` fn-pointer type per symbol, `native_available()`, `api()`, layout/constant unit tests. |
| `src/loader.rs` | 396 | Runtime loader: `LIBRARY_PATH_ENV(_LEGACY)`, `library_file_name()`, `search_paths()`, `LoadError`, `Api` struct of fn pointers resolved once into a `OnceLock`, unit tests. |
| `src/fx.rs` | 156 | Runtime loader for `bass_fx` (separate lib, not a plugin): `load_bass_fx(dir)`, `bass_fx_loaded()`, own `library_file_name()`; calls `BASS_FX_GetVersion` to hook into BASS. Unit tests. |
| `src/safe.rs` | 1122 | Safe wrappers: `BassError` (`NOT_LOADED = -1000`), lifecycle/config/devices/plugins, `Stream` (owns `Arc<[u8]>` for mem streams, frees on Drop), channel control, 3D, FX (`fx_set_bqf`), tags, `SyncGuard` + `set_sync` (boxed closure trampoline). Unit tests. |
| `tests/smoke.rs` | 446 | Integration tests against the real DLL on the no-sound device; skip when not loadable. |
| `vendor/licenses/README.md` | 72 | Explains BASS is fetched, not committed; pin provenance; licence position. |

Untracked (gitignored by LB `.gitignore:10` `crates/bass-sys/vendor/*/bass*.dll`):
`vendor/windows-x86_64/bass.dll` (166872 bytes), `vendor/windows-x86_64/bass_fx.dll` (88064 bytes).

No `links = ...` key, no features, no dev-dependencies.

### `Cargo.toml` (verbatim, identical in LB and FF)

```toml
[package]
name = "bass-sys"
description = "Raw FFI bindings to the BASS audio library, loaded at run time"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license = "MIT"
repository.workspace = true
authors.workspace = true
build = "build.rs"

[dependencies]
libloading.workspace = true
log.workspace = true
```

### FF `crates/bass-sys` for comparison

| Path | Lines |
|---|---|
| `Cargo.toml` | 14 (identical) |
| `build.rs` | 108 |
| `src/lib.rs` | 605 |
| `src/loader.rs` | 335 |
| `src/safe.rs` | 1000 |
| `tests/smoke.rs` | 443 |
| `vendor/licenses/README.md` | - |

FF has no `src/fx.rs`. FF vendor dir (`vendor/windows-x86_64/`): `bass.dll`, `bass_aac.dll`, `bassflac.dll`, `basshls.dll`, `basshls.txt`, `bassopus.dll`.

### LB is NOT a strict superset of FF

LB added fx + 3D, but **dropped** these FF items, which FF's game still uses:

- FF `src/lib.rs:231-243` MOD music flags:
  ```rust
  // --- MOD music flags (BASS_MusicLoad) -----------------------------------------

  /// Calculate the length on load, so `ChannelGetLength` answers
  /// (`BASS_MUSIC_PRESCAN` is `BASS_STREAM_PRESCAN` in bass.h).
  pub const BASS_MUSIC_PRESCAN: DWORD = BASS_STREAM_PRESCAN;
  /// Sensitive volume ramping, to avoid clicks.
  pub const BASS_MUSIC_RAMPS: DWORD = 0x400;
  /// Sinc-interpolated sample mixing.
  pub const BASS_MUSIC_SINCINTER: DWORD = 0x80_0000;
  /// Stop the music on a backward jump effect (`Bxx` to an earlier order), so
  /// a module written to loop forever ends after one pass.
  pub const BASS_MUSIC_STOPBACK: DWORD = 0x8_0000;
  ```
- FF `src/lib.rs:332-334`:
  ```rust
  /// BASSHLS: the current segment's `#EXTINF` line after the colon
  /// (`duration,title`): one UTF-8 string.
  pub const BASS_TAG_HLS_EXTINF: DWORD = 0x14000;
  ```
- FF `src/lib.rs:475-484`:
  ```rust
  /// `HMUSIC BASS_MusicLoad(BOOL mem, const void *file, QWORD offset, DWORD length, DWORD flags, DWORD freq)`
  pub type FnMusicLoad = unsafe extern "system" fn(
      mem: BOOL,
      file: *const c_void,
      offset: QWORD,
      length: DWORD,
      flags: DWORD,
      freq: DWORD,
  ) -> HMUSIC;
  pub type FnMusicFree = unsafe extern "system" fn(handle: HMUSIC) -> BOOL;
  ```
- FF `src/loader.rs:150-151` fields `music_load: FnMusicLoad`, `music_free: FnMusicFree`; `:279-280` `sym!("BASS_MusicLoad")`, `sym!("BASS_MusicFree")`.
- FF `src/safe.rs`: `Stream` has a `music: bool` field (`:396-397`) and `fn free_handle(&self, a: &Api) -> BOOL` (`:432-444`, `BASS_MusicFree` vs `BASS_StreamFree`); `pub fn music_load_mem_shared(data: Arc<[u8]>, flags: u32) -> Result<Stream, BassError>` (`:488`); `pub fn tags_string(handle: u32, kind: u32) -> Option<String>` (`:765`, `tags_meta` delegates to it).
- FF `src/lib.rs:599-603` test asserts for the music constants; FF `tests/smoke.rs:299-382` `tiny_mod()`, `looping_mod()`, tests `a_tracker_module_loads_reports_its_length_and_frees_as_music` (`:318`) and `stopback_ends_a_module_that_jumps_back_to_the_start` (`:358`).
- FF `is_plugin_file_name` (`safe.rs:~332`) is `lower.starts_with("bass") && lower.ends_with(".dll") && lower != "bass.dll"` (no `bass_fx` exclusion); LB's excludes `bass_fx.dll` / `libbass_fx.*`.

FF callers of those items (outside bass-sys):
- `C:\Users\joshu\Freight-fate\crates\freight-fate\src\audio\bass.rs:28` imports `BASS_MUSIC_PRESCAN, BASS_MUSIC_RAMPS, BASS_MUSIC_SINCINTER, BASS_MUSIC_STOPBACK`; `:375` `safe::music_load_mem_shared(data, flags | BASS_STREAM_AUTOFREE)`.
- `C:\Users\joshu\Freight-fate\crates\freight-fate\src\audio\bass_radio.rs:9` imports `BASS_TAG_HLS_EXTINF`; `:367` `safe::tags_string(handle, BASS_TAG_HLS_EXTINF)`.

LB callers: only `crates/lucky-break` (`Cargo.toml:26` `bass-sys = { path = "../bass-sys" }`; `src/audio/bass3d.rs:30-31`, `:236-237` `bass_sys::api().ok().and_then(|a| a.library_dir())` then `bass_sys::fx::load_bass_fx(dir)`).

## 2. Game-specific strings, env vars, DLL search order

### Env-var constants (LB `src/loader.rs:31-37`, verbatim)

```rust
/// Environment variable naming the library (file or directory) to load.
pub const LIBRARY_PATH_ENV: &str = "LUCKY_BREAK_BASS_PATH";

/// The name this crate used in Freight Fate, still read when
/// [`LIBRARY_PATH_ENV`] is unset so an existing developer environment keeps
/// working.
pub const LIBRARY_PATH_ENV_LEGACY: &str = "FREIGHT_FATE_BASS_PATH";
```

Re-exported at `src/lib.rs:30`:
```rust
pub use loader::{library_file_name, Api, LoadError, LIBRARY_PATH_ENV, LIBRARY_PATH_ENV_LEGACY};
```
FF equivalents: `src/loader.rs:31` `pub const LIBRARY_PATH_ENV: &str = "FREIGHT_FATE_BASS_PATH";` and `src/lib.rs:29` `pub use loader::{library_file_name, Api, LoadError, LIBRARY_PATH_ENV};` (no legacy const).

Note: despite the doc comment ("still read when LIBRARY_PATH_ENV is unset"), the code pushes BOTH when both are set (LB first, FF second); the test `both_override_env_vars_are_honoured_in_order` asserts exactly that.

### Search order (LB `src/loader.rs:10-21` doc, then `:50-100` code, verbatim)

```rust
//! Discovery order, most specific first:
//!
//! 1. `LUCKY_BREAK_BASS_PATH` -- a file, or a directory containing the
//!    library. Set by developers and by the playtest tooling. The Freight
//!    Fate spelling this crate came from is still honoured as a fallback.
//! 2. the executable's directory (a frozen release, or `target/<profile>/`
//!    where `build.rs` staged the vendored copy), and its parent for test
//!    binaries living in `target/<profile>/deps/`
//! 3. the crate's own `vendor/<os>-<arch>/` directory (a dev checkout where
//!    nothing has been staged yet)
//! 4. the bare file name, so the platform loader gets a chance to find a
//!    system-wide install.
```

```rust
/// The crate's vendor directory for the build target, as known at compile
/// time. Exists in a dev checkout; harmlessly absent from a release install.
fn vendor_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("vendor")
        .join(format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
}

/// Candidate locations for the library, most specific first.
fn search_paths() -> Vec<PathBuf> {
    let file_name = library_file_name();
    let mut candidates = Vec::new();

    for var in [LIBRARY_PATH_ENV, LIBRARY_PATH_ENV_LEGACY] {
        let Ok(explicit) = std::env::var(var) else {
            continue;
        };
        if explicit.is_empty() {
            continue;
        }
        let explicit = PathBuf::from(explicit);
        if explicit.is_dir() {
            candidates.push(explicit.join(file_name));
        } else {
            candidates.push(explicit);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(file_name));
            // Test binaries live in target/<profile>/deps/; build.rs stages
            // into target/<profile>/.
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join(file_name));
            }
            // Packaged macOS bundles keep native libraries beside the app
            // resources rather than next to the executable.
            candidates.push(dir.join("..").join("Frameworks").join(file_name));
            candidates.push(dir.join("lib").join(file_name));
        }
    }

    candidates.push(vendor_dir().join(file_name));
    candidates.push(PathBuf::from(file_name));
    candidates
}
```

Concrete order: `$LUCKY_BREAK_BASS_PATH` (file, or dir/bass.dll) -> `$FREIGHT_FATE_BASS_PATH` -> `<exe_dir>/bass.dll` -> `<exe_dir>/../bass.dll` -> `<exe_dir>/../Frameworks/bass.dll` -> `<exe_dir>/lib/bass.dll` -> `<CARGO_MANIFEST_DIR>/vendor/<os>-<arch>/bass.dll` (compile-time path) -> bare `bass.dll`.
`library_file_name()` (`loader.rs:39-48`): `bass.dll` / `libbass.dylib` / `libbass.so`.

`Api::load` (`loader.rs:237-260`) tries candidates in order; first that `Library::new` opens is resolved. If it opens but a symbol is missing it returns `LoadError::MissingSymbol` immediately (does NOT try later candidates). Result (success or failure) cached in a `static OnceLock` for the process (`loader.rs:218-221`).

`bass_fx` search (LB `src/fx.rs:41-61`): `dir` argument (callers pass `Api::library_dir()`) -> `<exe_dir>/bass_fx.dll` -> `<exe_dir>/../bass_fx.dll` -> bare `bass_fx.dll`. No env var, no vendor dir. Cached in `static FX: OnceLock<Option<Library>>` (first call's `dir` wins).

### Every game-specific string/item in LB bass-sys (grep `freight|lucky|casino|python|pybass|sound_lib|sustain|radio|ignition|occlusion|game`, case-insensitive)

Code / strings (functional):
- `src/loader.rs:32` `pub const LIBRARY_PATH_ENV: &str = "LUCKY_BREAK_BASS_PATH";`
- `src/loader.rs:37` `pub const LIBRARY_PATH_ENV_LEGACY: &str = "FREIGHT_FATE_BASS_PATH";`
- `src/loader.rs:362` `        std::env::set_var(LIBRARY_PATH_ENV, "/tmp/lucky-break-bass-lib");`
- `src/loader.rs:363` `        std::env::set_var(LIBRARY_PATH_ENV_LEGACY, "/tmp/freight-fate-bass-lib");`
- `src/loader.rs:369` `            Some(&PathBuf::from("/tmp/lucky-break-bass-lib")),`
- `src/loader.rs:370` `            "the casino's own variable outranks the inherited one"`
- `src/loader.rs:374` `            Some(&PathBuf::from("/tmp/freight-fate-bass-lib"))`
- `build.rs:76` `             {target_os}-{target_arch}. Run \`uv run python tools/fetch_bass.py\` \`
- `build.rs:77` `             -- BASS is fetched, not committed. Until then the game runs without \`
- `src/fx.rs:106` `        log::warn!("bass_fx not found; occlusion will use volume only");`
- `src/lib.rs:748` `    fn flag_values_match_pybass() {` (test name)

Doc comments only:
- `build.rs:10` `//! library the occlusion filter needs -- into the Cargo target directory, so`
- `build.rs:71` `        // A developer who has not run the fetch gets a silent game and no`
- `src/fx.rs:10` `//! falls back to volume-only occlusion, exactly as a missing \`bass.dll\``
- `src/fx.rs:68` `/// directory gets the first result. Never panics and never stops the game:`
- `src/lib.rs:8` `//! BASS still starts the game -- it just loses audio, exactly as the Python`
- `src/lib.rs:11` `//! Only the entry points Freight Fate calls are declared: the surface is the`
- `src/lib.rs:12` `//! one the Python \`_BassBackend\` and \`SustainLoop\` use (memory streams, URL`
- `src/lib.rs:228` `/// game is created with this; the "fade to -1 volume" idiom relies on it.`
- `src/lib.rs:231` `/// Download and play an internet stream in small blocks (radio).`
- `src/lib.rs:280` `/// declared: it is the one the casino uses, as a low pass for occlusion.`
- `src/lib.rs:523` `/// is explicitly allowed (it is what the sustain-loop sync does).`
- `src/lib.rs:529` `/// The game passes NULL for this; the type exists so the \`BASS_StreamCreateURL\``
- `src/lib.rs:645` `/// the game use.`
- `src/lib.rs:749` `        // Spot checks against the values the Python game has been passing.`
- `src/loader.rs:4` `//! stop the game from starting (the Python build falls back to a null audio`
- `src/loader.rs:12-14` (search-order doc, quoted above)
- `src/loader.rs:34` `/// The name this crate used in Freight Fate, still read when`
- `src/loader.rs:136` `/// Only the symbols Freight Fate actually calls are resolved. Adding a new call`
- `src/safe.rs:3` `//! This is the layer the game's \`BassAudio\` backend calls. It turns BASS's`
- `src/safe.rs:9` `//! for a stale one, which is how the Python game treats "already autofreed"`
- `src/safe.rs:103` `/// which is what the Python side's \`decode("latin-1")\` fallback does.`
- `src/safe.rs:182` `/// sent to radio servers.`
- `src/safe.rs:292` `/// loads (the Python game learned that the hard way).`
- `src/safe.rs:354` `/// not fatal, exactly as the Python \`_load_plugins\` logs and carries on.`
- `src/safe.rs:393` `/// has returned in [\`Drop\`]. The Python backend does the same by hanging the`
- `src/safe.rs:397` `/// the same as \`sound_lib.Channel.__del__\`. A one-shot created with`
- `src/safe.rs:399` `/// \`channel_is_active\` says it has stopped; the game's facade keeps a retain`
- `src/safe.rs:488` `/// so the game calls it on a worker thread (the radio connect thread), never`
- `src/safe.rs:619` `/// inside a mixtime sync on the same channel -- that is the sustain loop.`
- `src/safe.rs:651` `/// pair the Python \`_stream_length_s\` uses to anchor the ignition crossfade.`
- `src/safe.rs:669` `/// Set or clear \`BASS_SAMPLE_LOOP\` -- \`sound_lib\`'s \`set_looping\`.`
- `src/safe.rs:724` `/// engine bands and road noise scale with \`BASS_ATTRIB_FREQ\`.`
- `src/safe.rs:748` `/// walking around a casino floor never moves fast enough for that to be`
- `src/safe.rs:792` `/// outside volume of 1.0: every source in the casino is omnidirectional, and`
- `src/safe.rs:919` `/// \`BASS_ChannelRemoveSync\` has returned. The Python \`SustainLoop\` pins its`
- `src/safe.rs:923` `/// For the game's one use -- a \`BASS_SYNC_POS | BASS_SYNC_MIXTIME\` that`
- `tests/smoke.rs:83` `    // NET timeouts are what the game configures first thing.`
- `tests/smoke.rs:196` `    // The SustainLoop shape: a POS|MIXTIME sync at loop_end that seeks back`
- `vendor/licenses/README.md:4,12,18,22,24-25,29-30,40,54` (whole file is LB-specific: `LUCKY_BREAK_BASS_PATH` at :22, "The casino plays its own WAV" at :24, Freight Fate at :25/:40, SDL2/Prism mention at :70-72).

Design-level game specifics baked into safe wrappers (not strings):
- `safe::set_3d_position` (`safe.rs:752-761`) always passes `vel = NULL` (Doppler off; rationale is casino footsteps).
- `safe::channel_set_3d_position` (`safe.rs:781-785`) passes `orient = NULL, vel = NULL`.
- `safe::channel_set_3d_attributes` (`safe.rs:794-803`) hard-codes cone `360, 360, 1.0`.
- `fx::load_bass_fx` log text about occlusion (`fx.rs:106`).

## 3. FFI and safe-wrapper style

The crate does NOT use `extern "C" { ... }` blocks (nothing is linked). Each symbol is a `pub type Fn* = unsafe extern "system" fn(...)` alias in `src/lib.rs`, a `pub` field on `loader::Api`, and a `sym!("BASS_...")` line in `Api::resolve`. Field names are the C name minus `BASS_`, snake_case (`loader.rs:136-138`). `extern "system"` = stdcall on 32-bit Windows, C elsewhere (`lib.rs:17-18`).

Crate-level attributes (`lib.rs:20-24`):
```rust
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

use std::os::raw::{c_char, c_int, c_void};
```

### Representative declarations (LB `src/lib.rs:517-525`, `:591-597`)

```rust
/// `void CALLBACK SYNCPROC(HSYNC handle, DWORD channel, DWORD data, void *user)`.
///
/// Called by BASS on one of its own threads -- for `BASS_SYNC_MIXTIME` syncs
/// that is the mixer/update thread, while it holds the channel's lock. The
/// callback must be quick and must not block on anything the mixer thread
/// may itself be waiting for. `BASS_ChannelSetPosition` on the same channel
/// is explicitly allowed (it is what the sustain-loop sync does).
pub type SYNCPROC =
    unsafe extern "system" fn(handle: HSYNC, channel: DWORD, data: DWORD, user: *mut c_void);
```

```rust
pub type FnChannelSetSync = unsafe extern "system" fn(
    handle: DWORD,
    type_: DWORD,
    param: QWORD,
    proc_: Option<SYNCPROC>,
    user: *mut c_void,
) -> HSYNC;
```
(Nullable callbacks are `Option<CALLBACKTYPE>`; `type`/`proc` are spelled `type_`/`proc_`.)

### Loading (LB `src/loader.rs:198-221`, `:262-330` abridged only where marked)

```rust
/// Resolve one symbol, naming it on failure.
///
/// # Safety
/// The caller must give a `T` matching the symbol's real C signature.
unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    match library.get::<T>(name) {
        Ok(found) => Ok(*Symbol::into_raw(found)),
        Err(err) => {
            let text = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]).into_owned();
            log::warn!("bass: missing symbol {text}: {err}");
            Err(text)
        }
    }
}

impl Api {
    /// Load and cache the BASS API, or the reason it is unavailable.
    ///
    /// The first call does the work; later calls return the cached result,
    /// including a cached failure -- a missing library is not retried.
    pub fn get() -> Result<&'static Api, LoadError> {
        static API: OnceLock<Result<Api, LoadError>> = OnceLock::new();
        API.get_or_init(Api::load).as_ref().map_err(Clone::clone)
    }
```

```rust
    /// # Safety
    /// Every signature below must match bass.h exactly.
    unsafe fn resolve(library: Library, loaded_from: PathBuf) -> Result<Api, LoadError> {
        macro_rules! sym {
            ($name:literal) => {
                match symbol(&library, concat!($name, "\0").as_bytes()) {
                    Ok(f) => f,
                    Err(symbol) => {
                        return Err(LoadError::MissingSymbol {
                            path: loaded_from,
                            symbol,
                        })
                    }
                }
            };
        }
        Ok(Api {
            init: sym!("BASS_Init"),
            free: sym!("BASS_Free"),
            // ... one line per field ...
            channel_set_fx: sym!("BASS_ChannelSetFX"),
            channel_remove_fx: sym!("BASS_ChannelRemoveFX"),
            fx_set_parameters: sym!("BASS_FXSetParameters"),

            _library: library,
            loaded_from,
        })
    }
```

`Api` struct is at `loader.rs:139-190` (`_library: Library`, `loaded_from: PathBuf`, then `pub` fn-pointer fields grouped by blank lines), with `unsafe impl Send for Api {}` / `unsafe impl Sync for Api {}` at `:195-196`. Every symbol is mandatory: one missing export fails the whole load.

Adding a binding = 3 edits: `pub type FnX = unsafe extern "system" fn(...) -> R;` in `lib.rs` (near `:534-632`), `pub x: FnX,` in `Api` (`loader.rs:139-190`), `x: sym!("BASS_X"),` in `resolve` (`loader.rs:278-329`). Then a wrapper in `safe.rs`.

### Safe wrapper pattern (LB `src/safe.rs:84-96`, `:807-822`, `:551-557`)

Helpers:
```rust
/// The loaded API, or `NOT_LOADED`.
fn lib() -> Result<&'static Api, BassError> {
    api().map_err(|_| BassError::NOT_LOADED)
}

/// Turn a BASS `BOOL` into a `Result`, reading the error code on failure.
fn check(ok: BOOL) -> Result<(), BassError> {
    if ok != 0 {
        Ok(())
    } else {
        Err(BassError::last())
    }
}
```

BOOL-returning:
```rust
/// `BASS_ChannelSetAttribute(handle, attrib, value)` -- `BASS_ATTRIB_VOL`,
/// `PAN`, `FREQ`, ...
pub fn channel_set_attribute(handle: u32, attrib: u32, value: f32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain scalars.
    check(unsafe { (a.channel_set_attribute)(handle, attrib, value) })
}
```

Handle-returning (0 = failure):
```rust
/// `BASS_ChannelSetFX(handle, kind, priority)`.
///
/// `kind` is a `BASS_FX_DX8_*` type from BASS itself or a `BASS_FX_BFX_*` one
/// from `bass_fx` -- the latter only after [`crate::fx::load_bass_fx`] has
/// succeeded, otherwise BASS answers `BASS_ERROR_ILLTYPE`. Higher priority
/// effects run first. The handle is `0` on failure, which is an error here.
pub fn channel_set_fx(handle: u32, kind: u32, priority: i32) -> Result<HFX, BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    let fx = unsafe { (a.channel_set_fx)(handle, kind, priority) };
    if fx == 0 {
        Err(BassError::last())
    } else {
        Ok(fx)
    }
}
```

Conventions: free functions (not methods) taking raw `u32` handles; every `unsafe` block has a `// SAFETY:` comment; DWORD `-1` / QWORD `-1` error sentinels checked as `u32::MAX` / `u64::MAX`; void-returning calls (`apply_3d`) and "status" queries (`channel_is_active`, `channel_is_sliding`, `tags_*`) return plain values and degrade silently when not loaded. Callback lifetime pattern: `SyncGuard` (`safe.rs:890-1037`) double-boxes the closure (`Box<Box<dyn FnMut + Send>>`), passes `Box::into_raw` as `user`, `extern "system"` trampoline wraps the call in `catch_unwind`, Drop calls `BASS_ChannelRemoveSync` then frees the box. Memory-stream buffer lifetime: `Stream { handle, _buffer: Option<Arc<[u8]>> }` (`safe.rs:402-458`) with `unsafe impl Send/Sync`; Drop calls `BASS_StreamFree` before the buffer drops.

## 4. BASS_StreamCreate / STREAMPROC and constants

### Not bound anywhere

`grep -rn "StreamCreate\b|BASS_StreamCreate[^FU]|STREAMPROC|STREAMPROC_PUSH|BASS_STREAMPROC_END|StreamPutData"` over `lucky-break/crates` and `Freight-fate/crates` (`*.rs`, `*.py`): **0 hits**. The only stream constructors bound are `BASS_StreamCreateFile` (`FnStreamCreateFile`, LB `lib.rs:557-563`; used with `mem = TRUE` only) and `BASS_StreamCreateURL` (`FnStreamCreateURL`, `lib.rs:564-570`). No `BASS_StreamPutData`, no `BASS_ChannelGetData`, no `BASS_SampleLoad`.

### Where a real bass.h is

- No `bass*.h` in `C:\Users\joshu\gh-projects\lucky-break`, `C:\Users\joshu\Freight-fate`, or `hearken` (both repos' top-level `vendor/` hold only `sdl2/`; the crate vendor dirs hold only DLLs). LB `build.rs:105` skips `.h` files when staging, so a header could live in `vendor/<os>-<arch>/` without being copied.
- Found: `C:\Users\joshu\NVGT\nvgt\windev\include\bass.h` (46721 bytes; line 2 "BASS 2.4 C/C++ header file", line 3 "Copyright (c) 1999-2022 Un4seen Developments Ltd.") and `C:\Users\joshu\NVGT\nvgt\windev\include\bass_fx.h` (19891 bytes, BASS_FX 2.4).
- Secondary source: sound_lib's `pybass.py`, e.g. `C:\Users\joshu\gh-projects\fate-engine\.venv\Lib\site-packages\sound_lib\external\pybass.py`.

### Verbatim from NVGT bass.h

```c
// bass.h:10-20
#ifdef _WIN32
...
#include <wtypes.h>
typedef unsigned __int64 QWORD;
#else
#include <stdint.h>
#define WINAPI
#define CALLBACK
// bass.h:51
#define BASSDEF(f) WINAPI f
```
So `CALLBACK`/`WINAPI` are stdcall on Win32 and empty elsewhere: `extern "system"` for both the export and the callback type, matching the crate.

```c
// bass.h:484-498
typedef DWORD (CALLBACK STREAMPROC)(HSTREAM handle, void *buffer, DWORD length, void *user);
/* User stream callback function.
handle : The stream that needs writing
buffer : Buffer to write the samples in
length : Number of bytes to write
user   : The 'user' parameter value given when calling BASS_StreamCreate
RETURN : Number of bytes written. Set the BASS_STREAMPROC_END flag to end the stream. */

#define BASS_STREAMPROC_END		0x80000000	// end of user stream flag

// Special STREAMPROCs
#define STREAMPROC_DUMMY		(STREAMPROC*)0		// "dummy" stream
#define STREAMPROC_PUSH			(STREAMPROC*)-1		// push stream
#define STREAMPROC_DEVICE		(STREAMPROC*)-2		// device mix stream
#define STREAMPROC_DEVICE_3D	(STREAMPROC*)-3		// device 3D mix stream
```

```c
// bass.h:1041, 1047
HSTREAM BASSDEF(BASS_StreamCreate)(DWORD freq, DWORD chans, DWORD flags, STREAMPROC *proc, void *user);
DWORD BASSDEF(BASS_StreamPutData)(HSTREAM handle, const void *buffer, DWORD length);
```

Other relevant header lines: `bass.h:47` `#define BASSVERSION 0x204`; `:191` `#define BASS_DEVICE_3D 4 // unused`; `:318-323` sample flags; `:332` `BASS_STREAM_AUTOFREE 0x40000`; `:335` `BASS_STREAM_DECODE 0x200000`; `:103` `BASS_ERROR_ENDED 45`; `:607` `BASS_ATTRIB_PUSH_LIMIT 17`; `:1019-1023`, `:1085-1087` the 3D functions (signatures match the crate's `Fn*` types).

### Constant check (header vs brief vs crate)

| Constant | bass.h (NVGT) | Brief | LB crate |
|---|---|---|---|
| `BASS_STREAMPROC_END` | `0x80000000` (`:492`) | 0x80000000 | **not defined** |
| `STREAMPROC_PUSH` | `(STREAMPROC*)-1` (`:496`) | - | **not defined** |
| `STREAMPROC_DUMMY` / `_DEVICE` / `_DEVICE_3D` | `0` / `-2` / `-3` (`:495-498`) | - | **not defined** |
| `BASS_SAMPLE_FLOAT` | `256` (`:319`) | 256 | `lib.rs:225` `pub const BASS_SAMPLE_FLOAT: DWORD = 256;` |
| `BASS_SAMPLE_3D` | `8` (`:322`) | 8 | `lib.rs:223` |
| `BASS_SAMPLE_MONO` | `2` (`:320`) | 2 | `lib.rs:221` |
| `BASS_STREAM_AUTOFREE` | `0x40000` | - | `lib.rs:229` |
| `BASS_STREAM_DECODE` | `0x200000` | - | `lib.rs:233` |
| `BASS_ATTRIB_PUSH_LIMIT` | `17` | - | `lib.rs:260` |
| `BASS_ERROR_ENDED` | `45` | - | `lib.rs:102` |
| `BASS_CTYPE_STREAM` | - | - | `lib.rs:371` `0x10000` |
| `BASS_FX_BFX_BQF` | enum index 19 from `BASS_FX_BFX_ROTATE = 0x10000` (`bass_fx.h:68-87`) | - | `lib.rs:283` `0x10013` (verified: 20th entry) |

All brief values match the header. Types `STREAMPROC`, `FnStreamCreate`, `FnStreamPutData` do not exist in the crate. `HSTREAM` exists (`lib.rs:41`).

Direct transcription of the C signatures into the crate's existing style (for reference; names follow `lib.rs` conventions):
```rust
/// `DWORD CALLBACK STREAMPROC(HSTREAM handle, void *buffer, DWORD length, void *user)`.
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
/// `DWORD BASS_StreamPutData(HSTREAM handle, const void *buffer, DWORD length)`
pub type FnStreamPutData =
    unsafe extern "system" fn(handle: HSTREAM, buffer: *const c_void, length: DWORD) -> DWORD;
```
(See gotchas re: `STREAMPROC_PUSH` being `(STREAMPROC*)-1`, which `Option<STREAMPROC>` cannot express without a transmute.)

## 5. Existing tests

### Unit tests in `src/lib.rs` (`#[cfg(test)] mod tests`, `:650-758`) -- no DLL needed
- `channelinfo_layout_matches_bass_h` (`:656`): size 40/32, align = ptr, every field offset.
- `deviceinfo_layout_matches_bass_h` (`:676`): size 24/12, offsets.
- `error_names_cover_every_declared_code` (`:686`): `error_name` for a sample of codes; unknown -> `None`.
- `vector_and_bqf_layouts_match_the_headers` (`:705`): `BASS_3DVECTOR` 12 bytes/align 4/offsets; `BASS_BFX_BQF` 28 bytes/align 4/offsets.
- `three_d_and_bfx_constants_match_the_headers` (`:728`): `BASS_SAMPLE_MONO == 2`, `BASS_SAMPLE_3D == 8`, `BASS_DEVICE_3D == 4`, 3DMODE/3DALG values, `BASS_FX_BFX_BQF == 0x10013`, BQF enum ends, `BASS_BFX_CHANALL == -1`.
- `flag_values_match_pybass` (`:748`): AUTOFREE, LOOP, UNICODE, `SYNC_POS|MIXTIME`, NET_READTIMEOUT, TAG_META, ATTRIB_PAN.

### Unit tests in `src/loader.rs` (`:333-396`)
- `search_paths_end_with_bare_library_name` (`:338`).
- `search_paths_include_the_crate_vendor_dir` (`:345`).
- `both_override_env_vars_are_honoured_in_order` (`:360`): sets both env vars (LB/FF strings), asserts LB first, FF second. Uses `env_lock()` mutex (`:352-357`). Uses `std::env::set_var` without `unsafe` -> relies on edition < 2024.
- `an_empty_override_is_ignored` (`:379`).
- `loading_is_cached_and_never_panics` (`:388`): `Api::get()` twice agrees; `native_available()` agrees. Works with or without DLL.

### Unit tests in `src/fx.rs` (`:118-156`)
- `library_file_name_matches_the_platform` (`:123`).
- `search_paths_prefer_the_given_directory_and_end_with_the_bare_name` (`:135`).
- `loading_is_cached_and_never_panics` (`:145`): `load_bass_fx` on a nonexistent temp dir then `None`; results agree; `bass_fx_loaded()` agrees.

### Unit tests in `src/safe.rs` (`:1039-1122`)
- `error_names_and_display` (`:1044`): `"BASS_ERROR_HANDLE (5)"`, `NOT_LOADED` name, unknown `"BASS_ERROR_?"`.
- `plugin_file_names` (`:1055`): plugin name filter incl. `bass_fx` exclusion.
- `latin1_fallback_for_tags` (`:1076`).
- `calls_without_a_library_fail_softly` (`:1082`): body runs only `if !native_available()`; asserts `NOT_LOADED` from init/3D/fx calls, `channel_is_active == STOPPED`, `tags_meta == None`, `apply_3d()` no panic. Vacuous when DLL present.

### Integration tests `tests/smoke.rs` (446 lines)

Skip mechanism (`:16-32`, verbatim):
```rust
static BASS_LOCK: Mutex<()> = Mutex::new(());

/// Serialise the tests and make sure the no-sound device is up. `None` when
/// BASS is not loadable on this machine.
fn bass() -> Option<MutexGuard<'static, ()>> {
    if !native_available() {
        eprintln!("bass-sys: library not loadable here; skipping");
        return None;
    }
    let guard = BASS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match init(BASS_NO_SOUND_DEVICE, 44100, 0) {
        Ok(()) => {}
        Err(e) if e.is(BASS_ERROR_ALREADY) => {}
        Err(e) => panic!("BASS_Init(no sound) failed: {e}"),
    }
    Some(guard)
}
```
Each test starts `let Some(_g) = bass() else { return };` (passes vacuously). Helpers: `sine_wav(seconds) -> Vec<u8>` (`:35-59`, 16-bit mono 44.1k 440 Hz WAV built in memory), `wait_for(timeout, cond) -> bool` (`:62-71`, 10 ms poll). Imports `use bass_sys::safe::*; use bass_sys::*;` (crate name `bass_sys` hard-coded, also `bass_sys::fx::...` at `:407`, `:411`).

Tests:
- `init_no_sound_device_and_read_version` (`:74`): `version() >> 16 == BASSVERSION`, `"2.4."` prefix, current device 0, device 0 initialised, set/get `BASS_CONFIG_NET_TIMEOUT`.
- `memory_wav_stream_length_and_conversions` (`:90`): length bytes, bytes<->seconds, `channel_info` freq/chans/ctype WAV, device 0, position set/get, `free()` then `BASS_ERROR_HANDLE`.
- `garbage_is_refused_with_a_named_error` (`:127`): `BASS_ERROR_FILEFORM`.
- `loop_flag_round_trips_through_channel_flags` (`:135`).
- `attributes_set_get_and_slide` (`:155`): VOL/PAN/FREQ set/get; slide completes; pause/play/stop states.
- `mixtime_pos_sync_fires_and_can_seek_the_channel` (`:190`): POS|MIXTIME sync seeks back; fires twice; removal stops firings.
- `dropping_the_sync_guard_removes_it` (`:247`): dropped guard never fires; bad handle -> `BASS_ERROR_HANDLE`.
- `autofree_stream_goes_away_when_it_ends` (`:277`).
- `plugins_load_from_the_library_directory` (`:300`): loads all add-ons beside the DLL; bassopus optional (eprintln skip).
- `free_and_reinit_the_device` (`:330`): free -> `BASS_ERROR_INIT`; re-init; second init `BASS_ERROR_ALREADY`.
- `url_stream_with_no_network_target_fails_softly` (`:347`): `http://127.0.0.1:9/nothing` fails with 1.5 s timeout; tag readers `None` on bad handle.
- `three_d_channel_placement_round_trips` (`:362`): `BASS_SAMPLE_3D` mono stream (skips on `BASS_ERROR_NO3D`), 3D factors/listener/attributes/position, mode OFF/RELATIVE, dead handle -> `BASS_ERROR_HANDLE`.
- `bass_fx_biquad_lowpass_attaches_to_a_channel` (`:404`): skips if `load_bass_fx` false; `BASS_FX_BFX_BQF` accepted, `fx_set_bqf` accepted, double remove -> `BASS_ERROR_HANDLE`, unknown BFX type fails.

## 6. tools/fetch_bass.py (LB)

`C:\Users\joshu\gh-projects\lucky-break\tools\fetch_bass.py`, 294 lines, stdlib only (`argparse, hashlib, io, platform, shutil, sys, urllib.error, urllib.request, zipfile, pathlib`). Run as `uv run python tools/fetch_bass.py`.

What/where: downloads un4seen zip archives over HTTPS, extracts one member per file (case-insensitive match), checks sha256 against a pin, writes to `crates/bass-sys/vendor/<os>-<arch>/<name>`. Only the running host's platform is filled (`target_keys()`); macOS fills both `macos-x86_64` and `macos-aarch64` from one universal binary. Unsupported host -> `SystemExit` naming `LUCKY_BREAK_BASS_PATH`.

Game-specific paths/strings:
- `:49-50`
  ```python
  REPO_ROOT = Path(__file__).resolve().parents[1]
  VENDOR = REPO_ROOT / "crates" / "bass-sys" / "vendor"
  ```
- `:8-17` docstring names `crates/bass-sys`, `LUCKY_BREAK_BASS_PATH`, "The casino plays its own WAV assets", "Freight Fate fetched", `crates/bass-sys/src/fx.rs`.
- `:58-59` comment "Pins taken from un4seen on 2026-09-02, which is when the casino's Rust audio backend first existed".
- `:153` `"Install BASS yourself and point LUCKY_BREAK_BASS_PATH at it."`
- `:175-176` "matched the same way the Freight Fate script matched them."
- `:276-277` error text `"Run: uv run python tools/fetch_bass.py"`.
- `:282` checks `VENDOR / "licenses" / "README.md"` exists (warns only).

Pin tables (verbatim, `:60-125`):
```python
WINDOWS_X64 = {
    "bass.dll": (
        "https://www.un4seen.com/files/bass24.zip",
        "x64/bass.dll",
        "febb2cf1882d554c3a958280777da0b69f07de6e262df271de11c56e4a54afd4",
    ),
    "bass_fx.dll": (
        "https://www.un4seen.com/files/z/0/bass_fx24.zip",
        "x64/bass_fx.dll",
        "a6e1847eef52d882b4137af514d834c2e220daceb417c821d1e502fb7a34c84a",
    ),
}

MACOS_UNIVERSAL = {
    "libbass.dylib": (
        "https://www.un4seen.com/files/bass24-osx.zip",
        "libbass.dylib",
        "e81fb7b4d0009ba6343fbfcd840620704dcb840686d405b9734cd37150d31974",
    ),
    "libbass_fx.dylib": (
        "https://www.un4seen.com/files/z/0/bass_fx24-osx.zip",
        "libbass_fx.dylib",
        "82aa99a433d9e86de7ecbbd65599b2ffe9befebcd420d6368ef3bd1d33e418fe",
    ),
}

LINUX_X86_64 = {
    "libbass.so": (
        "https://www.un4seen.com/files/bass24-linux.zip",
        "libs/x86_64/libbass.so",
        "bd0841d0c14f25065a16192be07c20b045e384199a86a00892cc3864474dbc51",
    ),
    "libbass_fx.so": (
        "https://www.un4seen.com/files/z/0/bass_fx24-linux.zip",
        "libs/x86_64/libbass_fx.so",
        "63a41ba9627d4577874fd8f241789834ad058f4ef3bac930bf9bbce54b4cedba",
    ),
}

LINUX_AARCH64 = {
    "libbass.so": (
        "https://www.un4seen.com/files/bass24-linux.zip",
        "libs/aarch64/libbass.so",
        "a05ba3afc880b03f36db8dea50fd1d912cacf2a2c85d785fbaef020f5a93adb2",
    ),
    "libbass_fx.so": (
        "https://www.un4seen.com/files/z/0/bass_fx24-linux.zip",
        "libs/aarch64/libbass_fx.so",
        "7bc424672048e6cb8e44a3e0b5da984ad7996cf77fa4d3e47808bca3c7e793e8",
    ),
}

TARGETS = {
    "windows-x86_64": WINDOWS_X64,
    "macos-x86_64": MACOS_UNIVERSAL,
    "macos-aarch64": MACOS_UNIVERSAL,
    "linux-x86_64": LINUX_X86_64,
    "linux-aarch64": LINUX_AARCH64,
}
```
On-disk check (2026-09-26): LB `vendor/windows-x86_64/bass.dll` sha256 `febb2cf1...afd4` and `bass_fx.dll` `a6e1847e...c84a` match the pins; FF's `bass.dll` is byte-identical (FF pin at `Freight-fate/tools/fetch_bass.py` has the same `febb2cf1...` hash).

`target_keys()` (`:132-154`):
```python
    if sys.platform == "win32":
        return ["windows-x86_64"]
    if sys.platform == "darwin":
        return ["macos-x86_64", "macos-aarch64"]
    if sys.platform.startswith("linux"):
        machine = platform.machine()
        if machine in ("x86_64", "AMD64"):
            return ["linux-x86_64"]
        if machine in ("aarch64", "arm64"):
            return ["linux-aarch64"]
```

CLI (`:224-239`; docstring `:30-33`):
```
uv run python tools/fetch_bass.py                # fetch what is missing
uv run python tools/fetch_bass.py --check        # verify, change nothing
uv run python tools/fetch_bass.py --force        # re-fetch everything
uv run python tools/fetch_bass.py --print-hashes # what upstream serves now
```
`--check` exits 1 if any file is missing/stale. `--print-hashes` downloads every platform's archives, prints `"{sha}  {key}/{name}  <- {url}!{member}  (pinned|DIFFERS from the pin)"`, writes nothing. Main loop (`:251-272`): existing file with matching hash -> `ok`; otherwise `stale`/fetch; bytes reused across the two macOS dirs via `fetched[want]`; archives cached per URL within a run. Network failure message names `{VENDOR}/<os>-<arch>/` (`:163-167`). Hash mismatch -> `SystemExit` with expected/got (`:191-198`). No tests for this script in LB (FF has `tests/test_native_library_targets.py` testing its own copy's `target_keys()`/`TARGETS`).

CI consumers in LB: `.github/workflows/rust.yml:59-61` (`actions/cache@v4`, `path: crates/bass-sys/vendor`, `key: bass-${{ runner.os }}-${{ hashFiles('tools/fetch_bass.py') }}`), Windows step `uv run python tools/fetch_bass.py` (best-effort on Ubuntu); `.github/workflows/release.yml:45` same cache path.

FF's copy (`C:\Users\joshu\Freight-fate\tools\fetch_bass.py`, 309 lines) has the same shape and `VENDOR` (`:45`), but also fetches `bass_aac.dll`, `bassflac.dll`, `bassopus.dll` (from a `SOUND_LIB_X64` GitHub raw URL, samtupy/sound_lib_macos_fixes) and `basshls.dll` (`basshls24.zip`), plus osx/linux flac/opus/hls; no `bass_fx`. Names `FREIGHT_FATE_BASS_PATH` at `:10`, `:220`.

## 7. build.rs (Freight Fate and Lucky Break)

### LB `crates/bass-sys/build.rs` (115 lines, verbatim)

```rust
//! Stage the fetched BASS shared libraries next to the build output.
//!
//! BASS is un4seen's and proprietary, so it is NOT committed to this public
//! repository: `tools/fetch_bass.py` puts it under `vendor/<os>-<arch>/`,
//! pinned by sha256, and this script stages it from there.
//!
//! BASS is loaded at run time (see `src/loader.rs`), so nothing is linked
//! here. What this script does is copy everything the fetch put in the
//! platform's vendor directory -- `bass` itself and `bass_fx`, the effects
//! library the occlusion filter needs -- into the Cargo target directory, so
//! `cargo run` and `cargo test` find them without the developer having to
//! touch PATH. They land in one directory because that is where both loaders
//! look: `src/loader.rs` for BASS and `src/fx.rs` for `bass_fx`, which is
//! sought beside it.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Shared-library file names for the target platform: BASS itself first,
/// then `bass_fx`. A stale copy of any of them is cleared when the fetch has
/// not been run for this platform.
fn library_file_names(target_os: &str) -> [&'static str; 2] {
    match target_os {
        "windows" => ["bass.dll", "bass_fx.dll"],
        "macos" => ["libbass.dylib", "libbass_fx.dylib"],
        _ => ["libbass.so", "libbass_fx.so"],
    }
}

/// Vendor subdirectory holding the libraries for one target.
///
/// Keyed by architecture as well as OS: macOS ships separate Intel and Apple
/// silicon builds under the same file name, so a single directory cannot hold
/// both.
fn vendor_subdirectory(target_os: &str, target_arch: &str) -> String {
    format!("{target_os}-{target_arch}")
}

/// Walk up from `OUT_DIR` to the profile directory (`target/debug`), which is
/// where Cargo places binaries and where the loader looks first.
fn profile_dir(out_dir: &Path) -> Option<PathBuf> {
    // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out
    out_dir.ancestors().nth(3).map(Path::to_path_buf)
}

fn main() {
    println!("cargo:rerun-if-changed=vendor");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let file_names = library_file_names(&target_os);
    let file_name = file_names[0];

    let vendor_dir = manifest_dir
        .join("vendor")
        .join(vendor_subdirectory(&target_os, &target_arch));
    let vendored = vendor_dir.join(file_name);
    println!("cargo:vendor_dir={}", vendor_dir.display());

    if !vendored.is_file() {
        // A copy staged by an earlier build, from before this platform's
        // library was dropped, would still be found and loaded by the loader.
        if let Some(profile_dir) = profile_dir(&out_dir) {
            for name in file_names {
                let _ = fs::remove_file(profile_dir.join(name));
            }
        }
        // A developer who has not run the fetch gets a silent game and no
        // obvious reason why, which is the worst way to learn that BASS is
        // not in the repository. Name the command in the warning.
        println!(
            "cargo:warning=bass-sys: {file_name} is missing for \
             {target_os}-{target_arch}. Run `uv run python tools/fetch_bass.py` \
             -- BASS is fetched, not committed. Until then the game runs without \
             audio unless the library is installed system-wide."
        );
        return;
    }

    let Some(profile_dir) = profile_dir(&out_dir) else {
        return;
    };
    // Everything in the platform directory, not just the core library:
    // bass_fx has to sit beside bass.dll for `fx::load_bass_fx` to find it,
    // and on Linux it resolves libbass.so through the dynamic loader's search
    // path, which includes the directory it was loaded from.
    let Ok(entries) = fs::read_dir(&vendor_dir) else {
        println!(
            "cargo:warning=bass-sys: could not read {}",
            vendor_dir.display()
        );
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Import libraries, headers and licence notes are for the build and
        // the reader, not the running program.
        if name.ends_with(".lib")
            || name.ends_with(".txt")
            || name.ends_with(".md")
            || name.ends_with(".h")
        {
            continue;
        }
        // Copy failures are not fatal: the loader also searches the vendor
        // directory and the system library path.
        if let Err(err) = fs::copy(entry.path(), profile_dir.join(name)) {
            println!("cargo:warning=bass-sys: could not stage {name}: {err}");
        }
    }
}
```
FF `crates/bass-sys/build.rs` (108 lines) is the same minus `bass_fx`: single `library_file_name(target_os) -> &'static str`, removes one stale file, doc comment names the plugins (`bassopus`, `bassflac`, `bass_aac`, `basshls`).

### FF `crates/freight-fate/build.rs` (149 lines)

`main` (`:12-44`, verbatim):
```rust
fn main() {
    delay_load_prism_backends();
    link_prism_system_libraries();
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.ancestors().nth(2).unwrap().to_path_buf();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let dir = root
        .join("vendor")
        .join("sdl2")
        .join(format!("{os}-{arch}"));
    println!("cargo:rerun-if-changed={}", dir.display());
    if !dir.is_dir() {
        // macOS and Linux are not vendored on purpose: SDL2 is compiled in
        // statically there (see Cargo.toml), so there is nothing missing and
        // nothing to warn about.
        if os != "macos" && os != "linux" {
            println!("cargo:warning=freight-fate: no vendored SDL2 for {os}-{arch} under vendor/sdl2; expecting a system SDL2");
        }
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    if let Some(profile) = out.ancestors().nth(3) {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "dll" | "so" | "dylib") {
                let _ = fs::copy(&p, profile.join(p.file_name().unwrap()));
            }
        }
    }
}
```
Header doc `:1-9` and `use std::{env, fs, path::PathBuf};` at `:10`.

`delay_load_prism_backends` (`:46-63`, verbatim):
```rust
/// Delay-load the DLLs behind Prism's Windows backends (NVDA, JAWS, ...).
///
/// prismer links Prism statically and publishes the list, but a link flag
/// only takes effect on the final link, which happens here. Without it every
/// screen reader client DLL becomes a hard import and the game will not start
/// on a machine missing any of them.
fn delay_load_prism_backends() {
    println!("cargo:rerun-if-env-changed=DEP_PRISMER_DELAY_LOAD_DLLS");
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let Ok(dlls) = env::var("DEP_PRISMER_DELAY_LOAD_DLLS") else {
        return;
    };
    for dll in dlls.split(';').filter(|dll| !dll.is_empty()) {
        println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
    }
}
```

How the list arrives (a `DEP_` env var, two hops):
1. `prism-sys` 0.1.3 (crates.io; `Cargo.toml` `links = "prism"`) `build.rs` `link_windows_import_libs()` reads `OUT_DIR/lib/prism-static-windows.txt` (lines of `<importlib> <dll>`), emits `cargo:rustc-link-lib=dylib=<lib>` per line, then `println!("cargo:delay_load_dlls={}", dlls.join(";"));` (prism-sys `build.rs:64`).
2. `prismer` 0.1.3 (`links = "prismer"`, edition 2024) `build.rs` (verbatim, whole file):
   ```rust
   //! Passes prism-sys's delay-load list through to whoever depends on prismer.
   //!
   //! Cargo hands a `links` crate's metadata to its direct dependents only, so
   //! without this an application depending on prismer could not see what
   //! prism-sys published.

   fn main() {
   	println!("cargo:rerun-if-changed=build.rs");
   	if let Ok(dlls) = std::env::var("DEP_PRISM_DELAY_LOAD_DLLS") {
   		println!("cargo:delay_load_dlls={dlls}");
   	}
   }
   ```
3. The game crate (direct dependent of prismer: FF `crates/freight-fate/Cargo.toml` `prismer = { version = "0.1.3", features = ["static"] }`) sees `DEP_PRISMER_DELAY_LOAD_DLLS` (`;`-separated) and emits `/DELAYLOAD:` link args (MSVC only).
(Registry sources: `C:\Users\joshu\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\prismer-0.1.3\build.rs`, `...\prism-sys-0.1.3\build.rs`.)

Rest of FF build.rs:
- `link_prism_system_libraries()` (`:72-119`): on linux/macos `cargo:rustc-link-lib=static:+whole-archive,-bundle=prism`; macOS frameworks `Foundation, AVFoundation, AppKit, IOKit, CoreFoundation` + `objc`; Linux calls `link_libstdcxx_statically()` then `pkg-config --libs` for `speech-dispatcher`, `glibmm-2.68`, `giomm-2.68`, translating `-L`/`-l`.
- `link_libstdcxx_statically()` (`:128-149`): `$CXX -print-file-name=libstdc++.a` (default `c++`), copies the archive into `OUT_DIR/static-libstdcxx/` and adds it as `rustc-link-search=native`; warns if not absolute.

SDL2 vendoring (FF and LB identical layout): `<repo>/vendor/sdl2/` with `LICENSE.txt`, `README.md`, `windows-x86_64/SDL2.dll`, `windows-x86_64/SDL2.lib` -- all git-tracked in both repos. build.rs computes `<repo root> = CARGO_MANIFEST_DIR.ancestors().nth(2)`, adds `cargo:rustc-link-search=native=<root>/vendor/sdl2/<os>-<arch>` (import lib for the `sdl2` crate's dynamic link) and copies `*.dll|*.so|*.dylib` from there into `target/<profile>/` (`OUT_DIR.ancestors().nth(3)`). macOS/Linux have no vendor dir: `sdl2` crate uses `bundled` + `static-link` features (FF `crates/freight-fate/Cargo.toml` target sections).

### LB equivalents

- `crates/lucky-break/build.rs` (42 lines): exactly FF's `main` SDL2 block minus the two prism calls; warning text `lucky-break: no vendored SDL2 for {os}-{arch} under vendor/sdl2; expecting a system SDL2` (`:27`). No delay-load, no prism linking.
- LB does not use `prismer`. It has its own `crates/prism-sys` (build.rs 96 lines: runtime-loaded `prism.dll`/`libprism.*`, same staging pattern as bass-sys, `cargo:vendor_dir=` at `:50`, vendor dirs for windows/macos/linux x86_64+aarch64 under `crates/prism-sys/vendor/`) and `crates/prism`. `grep DELAYLOAD|delay_load|DEP_` over LB `crates/`: 0 hits.

## 8. rust-toolchain.toml, workspace lints, profiles

### `rust-toolchain.toml`

LB (`C:\Users\joshu\gh-projects\lucky-break\rust-toolchain.toml`, verbatim):
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

FF (`C:\Users\joshu\Freight-fate\rust-toolchain.toml`, verbatim):
```toml
[toolchain]
# Keep CI, snapshots and local builds on the same compiler. A moving stable
# channel invalidates compiler caches when the runner image updates.
# Bump deliberately after validating the new compiler on Career 1.9.
channel = "1.98.0"
# Do not inherit a user's complete profile: it requests components unavailable
# on stable Windows (including Miri and Cranelift) before Cargo can build.
profile = "minimal"
components = ["rustfmt", "clippy"]
```

### `[workspace.package]`

LB `Cargo.toml:11-17`:
```toml
[workspace.package]
version = "1.0.0"
edition = "2021"
rust-version = "1.95"
license = "PolyForm-Noncommercial-1.0.0"
repository = "https://github.com/Orinks/lucky-break"
authors = ["Joshua Tubbs (Orinks)"]
```
FF `Cargo.toml:9-15`: same but `version = "1.9.0"`, `repository = "https://github.com/Orinks/Freight-fate"`. Both `edition = "2021"`, `rust-version = "1.95"`. (bass-sys overrides `license = "MIT"`.) Both `[workspace] resolver = "2"`.

Relevant `[workspace.dependencies]`: both `libloading = "0.8"` (LB Cargo.lock: 0.8.9), `log = "0.4"`.

### Workspace lints

LB `Cargo.toml:42-43`:
```toml
[workspace.lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ["cfg(ci_quick)"] }
```
FF `Cargo.toml:64-69`:
```toml
[workspace.lints.rust]
# `--cfg ci_quick` is what the per-push CI passes so the whole-map sweeps
# (`#[cfg_attr(ci_quick, ignore = "sweep: ...")]`) are left to the nightly
# snapshot build and to full local runs; see .github/workflows/rust.yml.
# Declared here so `unexpected_cfgs` stays quiet under `-D warnings`.
unexpected_cfgs = { level = "warn", check-cfg = ["cfg(ci_quick)"] }
```
No `[workspace.lints.clippy]` in either. bass-sys has no `[lints] workspace = true` in either repo (opted in: FF `ff-core`, `freight-fate`; LB `lb-core`, `lucky-break`). CI clippy: LB `rust.yml:86` `cargo clippy --workspace --all-targets -- -D warnings` (covers bass-sys); FF `rust.yml:149` `cargo clippy --all-targets --locked -- -D warnings`. LB `rust.yml:27` / FF `:97` `cargo fmt --all --check`.

### Profiles

LB `Cargo.toml:45-58`:
```toml
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
FF `Cargo.toml:71-103`: identical values, with comments:
```toml
[profile.release]
opt-level = 3
lto = "thin"
strip = true

[profile.dev]
# opt-level 0 compiles noticeably faster and the game is not CPU-bound in a
# debug run; the one place that wanted optimisation was the world load, and
# the baked container solved that instead.
opt-level = 0

[profile.test]
# Debug info is the bulk of what the linker writes, and ~127 test binaries
# each carry a copy: turning it off was worth minutes per cold build and a
# few gigabytes on disk. A backtrace still names the failing test; only
# line numbers inside dependencies are lost.
debug = 0
# The dev profile reasons that the GAME is not CPU-bound in a debug run,
# which is true and does not carry over here: the suite drives whole
# simulated trips, and at opt-level 0 one playtest binary took 105 seconds
# and ff-core alone took 100. Tests are compiled once and run over and over,
# so the trade runs the other way from the game build. Level 1, not 2: at
# four threads the game crate's suite ran in 60 s at level 1 against 58 s
# at level 2 (2026-09-02), and the hosted Windows runner spends four
# minutes compiling the two big test binaries, which is where the LLVM
# time saved by the lower level goes.
opt-level = 1

[profile.dev.package."*"]
# Dependencies are compiled once and never edited, so optimising them costs
# nothing in iteration time and speeds every test that parses, hashes or
# decompresses.
opt-level = 2
```

## Gotchas

1. **LB is not a superset of FF.** LB dropped `BASS_MusicLoad`/`BASS_MusicFree`, `BASS_MUSIC_*` flags, `music_load_mem_shared`, `Stream.music`/`free_handle`, `tags_string`, `BASS_TAG_HLS_EXTINF`, and two MOD smoke tests. FF's game uses them (`audio/bass.rs:28,375`, `audio/bass_radio.rs:9,367`). A shared crate built from LB alone breaks FF.
2. **Every `Api` symbol is mandatory.** One missing export fails the whole load with `MissingSymbol`, and later candidates are not tried (`loader.rs:243-251`). Only put `bass.dll` exports in `Api`; `bass_fx` symbols belong in `fx.rs`.
3. **`STREAMPROC_PUSH` is `(STREAMPROC*)-1`.** `Option<STREAMPROC>` cannot hold it without a transmute, so push streams need a different parameter type (e.g. a raw pointer or `usize`) for `proc`.
4. **Edition 2024 breaks things.** Both repos are edition 2021. Under 2024: `std::env::set_var`/`remove_var` in `loader.rs:362-366,381-383` become `unsafe`, and `unsafe_op_in_unsafe_fn` warns in `symbol()` (`loader.rs:203`), `resolve()`, `c_string_at` (`safe.rs:117`) and `sync_trampoline` (`safe.rs:905`). That warning fails `clippy -D warnings`.
5. **Renaming the crate renames it for `use`.** Package `hearken-bass-sys` becomes `hearken_bass_sys`. `tests/smoke.rs:13-14,407,411` and every consumer `use bass_sys::...` break unless `[lib] name = "bass_sys"` is set or the imports change.
6. **Vendor paths are tied to the crate directory.** `fetch_bass.py:50` `VENDOR = REPO_ROOT / "crates" / "bass-sys" / "vendor"`; LB `.gitignore:10-12` (`crates/bass-sys/vendor/*/bass*.dll` etc.); FF `.gitignore:143-145`; LB CI `rust.yml:61` and `release.yml:45` cache path; `build.rs:76` warning text; `vendor/licenses/README.md`. All name `crates/bass-sys`.
7. **Git or registry dependency = no DLLs.** `build.rs` stages from `CARGO_MANIFEST_DIR/vendor/...`, and `loader.rs` `vendor_dir()` bakes the same path in at compile time. For a git dependency that is a directory under `~/.cargo/git/checkouts/`, where the fetched (gitignored) DLLs never exist. Staging goes to the consumer's `target/<profile>/` via `OUT_DIR.ancestors().nth(3)`, but there is nothing to stage, so the consumer needs its own fetch location or the env var.
8. **`cargo:vendor_dir=` does nothing.** The crate has no `links` key, so no `DEP_*_VENDOR_DIR` is produced, and nothing reads one.
9. **Env-var doc and code disagree.** The comment says the legacy variable is read "when LIBRARY_PATH_ENV is unset". The code pushes both when both are set (LB first). The test asserts the code's behaviour.
10. **`bass_fx` lookup is narrower than `bass` lookup.** No env var and no vendor dir: only the passed `dir`, the exe dir, its parent, and the bare name. The result is cached with the first call's `dir`.
11. **Game policy is baked into the 3D wrappers.** Listener and source velocity are always NULL (no Doppler), source orientation is NULL, and the cone is fixed at 360/360/1.0. The doc comments justify this with the casino.
12. **Plugin filters differ.** LB `is_plugin_file_name` excludes `bass_fx`; FF's does not. FF's vendor dir has no `bass_fx`, so LB's version is safe for both.
13. **Smoke tests share one global device.** They take `BASS_LOCK`, and `free_and_reinit_the_device` frees and re-inits it. Any new integration test must use the same `bass()` guard, or it races under parallel `cargo test`.
14. **The toolchains differ.** FF pins `1.98.0` with `profile = "minimal"`; LB tracks `stable`. Both declare `rust-version = "1.95"`.
