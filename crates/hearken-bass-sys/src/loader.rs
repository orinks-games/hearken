//! Runtime loading of the BASS shared library.
//!
//! Audio is an enhancement, not a hard dependency: a missing library must not
//! stop a game from starting. So rather than linking against an import
//! library, the symbols are resolved once on first use and cached. If any part
//! of that fails, [`Api::get`] returns the [`LoadError`] and the caller falls
//! back to running without sound.
//!
//! Discovery order, most specific first:
//!
//! 1. [`LIBRARY_PATH_ENV`] -- a file, or a directory containing the
//!    library. Set by developers and by playtest tooling.
//! 2. the executable's directory (a frozen release, or `target/<profile>/`
//!    where `build.rs` staged the vendored copy), and its parent for test
//!    binaries living in `target/<profile>/deps/`
//! 3. the crate's own `vendor/<os>-<arch>/` directory (a dev checkout where
//!    nothing has been staged yet)
//! 4. the bare file name, so the platform loader gets a chance to find a
//!    system-wide install.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libloading::{Library, Symbol};

use crate::*;

/// Environment variable naming the BASS library to load: a file, or a
/// directory containing it. A game sets it (for example in
/// `.cargo/config.toml` `[env]`) to wherever it fetched BASS.
pub const LIBRARY_PATH_ENV: &str = "HEARKEN_BASS_PATH";

/// Shared-library file name for the current platform.
pub fn library_file_name() -> &'static str {
    if cfg!(windows) {
        "bass.dll"
    } else if cfg!(target_os = "macos") {
        "libbass.dylib"
    } else {
        "libbass.so"
    }
}

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

    for var in [LIBRARY_PATH_ENV] {
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

/// Why the library could not be loaded. Cloneable so the cached failure can be
/// handed to every caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// No candidate path could be opened as a shared library.
    NotFound { tried: Vec<PathBuf> },
    /// A library opened but lacked one of the required exports -- the wrong
    /// BASS build, or something else called `bass.dll`.
    MissingSymbol { path: PathBuf, symbol: String },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotFound { tried } => {
                write!(f, "BASS library {} not found; tried", library_file_name())?;
                for path in tried {
                    write!(f, " {}", path.display())?;
                }
                Ok(())
            }
            LoadError::MissingSymbol { path, symbol } => write!(
                f,
                "{} loaded but is missing the BASS export {symbol}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LoadError {}

/// Resolved BASS entry points.
///
/// Only the symbols a game built on this crate actually calls are resolved.
/// Adding a new call site means adding a field here and a matching `load`
/// line. Field names are the C names without the `BASS_` prefix, in snake
/// case.
pub struct Api {
    _library: Library,
    loaded_from: PathBuf,

    pub init: FnInit,
    pub free: FnFree,
    pub set_config: FnSetConfig,
    pub set_config_ptr: FnSetConfigPtr,
    pub get_config: FnGetConfig,
    pub error_get_code: FnErrorGetCode,
    pub get_version: FnGetVersion,
    pub get_device: FnGetDevice,
    pub get_device_info: FnGetDeviceInfo,
    pub update: FnUpdate,

    pub plugin_load: FnPluginLoad,
    pub plugin_free: FnPluginFree,

    pub stream_create_file: FnStreamCreateFile,
    pub stream_create_url: FnStreamCreateURL,
    pub stream_create: FnStreamCreate,
    pub stream_free: FnStreamFree,
    pub music_load: FnMusicLoad,
    pub music_free: FnMusicFree,

    pub channel_play: FnChannelPlay,
    pub channel_pause: FnChannelPause,
    pub channel_stop: FnChannelStop,
    pub channel_is_active: FnChannelIsActive,
    pub channel_update: FnChannelUpdate,
    pub channel_get_data: FnChannelGetData,
    pub channel_set_attribute: FnChannelSetAttribute,
    pub channel_get_attribute: FnChannelGetAttribute,
    pub channel_slide_attribute: FnChannelSlideAttribute,
    pub channel_is_sliding: FnChannelIsSliding,
    pub channel_get_length: FnChannelGetLength,
    pub channel_get_position: FnChannelGetPosition,
    pub channel_set_position: FnChannelSetPosition,
    pub channel_bytes2seconds: FnChannelBytes2Seconds,
    pub channel_seconds2bytes: FnChannelSeconds2Bytes,
    pub channel_set_sync: FnChannelSetSync,
    pub channel_remove_sync: FnChannelRemoveSync,
    pub channel_get_tags: FnChannelGetTags,
    pub channel_flags: FnChannelFlags,
    pub channel_get_info: FnChannelGetInfo,
    pub channel_get_device: FnChannelGetDevice,

    pub set_3d_factors: FnSet3DFactors,
    pub set_3d_position: FnSet3DPosition,
    pub apply_3d: FnApply3D,
    pub channel_set_3d_attributes: FnChannelSet3DAttributes,
    pub channel_set_3d_position: FnChannelSet3DPosition,
    pub channel_set_fx: FnChannelSetFX,
    pub channel_remove_fx: FnChannelRemoveFX,
    pub fx_set_parameters: FnFXSetParameters,
}

// The library and its function pointers stay alive for the process lifetime
// and BASS is documented thread-safe (every function may be called from any
// thread), so the resolved table is shareable.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

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

    /// The path the library was opened from. Plugins are looked for in the
    /// same directory, which is where `build.rs` and the release layout put
    /// them.
    pub fn library_path(&self) -> &Path {
        &self.loaded_from
    }

    /// The directory holding the library, if the path has one.
    pub fn library_dir(&self) -> Option<&Path> {
        self.loaded_from
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
    }

    fn load() -> Result<Api, LoadError> {
        let tried = search_paths();
        for candidate in &tried {
            // SAFETY: loading a shared library runs its initialisers. BASS's
            // are benign, and the candidate paths are app-controlled.
            match unsafe { Library::new(candidate) } {
                Ok(library) => match unsafe { Api::resolve(library, candidate.clone()) } {
                    Ok(api) => {
                        log::info!("bass: loaded from {}", candidate.display());
                        return Ok(api);
                    }
                    Err(err) => {
                        log::warn!("bass: {err}");
                        return Err(err);
                    }
                },
                Err(err) => {
                    log::debug!("bass: {} not loadable: {err}", candidate.display());
                }
            }
        }
        log::info!("bass: library not found; audio is unavailable");
        Err(LoadError::NotFound { tried })
    }

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
            set_config: sym!("BASS_SetConfig"),
            set_config_ptr: sym!("BASS_SetConfigPtr"),
            get_config: sym!("BASS_GetConfig"),
            error_get_code: sym!("BASS_ErrorGetCode"),
            get_version: sym!("BASS_GetVersion"),
            get_device: sym!("BASS_GetDevice"),
            get_device_info: sym!("BASS_GetDeviceInfo"),
            update: sym!("BASS_Update"),

            plugin_load: sym!("BASS_PluginLoad"),
            plugin_free: sym!("BASS_PluginFree"),

            stream_create_file: sym!("BASS_StreamCreateFile"),
            stream_create_url: sym!("BASS_StreamCreateURL"),
            stream_create: sym!("BASS_StreamCreate"),
            stream_free: sym!("BASS_StreamFree"),
            music_load: sym!("BASS_MusicLoad"),
            music_free: sym!("BASS_MusicFree"),

            channel_play: sym!("BASS_ChannelPlay"),
            channel_pause: sym!("BASS_ChannelPause"),
            channel_stop: sym!("BASS_ChannelStop"),
            channel_is_active: sym!("BASS_ChannelIsActive"),
            channel_update: sym!("BASS_ChannelUpdate"),
            channel_get_data: sym!("BASS_ChannelGetData"),
            channel_set_attribute: sym!("BASS_ChannelSetAttribute"),
            channel_get_attribute: sym!("BASS_ChannelGetAttribute"),
            channel_slide_attribute: sym!("BASS_ChannelSlideAttribute"),
            channel_is_sliding: sym!("BASS_ChannelIsSliding"),
            channel_get_length: sym!("BASS_ChannelGetLength"),
            channel_get_position: sym!("BASS_ChannelGetPosition"),
            channel_set_position: sym!("BASS_ChannelSetPosition"),
            channel_bytes2seconds: sym!("BASS_ChannelBytes2Seconds"),
            channel_seconds2bytes: sym!("BASS_ChannelSeconds2Bytes"),
            channel_set_sync: sym!("BASS_ChannelSetSync"),
            channel_remove_sync: sym!("BASS_ChannelRemoveSync"),
            channel_get_tags: sym!("BASS_ChannelGetTags"),
            channel_flags: sym!("BASS_ChannelFlags"),
            channel_get_info: sym!("BASS_ChannelGetInfo"),
            channel_get_device: sym!("BASS_ChannelGetDevice"),

            set_3d_factors: sym!("BASS_Set3DFactors"),
            set_3d_position: sym!("BASS_Set3DPosition"),
            apply_3d: sym!("BASS_Apply3D"),
            channel_set_3d_attributes: sym!("BASS_ChannelSet3DAttributes"),
            channel_set_3d_position: sym!("BASS_ChannelSet3DPosition"),
            channel_set_fx: sym!("BASS_ChannelSetFX"),
            channel_remove_fx: sym!("BASS_ChannelRemoveFX"),
            fx_set_parameters: sym!("BASS_FXSetParameters"),

            _library: library,
            loaded_from,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_paths_end_with_bare_library_name() {
        let paths = search_paths();
        let last = paths.last().expect("at least one candidate");
        assert_eq!(last, &PathBuf::from(library_file_name()));
    }

    #[test]
    fn search_paths_include_the_crate_vendor_dir() {
        let paths = search_paths();
        assert!(paths.contains(&vendor_dir().join(library_file_name())));
    }

    /// The env-var tests set process-wide state, so they take one lock and
    /// cannot see each other's values.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

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

    #[test]
    fn an_empty_override_is_ignored() {
        let _guard = env_lock();
        std::env::set_var(LIBRARY_PATH_ENV, "");
        let paths = search_paths();
        std::env::remove_var(LIBRARY_PATH_ENV);
        assert!(!paths.iter().any(|p| p.as_os_str().is_empty()));
    }

    #[test]
    fn loading_is_cached_and_never_panics() {
        // Whether or not BASS is present, two calls must agree and neither
        // may panic.
        let first = Api::get().is_ok();
        let second = Api::get().is_ok();
        assert_eq!(first, second);
        assert_eq!(native_available(), first);
    }
}
