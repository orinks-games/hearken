//! Thin safe-ish wrappers over the raw BASS entry points.
//!
//! This is the layer a game's audio backend calls. It turns BASS's
//! `BOOL` + `BASS_ErrorGetCode` convention into `Result<_, BassError>`, owns
//! the two things that have lifetimes BASS cannot see -- the memory buffer a
//! memory stream decodes from ([`Stream`]) and the closure behind a sync
//! callback ([`SyncGuard`]) -- and nothing more. Channel handles are still plain
//! `u32`s: BASS validates them on every call and answers `BASS_ERROR_HANDLE`
//! for a stale one, so a caller can treat "already freed" as just another
//! error code, and there is nothing for a newtype to add.
//!
//! Every function here works on the "no sound" device
//! (`init(BASS_NO_SOUND_DEVICE, ..)`): channels play, decode and fire syncs
//! in real time, they just produce no output. Headless runs and the tests use
//! it.
//!
//! When the library is not loadable at all, every call fails with
//! [`BassError::NOT_LOADED`] rather than panicking, so a caller that forgot
//! to check [`crate::native_available`] still degrades to silence.

use std::ffi::{CStr, CString};
use std::fmt;
use std::os::raw::{c_char, c_void};
use std::path::Path;
use std::ptr;
use std::sync::Arc;

use crate::*;

mod effects;
mod stream;
mod sync;
mod tags;
mod three_d;

pub use effects::*;
pub use stream::*;
pub use sync::*;
pub use tags::*;
pub use three_d::*;

/// Our own code for "the BASS library is not loaded", outside BASS's range.
pub const ERROR_NOT_LOADED: i32 = -1000;

/// A failed BASS call: the code `BASS_ErrorGetCode` reported right after it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BassError {
    pub code: i32,
}

impl BassError {
    /// The library could not be loaded, so no call was made.
    pub const NOT_LOADED: BassError = BassError {
        code: ERROR_NOT_LOADED,
    };

    /// The error code BASS is currently reporting for this thread.
    pub fn last() -> BassError {
        match api() {
            // SAFETY: no arguments, no preconditions.
            Ok(a) => BassError {
                code: unsafe { (a.error_get_code)() },
            },
            Err(_) => BassError::NOT_LOADED,
        }
    }

    /// `BASS_ERROR_*` name for a known code; a placeholder otherwise.
    pub fn name(&self) -> &'static str {
        if self.code == ERROR_NOT_LOADED {
            return "BASS_NOT_LOADED";
        }
        error_name(self.code).unwrap_or("BASS_ERROR_?")
    }

    /// Is this the given `BASS_ERROR_*` code?
    pub fn is(&self, code: i32) -> bool {
        self.code == code
    }
}

impl fmt::Debug for BassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BassError({} = {})", self.name(), self.code)
    }
}

impl fmt::Display for BassError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name(), self.code)
    }
}

impl std::error::Error for BassError {}

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

/// Decode a BASS C string: UTF-8 when it is, latin-1 otherwise.
///
/// ICY metadata and device names come back as whatever bytes the server or
/// the OS produced -- usually UTF-8 these days, sometimes a legacy code page.
/// Latin-1 is the fallback because it never fails and keeps ASCII intact.
fn decode_c_string(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// # Safety
/// `ptr` must be null or a valid NUL-terminated C string.
unsafe fn c_string_at(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(decode_c_string(CStr::from_ptr(ptr).to_bytes()))
}

// --- Library lifecycle --------------------------------------------------------------

/// `BASS_Init(device, freq, flags, NULL, NULL)`.
///
/// `device` is `BASS_NO_SOUND_DEVICE` (0) for silent decoding,
/// `BASS_DEFAULT_DEVICE` (-1) for the system default, or a 1-based index from
/// [`device_info`]. A second `init` of an already-initialised device fails
/// with `BASS_ERROR_ALREADY`; callers that want "make sure it is up" treat
/// that as success.
pub fn init(device: i32, freq: u32, flags: u32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: window and clsid are optional and documented as NULL-able.
    check(unsafe { (a.init)(device, freq, flags, ptr::null_mut(), ptr::null()) })
}

/// `BASS_Free`: frees the current device and every channel on it.
pub fn free() -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: no arguments.
    check(unsafe { (a.free)() })
}

/// `BASS_GetVersion`, e.g. `0x02041100` for 2.4.17.
pub fn version() -> Result<u32, BassError> {
    let a = lib()?;
    // SAFETY: no arguments.
    Ok(unsafe { (a.get_version)() })
}

/// The version as dotted text, `"2.4.17.0"` style (major.minor.revision.build
/// from the four bytes of [`version`]).
pub fn version_string() -> Result<String, BassError> {
    let v = version()?;
    Ok(format!(
        "{}.{}.{}.{}",
        (v >> 24) & 0xff,
        (v >> 16) & 0xff,
        (v >> 8) & 0xff,
        v & 0xff
    ))
}

/// `BASS_SetConfig(option, value)`.
pub fn set_config(option: u32, value: u32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    check(unsafe { (a.set_config)(option, value) })
}

/// `BASS_GetConfig(option)`. BASS returns `(DWORD)-1` on error.
pub fn get_config(option: u32) -> Result<u32, BassError> {
    let a = lib()?;
    // SAFETY: plain integer.
    let v = unsafe { (a.get_config)(option) };
    if v == u32::MAX {
        Err(BassError::last())
    } else {
        Ok(v)
    }
}

/// `BASS_SetConfigPtr(BASS_CONFIG_NET_AGENT, agent)`: the HTTP user agent
/// sent when opening a URL stream.
///
/// BASS keeps the pointer rather than copying the string for this option, so
/// the C string is leaked on purpose -- the agent is set once per process.
pub fn set_net_agent(agent: &str) -> Result<(), BassError> {
    let a = lib()?;
    let c = CString::new(agent).map_err(|_| BassError {
        code: BASS_ERROR_ILLPARAM,
    })?;
    let leaked: &'static CStr = Box::leak(c.into_boxed_c_str());
    // SAFETY: the string lives for the rest of the process.
    check(unsafe { (a.set_config_ptr)(BASS_CONFIG_NET_AGENT, leaked.as_ptr().cast()) })
}

/// `BASS_Update(length_ms)`: render `length_ms` of every playing channel now,
/// on the calling thread. Mixtime syncs due in that span fire before it
/// returns. Only needed when the update thread is disabled
/// (`BASS_CONFIG_UPDATETHREADS` = 0) or a test wants determinism.
pub fn update(length_ms: u32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integer.
    check(unsafe { (a.update)(length_ms) })
}

// --- Devices ------------------------------------------------------------------------

/// One entry from `BASS_GetDeviceInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub driver: Option<String>,
    /// `BASS_DEVICE_ENABLED` / `DEFAULT` / `INIT` / `LOOPBACK` bits plus the
    /// type in the top byte.
    pub flags: u32,
}

impl DeviceInfo {
    pub fn enabled(&self) -> bool {
        self.flags & BASS_DEVICE_ENABLED != 0
    }
    pub fn is_default(&self) -> bool {
        self.flags & BASS_DEVICE_DEFAULT != 0
    }
    pub fn initialised(&self) -> bool {
        self.flags & BASS_DEVICE_INIT != 0
    }
}

/// `BASS_GetDeviceInfo(device)`. Device 0 is "No sound"; real outputs start
/// at 1. `BASS_ERROR_DEVICE` past the end, which is how [`devices`] stops.
pub fn device_info(device: u32) -> Result<DeviceInfo, BassError> {
    let a = lib()?;
    let mut raw = BASS_DEVICEINFO {
        name: ptr::null(),
        driver: ptr::null(),
        flags: 0,
    };
    // SAFETY: `raw` is a correctly laid-out, writable BASS_DEVICEINFO.
    check(unsafe { (a.get_device_info)(device, &mut raw) })?;
    // SAFETY: BASS filled the pointers with NUL-terminated strings (or NULL)
    // that stay valid until the next BASS_GetDeviceInfo call, which cannot
    // happen before we copy them out here.
    let (name, driver) = unsafe { (c_string_at(raw.name), c_string_at(raw.driver)) };
    Ok(DeviceInfo {
        name: name.unwrap_or_default(),
        driver,
        flags: raw.flags,
    })
}

/// Every output device BASS knows, as `(index, info)`, starting at device 1.
/// Index `i` is what `init(i, ..)` takes and what [`current_device`] returns.
pub fn devices() -> Vec<(u32, DeviceInfo)> {
    let mut out = Vec::new();
    let mut i = 1;
    while let Ok(info) = device_info(i) {
        out.push((i, info));
        i += 1;
    }
    out
}

/// `BASS_GetDevice`: the device the calling thread's channels go to.
pub fn current_device() -> Result<u32, BassError> {
    let a = lib()?;
    // SAFETY: no arguments.
    let d = unsafe { (a.get_device)() };
    if d == u32::MAX {
        Err(BassError::last())
    } else {
        Ok(d)
    }
}

/// `BASS_ChannelGetDevice(handle)`.
pub fn channel_device(handle: u32) -> Result<u32, BassError> {
    let a = lib()?;
    // SAFETY: plain integer; BASS validates the handle.
    let d = unsafe { (a.channel_get_device)(handle) };
    if d == u32::MAX {
        Err(BassError::last())
    } else {
        Ok(d)
    }
}

// --- Plugins --------------------------------------------------------------------------

/// `BASS_PluginLoad(path)`, with the path handed over as UTF-16 +
/// `BASS_UNICODE` on Windows so an install under a non-ANSI user name still
/// loads.
pub fn plugin_load(path: &Path) -> Result<HPLUGIN, BassError> {
    let a = lib()?;
    #[cfg(windows)]
    let handle = {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        // SAFETY: `wide` is NUL-terminated UTF-16 and outlives the call.
        unsafe { (a.plugin_load)(wide.as_ptr().cast(), BASS_UNICODE) }
    };
    #[cfg(not(windows))]
    let handle = {
        let text = path.to_str().ok_or(BassError {
            code: BASS_ERROR_ILLPARAM,
        })?;
        let c = CString::new(text).map_err(|_| BassError {
            code: BASS_ERROR_ILLPARAM,
        })?;
        // SAFETY: `c` is NUL-terminated and outlives the call.
        unsafe { (a.plugin_load)(c.as_ptr().cast(), 0) }
    };
    if handle == 0 {
        Err(BassError::last())
    } else {
        Ok(handle)
    }
}

/// `BASS_PluginFree(handle)`; `0` frees every plugin.
pub fn plugin_free(handle: HPLUGIN) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integer.
    check(unsafe { (a.plugin_free)(handle) })
}

/// Is this file name a BASS add-on (not the core library itself)?
///
/// `bass_fx` is deliberately excluded. It sits beside `bass.dll` and matches
/// the `bass*` shape, but it is a separate un4seen library rather than a
/// plugin: `BASS_PluginLoad` refuses it, and it is opened by
/// [`crate::fx::load_bass_fx`] instead.
fn is_plugin_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if cfg!(windows) {
        lower.starts_with("bass")
            && lower.ends_with(".dll")
            && lower != "bass.dll"
            && lower != "bass_fx.dll"
    } else {
        lower.starts_with("libbass")
            && (lower.ends_with(".so") || lower.ends_with(".dylib"))
            && lower != "libbass.so"
            && lower != "libbass.dylib"
            && lower != "libbass_fx.so"
            && lower != "libbass_fx.dylib"
    }
}

/// Load every BASS add-on in `dir` -- `bass*.dll` on Windows, `libbass*.so` /
/// `libbass*.dylib` elsewhere -- except the core library. One entry per file
/// in name order, with the handle or the error; a refused plugin is reported,
/// not fatal, so the caller can log it and carry on.
///
/// An unreadable or missing directory yields an empty list.
pub fn load_plugins_from(dir: &Path) -> Vec<(String, Result<HPLUGIN, BassError>)> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().is_file())
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| is_plugin_file_name(n))
            .collect(),
        Err(err) => {
            log::debug!("bass: no plugin directory {}: {err}", dir.display());
            return Vec::new();
        }
    };
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let result = plugin_load(&dir.join(&name));
            match &result {
                Ok(h) => log::info!("bass: loaded plugin {name} (handle {h})"),
                Err(e) => log::warn!("bass: could not load plugin {name}: {e}"),
            }
            (name, result)
        })
        .collect()
}

// --- Channel control --------------------------------------------------------------

/// `BASS_ChannelPlay(handle, restart)`.
pub fn channel_play(handle: u32, restart: bool) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    check(unsafe { (a.channel_play)(handle, restart as BOOL) })
}

/// `BASS_ChannelPause(handle)`.
pub fn channel_pause(handle: u32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integer.
    check(unsafe { (a.channel_pause)(handle) })
}

/// `BASS_ChannelStop(handle)`. An autofree stream is freed by this.
pub fn channel_stop(handle: u32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integer.
    check(unsafe { (a.channel_stop)(handle) })
}

/// `BASS_ChannelIsActive(handle)`: one of the `BASS_ACTIVE_*` values.
/// `BASS_ACTIVE_STOPPED` for an invalid handle or an unloaded library, which
/// is what a caller polling "is it still going" wants.
pub fn channel_is_active(handle: u32) -> u32 {
    match lib() {
        // SAFETY: plain integer.
        Ok(a) => unsafe { (a.channel_is_active)(handle) },
        Err(_) => BASS_ACTIVE_STOPPED,
    }
}

/// `BASS_ChannelUpdate(handle, length_ms)`: render up to `length_ms` of this
/// channel into its playback buffer now, on the calling thread.
pub fn channel_update(handle: u32, length_ms: u32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    check(unsafe { (a.channel_update)(handle, length_ms) })
}

/// `BASS_ChannelSetAttribute(handle, attrib, value)` -- `BASS_ATTRIB_VOL`,
/// `PAN`, `FREQ`, ...
pub fn channel_set_attribute(handle: u32, attrib: u32, value: f32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain scalars.
    check(unsafe { (a.channel_set_attribute)(handle, attrib, value) })
}

/// `BASS_ChannelGetAttribute(handle, attrib)`.
pub fn channel_get_attribute(handle: u32, attrib: u32) -> Result<f32, BassError> {
    let a = lib()?;
    let mut value = 0f32;
    // SAFETY: `value` is a writable f32 that outlives the call.
    check(unsafe { (a.channel_get_attribute)(handle, attrib, &mut value) })?;
    Ok(value)
}

/// `BASS_ChannelSlideAttribute(handle, attrib, value, time_ms)`.
///
/// Sliding `BASS_ATTRIB_VOL` to `-1.0` is the fade-out idiom: BASS stops the
/// channel when the slide reaches zero, which autofrees an autofree stream.
/// OR `BASS_SLIDE_LOG` into `attrib` for a logarithmic slide.
pub fn channel_slide_attribute(
    handle: u32,
    attrib: u32,
    value: f32,
    time_ms: u32,
) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain scalars.
    check(unsafe { (a.channel_slide_attribute)(handle, attrib, value, time_ms) })
}

/// `BASS_ChannelIsSliding(handle, attrib)`; `attrib` 0 asks "any attribute".
pub fn channel_is_sliding(handle: u32, attrib: u32) -> bool {
    match lib() {
        // SAFETY: plain integers.
        Ok(a) => unsafe { (a.channel_is_sliding)(handle, attrib) != 0 },
        Err(_) => false,
    }
}

/// `BASS_ChannelGetLength(handle, BASS_POS_BYTE)`. BASS returns `(QWORD)-1`
/// on error (and for an endless internet stream).
pub fn channel_length_bytes(handle: u32) -> Result<u64, BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    let v = unsafe { (a.channel_get_length)(handle, BASS_POS_BYTE) };
    if v == u64::MAX {
        Err(BassError::last())
    } else {
        Ok(v)
    }
}

/// `BASS_ChannelGetPosition(handle, BASS_POS_BYTE)`.
pub fn channel_position_bytes(handle: u32) -> Result<u64, BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    let v = unsafe { (a.channel_get_position)(handle, BASS_POS_BYTE) };
    if v == u64::MAX {
        Err(BassError::last())
    } else {
        Ok(v)
    }
}

/// `BASS_ChannelSetPosition(handle, pos, BASS_POS_BYTE)`. Safe to call from
/// inside a mixtime sync on the same channel -- that is how a seamless loop
/// seeks back to its start.
pub fn channel_set_position_bytes(handle: u32, pos: u64) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    check(unsafe { (a.channel_set_position)(handle, pos, BASS_POS_BYTE) })
}

/// `BASS_ChannelBytes2Seconds`. BASS returns a negative value on error.
pub fn bytes_to_seconds(handle: u32, bytes: u64) -> Result<f64, BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    let v = unsafe { (a.channel_bytes2seconds)(handle, bytes) };
    if v < 0.0 {
        Err(BassError::last())
    } else {
        Ok(v)
    }
}

/// `BASS_ChannelSeconds2Bytes`. BASS returns `(QWORD)-1` on error.
pub fn seconds_to_bytes(handle: u32, seconds: f64) -> Result<u64, BassError> {
    let a = lib()?;
    // SAFETY: plain scalars.
    let v = unsafe { (a.channel_seconds2bytes)(handle, seconds) };
    if v == u64::MAX {
        Err(BassError::last())
    } else {
        Ok(v)
    }
}

/// Length in seconds: `channel_length_bytes` through `bytes_to_seconds`, the
/// pair a caller uses to anchor a crossfade to a track's own duration.
pub fn channel_length_seconds(handle: u32) -> Result<f64, BassError> {
    bytes_to_seconds(handle, channel_length_bytes(handle)?)
}

/// `BASS_ChannelFlags(handle, flags, mask)`: set the bits of `mask` to
/// `flags` and return the channel's new flags. `mask` 0 just reads them.
pub fn channel_flags(handle: u32, flags: u32, mask: u32) -> Result<u32, BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    let v = unsafe { (a.channel_flags)(handle, flags, mask) };
    if v == u32::MAX {
        Err(BassError::last())
    } else {
        Ok(v)
    }
}

/// Set or clear `BASS_SAMPLE_LOOP` on the channel.
pub fn set_looping(handle: u32, looping: bool) -> Result<(), BassError> {
    let flags = if looping { BASS_SAMPLE_LOOP } else { 0 };
    channel_flags(handle, flags, BASS_SAMPLE_LOOP).map(|_| ())
}

/// Is `BASS_SAMPLE_LOOP` set?
pub fn is_looping(handle: u32) -> Result<bool, BassError> {
    Ok(channel_flags(handle, 0, 0)? & BASS_SAMPLE_LOOP != 0)
}

/// Owned copy of `BASS_CHANNELINFO`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    pub freq: u32,
    pub chans: u32,
    pub flags: u32,
    pub ctype: u32,
    pub origres: u32,
    pub plugin: HPLUGIN,
    pub sample: HSAMPLE,
    pub filename: Option<String>,
}

/// `BASS_ChannelGetInfo(handle)`.
pub fn channel_info(handle: u32) -> Result<ChannelInfo, BassError> {
    let a = lib()?;
    let mut raw = BASS_CHANNELINFO {
        freq: 0,
        chans: 0,
        flags: 0,
        ctype: 0,
        origres: 0,
        plugin: 0,
        sample: 0,
        filename: ptr::null(),
    };
    // SAFETY: `raw` is a correctly laid-out, writable BASS_CHANNELINFO.
    check(unsafe { (a.channel_get_info)(handle, &mut raw) })?;
    // SAFETY: BASS filled `filename` with NULL or a C string owned by the
    // channel, valid while the channel exists -- and we copy it at once.
    let filename = unsafe { c_string_at(raw.filename) };
    Ok(ChannelInfo {
        freq: raw.freq,
        chans: raw.chans,
        flags: raw.flags,
        ctype: raw.ctype,
        origres: raw.origres,
        plugin: raw.plugin,
        sample: raw.sample,
        filename,
    })
}

/// The channel's default playback rate (`BASS_CHANNELINFO.freq`), which the
/// engine bands and road noise scale with `BASS_ATTRIB_FREQ`.
pub fn channel_frequency(handle: u32) -> Result<u32, BassError> {
    Ok(channel_info(handle)?.freq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_names_and_display() {
        let e = BassError {
            code: BASS_ERROR_HANDLE,
        };
        assert_eq!(e.name(), "BASS_ERROR_HANDLE");
        assert_eq!(e.to_string(), "BASS_ERROR_HANDLE (5)");
        assert_eq!(BassError::NOT_LOADED.name(), "BASS_NOT_LOADED");
        assert_eq!(BassError { code: 999 }.name(), "BASS_ERROR_?");
    }

    #[test]
    fn plugin_file_names() {
        if cfg!(windows) {
            assert!(is_plugin_file_name("bassopus.dll"));
            assert!(is_plugin_file_name("BASS_AAC.DLL"));
            assert!(!is_plugin_file_name("bass.dll"));
            // bass_fx is a library, not a plugin: BASS_PluginLoad refuses it.
            assert!(!is_plugin_file_name("bass_fx.dll"));
            assert!(!is_plugin_file_name("BASS_FX.DLL"));
            assert!(!is_plugin_file_name("basshls.txt"));
            assert!(!is_plugin_file_name("prism.dll"));
        } else {
            assert!(is_plugin_file_name("libbassopus.so"));
            assert!(is_plugin_file_name("libbasshls.dylib"));
            assert!(!is_plugin_file_name("libbass.so"));
            assert!(!is_plugin_file_name("libbass.dylib"));
            assert!(!is_plugin_file_name("libbass_fx.so"));
            assert!(!is_plugin_file_name("libbass_fx.dylib"));
        }
    }

    #[test]
    fn latin1_fallback_for_tags() {
        assert_eq!(decode_c_string(b"caf\xc3\xa9"), "caf\u{e9}");
        assert_eq!(decode_c_string(b"caf\xe9"), "caf\u{e9}");
    }

    #[test]
    fn calls_without_a_library_fail_softly() {
        // Whatever the machine has, these must not panic; without the DLL
        // they all report NOT_LOADED.
        if !native_available() {
            assert_eq!(
                init(BASS_NO_SOUND_DEVICE, 44100, 0),
                Err(BassError::NOT_LOADED)
            );
            assert_eq!(channel_is_active(1), BASS_ACTIVE_STOPPED);
            assert_eq!(tags_meta(1), None);
            assert_eq!(
                set_3d_factors(1.0, 1.0, 1.0),
                Err(BassError::NOT_LOADED),
                "3D calls must report NOT_LOADED, not panic"
            );
            assert_eq!(
                set_3d_position(
                    BASS_3DVECTOR::ZERO,
                    BASS_3DVECTOR::ZERO,
                    BASS_3DVECTOR::ZERO
                ),
                Err(BassError::NOT_LOADED)
            );
            assert_eq!(
                channel_set_3d_position(1, BASS_3DVECTOR::ZERO),
                Err(BassError::NOT_LOADED)
            );
            assert_eq!(
                channel_set_3d_attributes(1, BASS_3DMODE_NORMAL, 1.0, 30.0),
                Err(BassError::NOT_LOADED)
            );
            assert_eq!(
                channel_set_fx(1, BASS_FX_BFX_BQF, 0).unwrap_err(),
                BassError::NOT_LOADED
            );
            assert_eq!(channel_remove_fx(1, 1), Err(BassError::NOT_LOADED));
            // No sound, no error: apply_3d has nothing to report either way.
            apply_3d();
        }
    }
}
