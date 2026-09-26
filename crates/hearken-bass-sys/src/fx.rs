//! Runtime loading of `bass_fx`, un4seen's effects library.
//!
//! `bass_fx` is a separate library, not a BASS plugin: `BASS_PluginLoad`
//! refuses it. It hooks itself into BASS the moment any of its functions is
//! called, so opening it and calling `BASS_FX_GetVersion` is the whole
//! initialisation -- from then on the `BASS_FX_BFX_*` effect types are
//! available to `BASS_ChannelSetFX`.
//!
//! Absent library: [`load_bass_fx`] returns `false` and effects that need it
//! are simply unavailable, exactly as a missing `bass.dll` falls back to
//! silence rather than refusing to start.
//!
//! Nothing else from `bass_fx` is bound. The effects themselves go through
//! BASS's own `BASS_ChannelSetFX` / `BASS_FXSetParameters`, which
//! [`crate::safe`] already wraps; this module exists only to get the library
//! into the process.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libloading::Library;

/// The loaded library, kept alive for the life of the process. `None` is a
/// cached "not found": like the BASS loader, a failure is not retried.
static FX: OnceLock<Option<Library>> = OnceLock::new();

/// `BASS_FX_GetVersion`, the one symbol this module calls.
type FnFxGetVersion = unsafe extern "system" fn() -> u32;

/// Shared-library file name for `bass_fx` on the current platform.
pub fn library_file_name() -> &'static str {
    if cfg!(windows) {
        "bass_fx.dll"
    } else if cfg!(target_os = "macos") {
        "libbass_fx.dylib"
    } else {
        "libbass_fx.so"
    }
}

/// Candidate locations, most specific first: the caller's directory (which is
/// where BASS itself was loaded from), the executable's own directory and its
/// parent (test binaries live in `target/<profile>/deps/`), then the bare
/// name so the platform loader can find a system-wide install.
fn search_paths(dir: Option<&Path>) -> Vec<PathBuf> {
    let file_name = library_file_name();
    let mut candidates = Vec::new();
    if let Some(dir) = dir {
        candidates.push(dir.join(file_name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(file_name));
            if let Some(grandparent) = parent.parent() {
                candidates.push(grandparent.join(file_name));
            }
        }
    }
    candidates.push(PathBuf::from(file_name));
    candidates
}

/// Load `bass_fx`, looking in `dir` first -- pass [`crate::Api::library_dir`]
/// so it is found beside `bass.dll`.
///
/// Cached: the first call does the work and every later one returns the same
/// answer, `dir` included, so a caller that retries with a different
/// directory gets the first result. Never panics and never stops the game:
/// `false` just means the BFX effects are unavailable.
pub fn load_bass_fx(dir: Option<&Path>) -> bool {
    FX.get_or_init(|| {
        for candidate in search_paths(dir) {
            // SAFETY: loading a shared library runs its initialisers.
            // bass_fx's do nothing but register with BASS, and the candidate
            // paths are app-controlled.
            let Ok(library) = (unsafe { Library::new(&candidate) }) else {
                log::debug!("bass_fx: {} not loadable", candidate.display());
                continue;
            };
            // SAFETY: BASS_FX_GetVersion takes no arguments and returns a
            // DWORD, per bass_fx.h. Calling it is also what hooks bass_fx
            // into BASS, which is the point of loading it at all.
            let version = unsafe {
                library
                    .get::<FnFxGetVersion>(b"BASS_FX_GetVersion\0")
                    .map(|f| f())
            };
            match version {
                Ok(v) => {
                    log::info!(
                        "bass_fx {}.{}.{}.{} loaded from {}",
                        (v >> 24) & 0xff,
                        (v >> 16) & 0xff,
                        (v >> 8) & 0xff,
                        v & 0xff,
                        candidate.display()
                    );
                    return Some(library);
                }
                Err(err) => log::warn!(
                    "bass_fx: {} has no BASS_FX_GetVersion: {err}",
                    candidate.display()
                ),
            }
        }
        log::warn!("bass_fx not found; effects that need it are unavailable");
        None
    })
    .is_some()
}

/// Has `bass_fx` been loaded? `false` before the first [`load_bass_fx`] as
/// well as after a failed one -- this asks nothing of the filesystem.
pub fn bass_fx_loaded() -> bool {
    FX.get().is_some_and(Option::is_some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_file_name_matches_the_platform() {
        let name = library_file_name();
        if cfg!(windows) {
            assert_eq!(name, "bass_fx.dll");
        } else if cfg!(target_os = "macos") {
            assert_eq!(name, "libbass_fx.dylib");
        } else {
            assert_eq!(name, "libbass_fx.so");
        }
    }

    #[test]
    fn search_paths_prefer_the_given_directory_and_end_with_the_bare_name() {
        let dir = Path::new("some/where");
        let paths = search_paths(Some(dir));
        assert_eq!(paths.first().unwrap(), &dir.join(library_file_name()));
        assert_eq!(paths.last().unwrap(), &PathBuf::from(library_file_name()));
        // Without a directory the caller's own is not among the candidates.
        assert!(!search_paths(None).contains(&dir.join(library_file_name())));
    }

    #[test]
    fn loading_is_cached_and_never_panics() {
        // A directory holding no bass_fx must not panic, and whatever the
        // machine has, two calls agree -- the result is cached, a failure
        // included.
        let empty = std::env::temp_dir().join("bass-sys-no-such-fx-dir");
        assert!(!empty.join(library_file_name()).exists());
        let first = load_bass_fx(Some(&empty));
        let second = load_bass_fx(None);
        assert_eq!(first, second);
        assert_eq!(bass_fx_loaded(), first);
    }
}
