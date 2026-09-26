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

// --- Streams ------------------------------------------------------------------------

/// A BASS stream handle plus, for memory streams, the buffer it decodes from.
///
/// `BASS_StreamCreateFile(mem=TRUE, ..)` does not copy the data: BASS reads
/// the caller's buffer during playback, and the documentation requires the
/// memory to stay valid until the stream is freed. So the bytes are pinned
/// here, in an `Arc<[u8]>` (one asset buffer can back many concurrent
/// one-shots without a copy each), and released only after `BASS_StreamFree`
/// has returned in [`Drop`].
///
/// Dropping a `Stream` frees the BASS stream even if it is still playing. A
/// one-shot created with `BASS_STREAM_AUTOFREE` must therefore be retained by
/// the caller until `channel_is_active` says it has stopped. If BASS has
/// already autofreed the handle, the free call in `Drop` just fails with
/// `BASS_ERROR_HANDLE` and is ignored.
#[derive(Debug)]
pub struct Stream {
    handle: HSTREAM,
    _buffer: Option<Arc<[u8]>>,
    /// A MOD music handle (`BASS_MusicLoad`), freed with `BASS_MusicFree`.
    music: bool,
}

// A handle is a number and BASS is thread-safe; the buffer is immutable.
unsafe impl Send for Stream {}
unsafe impl std::marker::Sync for Stream {}

impl Stream {
    /// The raw handle, for the `channel_*` functions.
    pub fn handle(&self) -> HSTREAM {
        self.handle
    }

    /// The pinned buffer, for a memory stream.
    pub fn buffer(&self) -> Option<&Arc<[u8]>> {
        self._buffer.as_ref()
    }

    /// Free the stream now and report whether BASS still knew it.
    pub fn free(mut self) -> Result<(), BassError> {
        let result = check(self.free_handle(lib()?));
        // Drop skips a zero handle, and releases the buffer only then -- after
        // the free call has returned, which is the ordering that matters.
        self.handle = 0;
        result
    }

    /// Give up ownership: the handle is returned and the buffer is leaked so
    /// BASS can keep reading it. For the caller who will never free the
    /// stream (an autofree one-shot they do not want to track).
    pub fn into_raw(self) -> HSTREAM {
        let handle = self.handle;
        std::mem::forget(self);
        handle
    }

    /// `BASS_MusicFree` for a music handle, `BASS_StreamFree` otherwise.
    fn free_handle(&self, a: &Api) -> BOOL {
        // SAFETY: plain integer; BASS validates the handle. One that
        // autofreed already answers BASS_ERROR_HANDLE, which is fine.
        unsafe {
            if self.music {
                (a.music_free)(self.handle)
            } else {
                (a.stream_free)(self.handle)
            }
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if self.handle == 0 {
            return;
        }
        if let Ok(a) = lib() {
            self.free_handle(a);
        }
        // `_buffer` drops after this, i.e. after the free call has returned.
    }
}

/// `BASS_StreamCreateFile(TRUE, data, 0, len, flags)` over a copy of `data`.
///
/// The copy is what keeps the buffer alive for BASS (see [`Stream`]). For an
/// asset played many times, keep an `Arc<[u8]>` and use
/// [`stream_create_mem_shared`] to skip the copy.
pub fn stream_create_mem(data: &[u8], flags: u32) -> Result<Stream, BassError> {
    stream_create_mem_shared(Arc::from(data), flags)
}

/// `BASS_StreamCreateFile(TRUE, ..)` over a shared buffer, no copy.
pub fn stream_create_mem_shared(data: Arc<[u8]>, flags: u32) -> Result<Stream, BassError> {
    let a = lib()?;
    // SAFETY: `data` is a valid buffer of `len` bytes and is pinned inside
    // the returned Stream until after StreamFree.
    let handle =
        unsafe { (a.stream_create_file)(1, data.as_ptr().cast(), 0, data.len() as QWORD, flags) };
    if handle == 0 {
        return Err(BassError::last());
    }
    Ok(Stream {
        handle,
        _buffer: Some(data),
        music: false,
    })
}

/// `BASS_MusicLoad(TRUE, ..)` over a shared buffer: a tracker module (IT,
/// XM, S3M, MOD, MO3) as a playable channel. BASS copies module data on
/// load, but the buffer is pinned like a stream's for one ownership rule.
pub fn music_load_mem_shared(data: Arc<[u8]>, flags: u32) -> Result<Stream, BassError> {
    let a = lib()?;
    let len = DWORD::try_from(data.len()).map_err(|_| BassError {
        code: BASS_ERROR_FILEFORM,
    })?;
    // SAFETY: `data` is a valid buffer of `len` bytes, alive for the call
    // and pinned in the returned Stream until after MusicFree.
    let handle = unsafe { (a.music_load)(1, data.as_ptr().cast(), 0, len, flags, 0) };
    if handle == 0 {
        return Err(BassError::last());
    }
    Ok(Stream {
        handle,
        _buffer: Some(data),
        music: true,
    })
}

/// `BASS_StreamCreateURL(url, 0, flags, NULL, NULL)`.
///
/// Blocks until the connection is made or `BASS_CONFIG_NET_TIMEOUT` expires,
/// so callers use a worker thread, never their frame loop. ICY/HTTP tags are
/// read afterwards with [`tags_meta`] / [`tags_strings`]; no `DOWNLOADPROC`
/// is installed.
pub fn stream_create_url(url: &str, flags: u32) -> Result<Stream, BassError> {
    let a = lib()?;
    let c = CString::new(url).map_err(|_| BassError {
        code: BASS_ERROR_ILLPARAM,
    })?;
    // SAFETY: `c` is NUL-terminated and outlives the call; BASS copies the
    // URL it needs. No download callback, so `user` is unused.
    let handle =
        unsafe { (a.stream_create_url)(c.as_ptr().cast(), 0, flags, None, ptr::null_mut()) };
    if handle == 0 {
        return Err(BassError::last());
    }
    Ok(Stream {
        handle,
        _buffer: None,
        music: false,
    })
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

// --- 3D sound ---------------------------------------------------------------------

/// `BASS_Set3DFactors(distf, rollf, doppf)`: the units, rolloff and Doppler
/// scaling the whole 3D mix uses.
///
/// `distf` is metres per world unit, `rollf` how quickly sound quietens with
/// distance (1.0 is real world), `doppf` how strong Doppler shift is. `-1.0`
/// in any slot leaves that factor alone. Takes effect on the next
/// [`apply_3d`].
pub fn set_3d_factors(distf: f32, rolloff: f32, doppf: f32) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain floats.
    check(unsafe { (a.set_3d_factors)(distf, rolloff, doppf) })
}

/// `BASS_Set3DPosition(&pos, NULL, &front, &top)`: where the listener is and
/// which way they face.
///
/// Velocity is NULL on purpose. It only feeds Doppler shift, and a walking
/// character never moves fast enough for that to be anything but an
/// artefact; passing a velocity derived from frame-to-frame position
/// differences makes footsteps warble. Takes effect on the next
/// [`apply_3d`].
pub fn set_3d_position(
    pos: BASS_3DVECTOR,
    front: BASS_3DVECTOR,
    top: BASS_3DVECTOR,
) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: three readable vectors that outlive the call; `vel` is
    // documented as NULL-able ("NULL = leave current").
    check(unsafe { (a.set_3d_position)(&pos, ptr::null(), &front, &top) })
}

/// `BASS_Apply3D()`: commit the listener and channel 3D changes made since
/// the last call.
///
/// Returns nothing, and BASS sets no error code for it, so an unloaded
/// library is simply a no-op -- the same shape as [`channel_is_active`].
/// Call it once per frame, after moving the listener and every source.
pub fn apply_3d() {
    if let Ok(a) = lib() {
        // SAFETY: no arguments.
        unsafe { (a.apply_3d)() }
    }
}

/// `BASS_ChannelSet3DPosition(handle, &pos, NULL, NULL)`: where a source is.
///
/// Orientation is NULL because the sources are omnidirectional (see
/// [`channel_set_3d_attributes`]) and velocity is NULL for the same reason it
/// is on the listener. Takes effect on the next [`apply_3d`].
pub fn channel_set_3d_position(handle: u32, pos: BASS_3DVECTOR) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: `pos` outlives the call; `orient` and `vel` are NULL-able.
    check(unsafe { (a.channel_set_3d_position)(handle, &pos, ptr::null(), ptr::null()) })
}

/// `BASS_ChannelSet3DAttributes(handle, mode, min, max, 360, 360, 1.0)`.
///
/// `mode` is one of the `BASS_3DMODE_*` values, `min` the distance below
/// which the sound stops getting louder and `max` the distance beyond which
/// it stops getting quieter. The cone is fixed at 360/360 degrees with an
/// outside volume of 1.0: every source is omnidirectional, and a cone that
/// covers the whole sphere is how BASS spells that.
pub fn channel_set_3d_attributes(
    handle: u32,
    mode: i32,
    min: f32,
    max: f32,
) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain scalars.
    check(unsafe { (a.channel_set_3d_attributes)(handle, mode, min, max, 360, 360, 1.0) })
}

// --- Channel effects --------------------------------------------------------------

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

/// `BASS_ChannelRemoveFX(handle, fx)`. Freeing the channel removes its
/// effects too, so this is only needed to drop one early.
pub fn channel_remove_fx(handle: u32, fx: HFX) -> Result<(), BassError> {
    let a = lib()?;
    // SAFETY: plain integers.
    check(unsafe { (a.channel_remove_fx)(handle, fx) })
}

/// `BASS_FXSetParameters(fx, &params)` for a `BASS_FX_BFX_BQF` effect.
///
/// The C call takes `const void *` and reads the struct the effect's type
/// implies, so the type-safety is in the caller pairing this with an `fx`
/// created as `BASS_FX_BFX_BQF`. That is why there is one wrapper per effect
/// rather than a generic one.
pub fn fx_set_bqf(fx: HFX, params: &BASS_BFX_BQF) -> Result<(), BassError> {
    let a = lib()?;
    let raw: *const BASS_BFX_BQF = params;
    // SAFETY: `params` is a live BASS_BFX_BQF, which is what a
    // BASS_FX_BFX_BQF effect reads, and it outlives the call.
    check(unsafe { (a.fx_set_parameters)(fx, raw.cast::<c_void>()) })
}

// --- Tags -----------------------------------------------------------------------------

/// `BASS_ChannelGetTags(handle, BASS_TAG_META)`: the latest ICY metadata
/// block (`StreamTitle='...';`), copied out as a string.
///
/// `None` when there is no metadata, the handle is gone, or the library is
/// not loaded. UTF-8 when the server sent UTF-8, latin-1 otherwise. Parsing
/// the `StreamTitle` out of it is the caller's job.
pub fn tags_meta(handle: u32) -> Option<String> {
    tags_string(handle, BASS_TAG_META)
}

/// `BASS_ChannelGetTags(handle, kind)` for the tag kinds that are one C
/// string (`BASS_TAG_META`, `BASS_TAG_HLS_EXTINF`), copied out.
pub fn tags_string(handle: u32, kind: u32) -> Option<String> {
    let a = lib().ok()?;
    // SAFETY: plain integers; the returned pointer is NULL or a C string in
    // BASS's buffer, valid until the next metadata update, and we copy it
    // before returning.
    unsafe { c_string_at((a.channel_get_tags)(handle, kind)) }
}

/// `BASS_ChannelGetTags(handle, kind)` for the list-shaped tag kinds
/// (`BASS_TAG_ICY`, `BASS_TAG_HTTP`, `BASS_TAG_OGG`, ...): a series of
/// NUL-terminated strings ended by an empty one, copied out.
pub fn tags_strings(handle: u32, kind: u32) -> Option<Vec<String>> {
    let a = lib().ok()?;
    // SAFETY: plain integers; NULL or a double-NUL-terminated block.
    let mut p = unsafe { (a.channel_get_tags)(handle, kind) };
    if p.is_null() {
        return None;
    }
    let mut out = Vec::new();
    loop {
        // SAFETY: each string is NUL-terminated and the block ends with an
        // empty string, so we never read past it.
        let item = unsafe { CStr::from_ptr(p) };
        let bytes = item.to_bytes();
        if bytes.is_empty() {
            break;
        }
        out.push(decode_c_string(bytes));
        // SAFETY: step over this string and its terminator to the next.
        p = unsafe { p.add(bytes.len() + 1) };
    }
    Some(out)
}

// --- Syncs -----------------------------------------------------------------------------

/// The closure behind a sync. `(sync handle, channel, data)`.
pub type SyncCallback = Box<dyn FnMut(HSYNC, u32, u32) + Send + 'static>;

/// The `SYNCPROC` BASS calls; `user` is the `*mut SyncCallback`.
///
/// Runs on a BASS thread: for `BASS_SYNC_MIXTIME` syncs the mixer/update
/// thread (inside the channel lock, so keep it short and do not wait on the
/// main thread), otherwise BASS's sync thread. A panic must not cross back
/// into BASS, so it is caught and logged.
unsafe extern "system" fn sync_trampoline(
    handle: HSYNC,
    channel: DWORD,
    data: DWORD,
    user: *mut c_void,
) {
    let callback = &mut *(user as *mut SyncCallback);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback(handle, channel, data)
    }));
    if outcome.is_err() {
        log::error!("bass: sync callback panicked (sync {handle}, channel {channel})");
    }
}

/// A registered sync and the closure it calls.
///
/// BASS holds a raw pointer to the closure for as long as the sync exists,
/// so the guard keeps the boxed closure alive and removes the sync on drop
/// (or in [`SyncGuard::remove`]), freeing the closure only after
/// `BASS_ChannelRemoveSync` has returned.
///
/// Thread contract: the closure runs on BASS's thread, never the caller's.
/// For a `BASS_SYNC_POS | BASS_SYNC_MIXTIME` sync that seeks the same channel
/// back to its loop start, that is the mixer thread, and
/// `channel_set_position_bytes` from inside it is explicitly allowed.
/// Removal is safe from any thread including the callback itself.
///
/// If the channel is freed first (an autofree stream ending), BASS removes
/// the sync itself and a later `remove`/drop just sees `BASS_ERROR_HANDLE`;
/// the closure is freed either way.
pub struct SyncGuard {
    channel: u32,
    handle: HSYNC,
    callback: *mut SyncCallback,
}

// The closure is `Send` and only BASS's thread calls it; the guard itself
// just owns the box and two numbers.
unsafe impl Send for SyncGuard {}

impl fmt::Debug for SyncGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncGuard")
            .field("channel", &self.channel)
            .field("handle", &self.handle)
            .finish()
    }
}

impl SyncGuard {
    /// The `HSYNC` BASS gave out.
    pub fn handle(&self) -> HSYNC {
        self.handle
    }

    /// The channel the sync is on.
    pub fn channel(&self) -> u32 {
        self.channel
    }

    fn remove_inner(&mut self) -> Result<(), BassError> {
        if self.callback.is_null() {
            return Ok(());
        }
        let result = lib().and_then(|a| {
            // SAFETY: plain integers; BASS validates both handles.
            check(unsafe { (a.channel_remove_sync)(self.channel, self.handle) })
        });
        // After BASS_ChannelRemoveSync returns the callback will not be
        // called again -- BASS finishes any in-flight call before the remove
        // completes (the sync list is walked under the channel lock), and a
        // channel that was already freed took its syncs with it. So the
        // closure can go now.
        // SAFETY: the pointer came from Box::into_raw in `set_sync` and is
        // freed exactly once, here.
        unsafe { drop(Box::from_raw(self.callback)) };
        self.callback = ptr::null_mut();
        result
    }

    /// `BASS_ChannelRemoveSync` now, and free the closure.
    pub fn remove(mut self) -> Result<(), BassError> {
        self.remove_inner()
    }

    /// Leave the sync registered for the channel's lifetime and leak the
    /// closure. For a sync that should outlive every guard (rare).
    pub fn forget(self) -> HSYNC {
        let handle = self.handle;
        std::mem::forget(self);
        handle
    }
}

impl Drop for SyncGuard {
    fn drop(&mut self) {
        let _ = self.remove_inner();
    }
}

/// `BASS_ChannelSetSync(handle, type, param, trampoline, closure)`.
///
/// `type_` is a `BASS_SYNC_*` value OR'd with `BASS_SYNC_MIXTIME` /
/// `BASS_SYNC_ONETIME` as needed; `param` is the type's parameter (the byte
/// position for `BASS_SYNC_POS`). See [`SyncGuard`] for who calls the closure and
/// when it is freed.
pub fn set_sync(
    handle: u32,
    type_: u32,
    param: u64,
    callback: SyncCallback,
) -> Result<SyncGuard, BassError> {
    let a = lib()?;
    // Double box: BASS gets a thin pointer to the Box<dyn FnMut>.
    let user: *mut SyncCallback = Box::into_raw(Box::new(callback));
    // SAFETY: the trampoline matches SYNCPROC and `user` stays valid until
    // the sync is removed (see SyncGuard::remove_inner).
    let sync =
        unsafe { (a.channel_set_sync)(handle, type_, param, Some(sync_trampoline), user.cast()) };
    if sync == 0 {
        let err = BassError::last();
        // SAFETY: BASS never saw a valid sync, so nothing else holds `user`.
        unsafe { drop(Box::from_raw(user)) };
        return Err(err);
    }
    Ok(SyncGuard {
        channel: handle,
        handle: sync,
        callback: user,
    })
}

/// `BASS_ChannelRemoveSync` through the guard. Same as [`SyncGuard::remove`]; this
/// spelling exists so the call reads like the C one.
pub fn remove_sync(sync: SyncGuard) -> Result<(), BassError> {
    sync.remove()
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
