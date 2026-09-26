//! Raw FFI types and runtime loader for the BASS audio library (Un4seen).
//!
//! BASS ships as a closed C library (`bass.dll` / `libbass.so` /
//! `libbass.dylib`) plus add-on plugins, so the C ABI is declared by hand here
//! and the platform binaries are vendored under `vendor/<os>-<arch>/`.
//!
//! The library is opened at run time rather than linked, so a machine without
//! BASS still starts a game built on it -- it just loses audio, degrading the
//! same way a missing optional codec would. See [`Api::get`] and [`api`].
//!
//! Only the entry points games built on this crate have needed so far are
//! declared: memory streams, URL streams, MOD music, attribute slides,
//! mixtime position syncs, ICY tags, plugins, 3D placement and biquad
//! filtering. Adding a call site means adding a function-pointer type here, a field in
//! [`loader::Api`] and a matching `load` line.
//!
//! Calling convention: BASS exports are `WINAPI` (stdcall) on 32-bit Windows
//! and plain C everywhere else; `extern "system"` is exactly that rule.

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]

use std::os::raw::{c_char, c_int, c_void};

pub mod fx;
mod loader;
pub mod safe;

pub use loader::{library_file_name, Api, LoadError, LIBRARY_PATH_ENV};

// --- Primitive aliases, matching bass.h -------------------------------------

/// `DWORD` -- 32-bit unsigned on every BASS platform.
pub type DWORD = u32;
/// `QWORD` -- 64-bit unsigned.
pub type QWORD = u64;
/// `BOOL` -- C int, non-zero is true.
pub type BOOL = c_int;
/// Sample stream handle.
pub type HSTREAM = DWORD;
/// Playing sample's channel handle (also what every `BASS_Channel*` takes).
pub type HCHANNEL = DWORD;
/// Synchroniser handle.
pub type HSYNC = DWORD;
/// Plugin handle.
pub type HPLUGIN = DWORD;
/// Sample handle.
pub type HSAMPLE = DWORD;
/// MOD music handle.
pub type HMUSIC = DWORD;
/// Channel effect handle (`BASS_ChannelSetFX`).
pub type HFX = DWORD;

/// The "no sound" device passed to `BASS_Init`: channels play and decode in
/// real time, syncs fire, nothing is audible. Headless runs and tests use it.
pub const BASS_NO_SOUND_DEVICE: c_int = 0;
/// The default device for `BASS_Init`.
pub const BASS_DEFAULT_DEVICE: c_int = -1;

/// `BASS_GetVersion` for the 2.4 line returns `0x0204xxxx`.
pub const BASSVERSION: DWORD = 0x204;

// --- Error codes (BASS_ErrorGetCode) ----------------------------------------

pub const BASS_OK: c_int = 0;
pub const BASS_ERROR_MEM: c_int = 1;
pub const BASS_ERROR_FILEOPEN: c_int = 2;
pub const BASS_ERROR_DRIVER: c_int = 3;
pub const BASS_ERROR_BUFLOST: c_int = 4;
pub const BASS_ERROR_HANDLE: c_int = 5;
pub const BASS_ERROR_FORMAT: c_int = 6;
pub const BASS_ERROR_POSITION: c_int = 7;
pub const BASS_ERROR_INIT: c_int = 8;
pub const BASS_ERROR_START: c_int = 9;
pub const BASS_ERROR_SSL: c_int = 10;
pub const BASS_ERROR_REINIT: c_int = 11;
pub const BASS_ERROR_ALREADY: c_int = 14;
pub const BASS_ERROR_NOTAUDIO: c_int = 17;
pub const BASS_ERROR_NOCHAN: c_int = 18;
pub const BASS_ERROR_ILLTYPE: c_int = 19;
pub const BASS_ERROR_ILLPARAM: c_int = 20;
pub const BASS_ERROR_NO3D: c_int = 21;
pub const BASS_ERROR_NOEAX: c_int = 22;
pub const BASS_ERROR_DEVICE: c_int = 23;
pub const BASS_ERROR_NOPLAY: c_int = 24;
pub const BASS_ERROR_FREQ: c_int = 25;
pub const BASS_ERROR_NOTFILE: c_int = 27;
pub const BASS_ERROR_NOHW: c_int = 29;
pub const BASS_ERROR_EMPTY: c_int = 31;
pub const BASS_ERROR_NONET: c_int = 32;
pub const BASS_ERROR_CREATE: c_int = 33;
pub const BASS_ERROR_NOFX: c_int = 34;
pub const BASS_ERROR_NOTAVAIL: c_int = 37;
pub const BASS_ERROR_DECODE: c_int = 38;
pub const BASS_ERROR_DX: c_int = 39;
pub const BASS_ERROR_TIMEOUT: c_int = 40;
pub const BASS_ERROR_FILEFORM: c_int = 41;
pub const BASS_ERROR_SPEAKER: c_int = 42;
pub const BASS_ERROR_VERSION: c_int = 43;
pub const BASS_ERROR_CODEC: c_int = 44;
pub const BASS_ERROR_ENDED: c_int = 45;
pub const BASS_ERROR_BUSY: c_int = 46;
pub const BASS_ERROR_UNSTREAMABLE: c_int = 47;
pub const BASS_ERROR_PROTOCOL: c_int = 48;
pub const BASS_ERROR_DENIED: c_int = 49;
pub const BASS_ERROR_UNKNOWN: c_int = -1;

/// Human-readable name for a BASS error code, `None` for an unknown one.
pub const fn error_name(code: c_int) -> Option<&'static str> {
    Some(match code {
        BASS_OK => "BASS_OK",
        BASS_ERROR_MEM => "BASS_ERROR_MEM",
        BASS_ERROR_FILEOPEN => "BASS_ERROR_FILEOPEN",
        BASS_ERROR_DRIVER => "BASS_ERROR_DRIVER",
        BASS_ERROR_BUFLOST => "BASS_ERROR_BUFLOST",
        BASS_ERROR_HANDLE => "BASS_ERROR_HANDLE",
        BASS_ERROR_FORMAT => "BASS_ERROR_FORMAT",
        BASS_ERROR_POSITION => "BASS_ERROR_POSITION",
        BASS_ERROR_INIT => "BASS_ERROR_INIT",
        BASS_ERROR_START => "BASS_ERROR_START",
        BASS_ERROR_SSL => "BASS_ERROR_SSL",
        BASS_ERROR_REINIT => "BASS_ERROR_REINIT",
        BASS_ERROR_ALREADY => "BASS_ERROR_ALREADY",
        BASS_ERROR_NOTAUDIO => "BASS_ERROR_NOTAUDIO",
        BASS_ERROR_NOCHAN => "BASS_ERROR_NOCHAN",
        BASS_ERROR_ILLTYPE => "BASS_ERROR_ILLTYPE",
        BASS_ERROR_ILLPARAM => "BASS_ERROR_ILLPARAM",
        BASS_ERROR_NO3D => "BASS_ERROR_NO3D",
        BASS_ERROR_NOEAX => "BASS_ERROR_NOEAX",
        BASS_ERROR_DEVICE => "BASS_ERROR_DEVICE",
        BASS_ERROR_NOPLAY => "BASS_ERROR_NOPLAY",
        BASS_ERROR_FREQ => "BASS_ERROR_FREQ",
        BASS_ERROR_NOTFILE => "BASS_ERROR_NOTFILE",
        BASS_ERROR_NOHW => "BASS_ERROR_NOHW",
        BASS_ERROR_EMPTY => "BASS_ERROR_EMPTY",
        BASS_ERROR_NONET => "BASS_ERROR_NONET",
        BASS_ERROR_CREATE => "BASS_ERROR_CREATE",
        BASS_ERROR_NOFX => "BASS_ERROR_NOFX",
        BASS_ERROR_NOTAVAIL => "BASS_ERROR_NOTAVAIL",
        BASS_ERROR_DECODE => "BASS_ERROR_DECODE",
        BASS_ERROR_DX => "BASS_ERROR_DX",
        BASS_ERROR_TIMEOUT => "BASS_ERROR_TIMEOUT",
        BASS_ERROR_FILEFORM => "BASS_ERROR_FILEFORM",
        BASS_ERROR_SPEAKER => "BASS_ERROR_SPEAKER",
        BASS_ERROR_VERSION => "BASS_ERROR_VERSION",
        BASS_ERROR_CODEC => "BASS_ERROR_CODEC",
        BASS_ERROR_ENDED => "BASS_ERROR_ENDED",
        BASS_ERROR_BUSY => "BASS_ERROR_BUSY",
        BASS_ERROR_UNSTREAMABLE => "BASS_ERROR_UNSTREAMABLE",
        BASS_ERROR_PROTOCOL => "BASS_ERROR_PROTOCOL",
        BASS_ERROR_DENIED => "BASS_ERROR_DENIED",
        BASS_ERROR_UNKNOWN => "BASS_ERROR_UNKNOWN",
        _ => return None,
    })
}

// --- BASS_SetConfig options --------------------------------------------------

pub const BASS_CONFIG_BUFFER: DWORD = 0;
pub const BASS_CONFIG_UPDATEPERIOD: DWORD = 1;
pub const BASS_CONFIG_GVOL_SAMPLE: DWORD = 4;
pub const BASS_CONFIG_GVOL_STREAM: DWORD = 5;
pub const BASS_CONFIG_GVOL_MUSIC: DWORD = 6;
pub const BASS_CONFIG_FLOATDSP: DWORD = 9;
/// Software 3D mixing algorithm; one of the `BASS_3DALG_*` values.
pub const BASS_CONFIG_3DALGORITHM: DWORD = 10;
pub const BASS_CONFIG_NET_TIMEOUT: DWORD = 11;
pub const BASS_CONFIG_NET_BUFFER: DWORD = 12;
pub const BASS_CONFIG_NET_PREBUF: DWORD = 15;
/// Pointer option (`BASS_SetConfigPtr`): the HTTP user-agent string.
pub const BASS_CONFIG_NET_AGENT: DWORD = 16;
/// Pointer option (`BASS_SetConfigPtr`): the HTTP proxy.
pub const BASS_CONFIG_NET_PROXY: DWORD = 17;
pub const BASS_CONFIG_NET_PASSIVE: DWORD = 18;
pub const BASS_CONFIG_NET_PLAYLIST: DWORD = 21;
pub const BASS_CONFIG_UPDATETHREADS: DWORD = 24;
pub const BASS_CONFIG_DEV_BUFFER: DWORD = 27;
pub const BASS_CONFIG_DEV_DEFAULT: DWORD = 36;
pub const BASS_CONFIG_NET_READTIMEOUT: DWORD = 37;
pub const BASS_CONFIG_UNICODE: DWORD = 42;
pub const BASS_CONFIG_SRC: DWORD = 43;
pub const BASS_CONFIG_SRC_SAMPLE: DWORD = 44;
pub const BASS_CONFIG_NET_PLAYLIST_DEPTH: DWORD = 59;
pub const BASS_CONFIG_NET_PREBUF_WAIT: DWORD = 60;

// --- BASS_Init flags ----------------------------------------------------------

pub const BASS_DEVICE_8BITS: DWORD = 1;
pub const BASS_DEVICE_MONO: DWORD = 2;
/// Request 3D support from the device. bass.h marks it "unused" as of
/// 2.4.17 -- 3D is always available -- but the flag is still accepted, and
/// passing it documents the intent at the `BASS_Init` call site.
pub const BASS_DEVICE_3D: DWORD = 4;
pub const BASS_DEVICE_16BITS: DWORD = 8;
pub const BASS_DEVICE_REINIT: DWORD = 128;
pub const BASS_DEVICE_LATENCY: DWORD = 0x100;
pub const BASS_DEVICE_CPSPEAKERS: DWORD = 0x400;
pub const BASS_DEVICE_SPEAKERS: DWORD = 0x800;
pub const BASS_DEVICE_NOSPEAKER: DWORD = 0x1000;
pub const BASS_DEVICE_DMIX: DWORD = 0x2000;
pub const BASS_DEVICE_FREQ: DWORD = 0x4000;
pub const BASS_DEVICE_STEREO: DWORD = 0x8000;
pub const BASS_DEVICE_HOG: DWORD = 0x10000;
pub const BASS_DEVICE_AUDIOTRACK: DWORD = 0x20000;
pub const BASS_DEVICE_DSOUND: DWORD = 0x40000;
pub const BASS_DEVICE_SOFTWARE: DWORD = 0x80000;

// --- BASS_DEVICEINFO flags ----------------------------------------------------

pub const BASS_DEVICE_ENABLED: DWORD = 1;
pub const BASS_DEVICE_DEFAULT: DWORD = 2;
pub const BASS_DEVICE_INIT: DWORD = 4;
pub const BASS_DEVICE_LOOPBACK: DWORD = 8;
pub const BASS_DEVICE_DEFAULTCOM: DWORD = 128;
pub const BASS_DEVICE_TYPE_MASK: DWORD = 0xff00_0000;

// --- Sample / stream flags ----------------------------------------------------

pub const BASS_SAMPLE_8BITS: DWORD = 1;
pub const BASS_SAMPLE_MONO: DWORD = 2;
pub const BASS_SAMPLE_LOOP: DWORD = 4;
pub const BASS_SAMPLE_3D: DWORD = 8;
pub const BASS_SAMPLE_SOFTWARE: DWORD = 16;
pub const BASS_SAMPLE_FLOAT: DWORD = 256;
pub const BASS_STREAM_PRESCAN: DWORD = 0x20000;
/// Free the stream automatically when it stops or ends. Every one-shot in the
/// game is created with this; the "fade to -1 volume" idiom relies on it.
pub const BASS_STREAM_AUTOFREE: DWORD = 0x40000;
pub const BASS_STREAM_RESTRATE: DWORD = 0x80000;
/// Download and play an internet stream in small blocks, rather than
/// buffering it all up front.
pub const BASS_STREAM_BLOCK: DWORD = 0x100000;
pub const BASS_STREAM_DECODE: DWORD = 0x200000;
/// Give server status info (HTTP/ICY tags) to the `DOWNLOADPROC`.
pub const BASS_STREAM_STATUS: DWORD = 0x800000;
/// `file`/`url` parameters are UTF-16 (Windows) rather than ANSI.
pub const BASS_UNICODE: DWORD = 0x8000_0000;

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

/// Slide attribute logarithmically (`BASS_ChannelSlideAttribute` flag).
pub const BASS_SLIDE_LOG: DWORD = 0x100_0000;

// --- Channel attributes ------------------------------------------------------

pub const BASS_ATTRIB_FREQ: DWORD = 1;
pub const BASS_ATTRIB_VOL: DWORD = 2;
pub const BASS_ATTRIB_PAN: DWORD = 3;
pub const BASS_ATTRIB_EAXMIX: DWORD = 4;
pub const BASS_ATTRIB_NOBUFFER: DWORD = 5;
pub const BASS_ATTRIB_VBR: DWORD = 6;
pub const BASS_ATTRIB_CPU: DWORD = 7;
pub const BASS_ATTRIB_SRC: DWORD = 8;
pub const BASS_ATTRIB_NET_RESUME: DWORD = 9;
pub const BASS_ATTRIB_SCANINFO: DWORD = 10;
pub const BASS_ATTRIB_NORAMP: DWORD = 11;
pub const BASS_ATTRIB_BITRATE: DWORD = 12;
pub const BASS_ATTRIB_BUFFER: DWORD = 13;
pub const BASS_ATTRIB_GRANULE: DWORD = 14;
pub const BASS_ATTRIB_USER: DWORD = 15;
pub const BASS_ATTRIB_TAIL: DWORD = 16;
pub const BASS_ATTRIB_PUSH_LIMIT: DWORD = 17;

// --- 3D sound -------------------------------------------------------------------

/// Normal 3D processing: the channel's position is in world space.
pub const BASS_3DMODE_NORMAL: c_int = 0;
/// The channel's position is relative to the listener rather than absolute.
pub const BASS_3DMODE_RELATIVE: c_int = 1;
/// No 3D processing: the channel plays flat, wherever it was placed.
pub const BASS_3DMODE_OFF: c_int = 2;

/// Default software 3D mixing algorithm (`BASS_CONFIG_3DALGORITHM`).
pub const BASS_3DALG_DEFAULT: DWORD = 0;
/// No software 3D processing (`BASS_CONFIG_3DALGORITHM`).
pub const BASS_3DALG_OFF: DWORD = 1;

// --- bass_fx BFX effect types and channel flags -----------------------------------

/// The BFX effect types `bass_fx` adds to `BASS_ChannelSetFX`, numbered from
/// `BASS_FX_BFX_ROTATE = 0x10000` in bass_fx.h. Only the biquad filter is
/// declared: it is the one games built on this crate have needed, as a low
/// pass for muffling a sound behind an obstacle.
///
/// `BASS_FX_BFX_BQF` is the twentieth of that enum, hence `0x10013`.
pub const BASS_FX_BFX_BQF: DWORD = 0x10013;

/// `BASS_BFX_BQF.lFilter` values, in bass_fx.h's enum order.
pub const BASS_BFX_BQF_LOWPASS: c_int = 0;
pub const BASS_BFX_BQF_HIGHPASS: c_int = 1;
pub const BASS_BFX_BQF_BANDPASS: c_int = 2;
pub const BASS_BFX_BQF_BANDPASS_Q: c_int = 3;
pub const BASS_BFX_BQF_NOTCH: c_int = 4;
pub const BASS_BFX_BQF_ALLPASS: c_int = 5;
pub const BASS_BFX_BQF_PEAKINGEQ: c_int = 6;
pub const BASS_BFX_BQF_LOWSHELF: c_int = 7;
pub const BASS_BFX_BQF_HIGHSHELF: c_int = 8;

/// Apply a BFX effect to every channel of the stream (the default).
pub const BASS_BFX_CHANALL: c_int = -1;
/// Disable a BFX effect on every channel.
pub const BASS_BFX_CHANNONE: c_int = 0;

// --- BASS_ChannelIsActive results ---------------------------------------------

pub const BASS_ACTIVE_STOPPED: DWORD = 0;
pub const BASS_ACTIVE_PLAYING: DWORD = 1;
pub const BASS_ACTIVE_STALLED: DWORD = 2;
pub const BASS_ACTIVE_PAUSED: DWORD = 3;
pub const BASS_ACTIVE_PAUSED_DEVICE: DWORD = 4;

// --- Sync types and flags -------------------------------------------------------

pub const BASS_SYNC_POS: DWORD = 0;
pub const BASS_SYNC_END: DWORD = 2;
pub const BASS_SYNC_META: DWORD = 4;
pub const BASS_SYNC_SLIDE: DWORD = 5;
pub const BASS_SYNC_STALL: DWORD = 6;
pub const BASS_SYNC_DOWNLOAD: DWORD = 7;
pub const BASS_SYNC_FREE: DWORD = 8;
pub const BASS_SYNC_SETPOS: DWORD = 11;
pub const BASS_SYNC_OGG_CHANGE: DWORD = 12;
pub const BASS_SYNC_DEV_FAIL: DWORD = 14;
pub const BASS_SYNC_DEV_FORMAT: DWORD = 15;
/// Flag: call the sync on a dedicated BASS thread rather than the mixer.
pub const BASS_SYNC_THREAD: DWORD = 0x2000_0000;
/// Flag: call the sync on the mixer thread at mix time rather than when the
/// position is heard. This is what lets a loop seek land exactly on the
/// boundary with no audible gap.
pub const BASS_SYNC_MIXTIME: DWORD = 0x4000_0000;
pub const BASS_SYNC_ONETIME: DWORD = 0x8000_0000;

// --- Position modes ---------------------------------------------------------------

pub const BASS_POS_BYTE: DWORD = 0;
pub const BASS_POS_MUSIC_ORDER: DWORD = 1;
pub const BASS_POS_OGG: DWORD = 3;
pub const BASS_POS_END: DWORD = 0x10;
pub const BASS_POS_LOOP: DWORD = 0x11;
pub const BASS_POS_FLUSH: DWORD = 0x100_0000;
pub const BASS_POS_RESET: DWORD = 0x200_0000;
pub const BASS_POS_RELATIVE: DWORD = 0x400_0000;
pub const BASS_POS_INEXACT: DWORD = 0x800_0000;
pub const BASS_POS_DECODE: DWORD = 0x1000_0000;
pub const BASS_POS_DECODETO: DWORD = 0x2000_0000;
pub const BASS_POS_SCAN: DWORD = 0x4000_0000;

// --- Tag types (BASS_ChannelGetTags) ------------------------------------------

/// ID3v1 tags: `TAG_ID3` structure.
pub const BASS_TAG_ID3: DWORD = 0;
/// ID3v2 tags: variable-length block.
pub const BASS_TAG_ID3V2: DWORD = 1;
/// OGG comments: series of null-terminated UTF-8 strings.
pub const BASS_TAG_OGG: DWORD = 2;
/// HTTP headers: series of null-terminated ANSI strings.
pub const BASS_TAG_HTTP: DWORD = 3;
/// ICY headers: series of null-terminated ANSI strings.
pub const BASS_TAG_ICY: DWORD = 4;
/// ICY metadata (the `StreamTitle='...'` block): one ANSI string.
pub const BASS_TAG_META: DWORD = 5;
pub const BASS_TAG_APE: DWORD = 6;
pub const BASS_TAG_MP4: DWORD = 7;
pub const BASS_TAG_WMA: DWORD = 8;
pub const BASS_TAG_VENDOR: DWORD = 9;
pub const BASS_TAG_LYRICS3: DWORD = 10;
pub const BASS_TAG_WAVEFORMAT: DWORD = 14;
pub const BASS_TAG_RIFF_INFO: DWORD = 0x100;
/// BASSHLS: the current segment's `#EXTINF` line after the colon
/// (`duration,title`): one UTF-8 string.
pub const BASS_TAG_HLS_EXTINF: DWORD = 0x14000;

// --- Channel types (BASS_CHANNELINFO.ctype) -----------------------------------

pub const BASS_CTYPE_SAMPLE: DWORD = 1;
pub const BASS_CTYPE_RECORD: DWORD = 2;
pub const BASS_CTYPE_STREAM: DWORD = 0x10000;
pub const BASS_CTYPE_STREAM_VORBIS: DWORD = 0x10002;
pub const BASS_CTYPE_STREAM_OGG: DWORD = 0x10002;
pub const BASS_CTYPE_STREAM_MP1: DWORD = 0x10003;
pub const BASS_CTYPE_STREAM_MP2: DWORD = 0x10004;
pub const BASS_CTYPE_STREAM_MP3: DWORD = 0x10005;
pub const BASS_CTYPE_STREAM_AIFF: DWORD = 0x10006;
pub const BASS_CTYPE_STREAM_CA: DWORD = 0x10007;
pub const BASS_CTYPE_STREAM_MF: DWORD = 0x10008;
pub const BASS_CTYPE_STREAM_AM: DWORD = 0x10009;
pub const BASS_CTYPE_STREAM_SAMPLE: DWORD = 0x1000a;
pub const BASS_CTYPE_STREAM_DUMMY: DWORD = 0x18000;
pub const BASS_CTYPE_STREAM_DEVICE: DWORD = 0x18001;
/// WAVE is a flag on the channel type: `BASS_CTYPE_STREAM_WAV | codec`.
pub const BASS_CTYPE_STREAM_WAV: DWORD = 0x40000;
pub const BASS_CTYPE_STREAM_WAV_PCM: DWORD = 0x50001;
pub const BASS_CTYPE_STREAM_WAV_FLOAT: DWORD = 0x50003;
pub const BASS_CTYPE_STREAM_OPUS: DWORD = 0x11200;
pub const BASS_CTYPE_STREAM_FLAC: DWORD = 0x10900;
pub const BASS_CTYPE_STREAM_FLAC_OGG: DWORD = 0x10901;
pub const BASS_CTYPE_STREAM_AAC: DWORD = 0x10b00;
pub const BASS_CTYPE_STREAM_MP4: DWORD = 0x10b01;
pub const BASS_CTYPE_STREAM_HLS: DWORD = 0x10f00;

// --- Structs, exact C layout ---------------------------------------------------

/// Device information from `BASS_GetDeviceInfo`.
///
/// ```c
/// typedef struct {
///     const char *name;   // description
///     const char *driver; // driver
///     DWORD flags;
/// } BASS_DEVICEINFO;
/// ```
///
/// On 64-bit targets that is 24 bytes (two pointers, a DWORD and four bytes of
/// tail padding); the layout test below pins it. The strings are UTF-8 when
/// `BASS_CONFIG_UNICODE` is set, ANSI otherwise; they point into BASS's own
/// storage and are only valid until the next call.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BASS_DEVICEINFO {
    pub name: *const c_char,
    pub driver: *const c_char,
    pub flags: DWORD,
}

/// Channel information from `BASS_ChannelGetInfo`.
///
/// ```c
/// typedef struct {
///     DWORD freq;      // default playback rate
///     DWORD chans;     // channels
///     DWORD flags;     // BASS_SAMPLE/STREAM/MUSIC/SPEAKER flags
///     DWORD ctype;     // type of channel
///     DWORD origres;   // original resolution
///     HPLUGIN plugin;  // plugin
///     HSAMPLE sample;  // sample
///     const char *filename; // filename
/// } BASS_CHANNELINFO;
/// ```
///
/// Seven DWORDs, four bytes of padding, then a pointer: 40 bytes on 64-bit,
/// 32 on 32-bit. Pinned by the layout test below.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BASS_CHANNELINFO {
    pub freq: DWORD,
    pub chans: DWORD,
    pub flags: DWORD,
    pub ctype: DWORD,
    pub origres: DWORD,
    pub plugin: HPLUGIN,
    pub sample: HSAMPLE,
    pub filename: *const c_char,
}

/// A 3D position, velocity or orientation.
///
/// ```c
/// typedef struct BASS_3DVECTOR {
///     float x;  // +=right, -=left
///     float y;  // +=up, -=down
///     float z;  // +=front, -=behind
/// } BASS_3DVECTOR;
/// ```
///
/// Three floats, 12 bytes, no padding. BASS's axes are the ones spelled out
/// above: x right, y up, z forward -- a left-handed frame, so a caller with a
/// right-handed world negates z on the way in.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BASS_3DVECTOR {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl BASS_3DVECTOR {
    /// A vector from its three components.
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        BASS_3DVECTOR { x, y, z }
    }

    /// The origin, which is also what `Default` gives.
    pub const ZERO: BASS_3DVECTOR = BASS_3DVECTOR::new(0.0, 0.0, 0.0);
}

/// Parameters for the `bass_fx` biquad filter (`BASS_FX_BFX_BQF`).
///
/// ```c
/// typedef struct {
///     int   lFilter;     // BASS_BFX_BQF_xxx
///     float fCenter;     // cutoff/central frequency in Hz
///     float fGain;       // dB, shelving and peaking filters only
///     float fBandwidth;  // octaves; takes priority over fQ
///     float fQ;          // used when fBandwidth is 0
///     float fS;          // shelf slope, shelving filters only
///     int   lChannel;    // BASS_BFX_CHANxxx
/// } BASS_BFX_BQF;
/// ```
///
/// Seven 4-byte fields, 28 bytes; bass_fx.h's `#pragma pack(push, 4)` covers
/// only `BASS_BFX_ENV_NODE`, so this one has its natural layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BASS_BFX_BQF {
    /// `BASS_BFX_BQF_*` filter type.
    pub l_filter: c_int,
    /// Cutoff (central) frequency in Hz, `1 .. freq/2`.
    pub f_center: f32,
    /// Gain in dB; only PEAKINGEQ and the shelving filters use it.
    pub f_gain: f32,
    /// Bandwidth in octaves. Non-zero here means `f_q` is ignored.
    pub f_bandwidth: f32,
    /// Q, `0.1 ..= 1.0`, used when `f_bandwidth` is zero.
    pub f_q: f32,
    /// Shelf slope, `0.1 ..= 1.0`, shelving filters only.
    pub f_s: f32,
    /// `BASS_BFX_CHAN*` mask; `BASS_BFX_CHANALL` for the whole stream.
    pub l_channel: c_int,
}

// --- Callback types -----------------------------------------------------------------

/// `void CALLBACK SYNCPROC(HSYNC handle, DWORD channel, DWORD data, void *user)`.
///
/// Called by BASS on one of its own threads -- for `BASS_SYNC_MIXTIME` syncs
/// that is the mixer/update thread, while it holds the channel's lock. The
/// callback must be quick and must not block on anything the mixer thread
/// may itself be waiting for. `BASS_ChannelSetPosition` on the same channel
/// is explicitly allowed (it is what a seamless-loop position sync does).
pub type SYNCPROC =
    unsafe extern "system" fn(handle: HSYNC, channel: DWORD, data: DWORD, user: *mut c_void);

/// `void CALLBACK DOWNLOADPROC(const void *buffer, DWORD length, void *user)`.
///
/// The game passes NULL for this; the type exists so the `BASS_StreamCreateURL`
/// signature is exact.
pub type DOWNLOADPROC =
    unsafe extern "system" fn(buffer: *const c_void, length: DWORD, user: *mut c_void);

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

// --- Function pointer types, one per symbol the loader resolves -------------------

pub type FnInit = unsafe extern "system" fn(
    device: c_int,
    freq: DWORD,
    flags: DWORD,
    win: *mut c_void,
    clsid: *const c_void,
) -> BOOL;
pub type FnFree = unsafe extern "system" fn() -> BOOL;
pub type FnSetConfig = unsafe extern "system" fn(option: DWORD, value: DWORD) -> BOOL;
pub type FnSetConfigPtr = unsafe extern "system" fn(option: DWORD, value: *const c_void) -> BOOL;
pub type FnGetConfig = unsafe extern "system" fn(option: DWORD) -> DWORD;
pub type FnErrorGetCode = unsafe extern "system" fn() -> c_int;
pub type FnGetVersion = unsafe extern "system" fn() -> DWORD;
pub type FnGetDevice = unsafe extern "system" fn() -> DWORD;
pub type FnGetDeviceInfo =
    unsafe extern "system" fn(device: DWORD, info: *mut BASS_DEVICEINFO) -> BOOL;
pub type FnUpdate = unsafe extern "system" fn(length: DWORD) -> BOOL;

pub type FnPluginLoad = unsafe extern "system" fn(file: *const c_void, flags: DWORD) -> HPLUGIN;
pub type FnPluginFree = unsafe extern "system" fn(handle: HPLUGIN) -> BOOL;

pub type FnStreamCreateFile = unsafe extern "system" fn(
    mem: BOOL,
    file: *const c_void,
    offset: QWORD,
    length: QWORD,
    flags: DWORD,
) -> HSTREAM;
pub type FnStreamCreateURL = unsafe extern "system" fn(
    url: *const c_void,
    offset: DWORD,
    flags: DWORD,
    proc_: Option<DOWNLOADPROC>,
    user: *mut c_void,
) -> HSTREAM;
pub type FnStreamFree = unsafe extern "system" fn(handle: HSTREAM) -> BOOL;

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

pub type FnChannelPlay = unsafe extern "system" fn(handle: DWORD, restart: BOOL) -> BOOL;
pub type FnChannelPause = unsafe extern "system" fn(handle: DWORD) -> BOOL;
pub type FnChannelStop = unsafe extern "system" fn(handle: DWORD) -> BOOL;
pub type FnChannelIsActive = unsafe extern "system" fn(handle: DWORD) -> DWORD;
pub type FnChannelUpdate = unsafe extern "system" fn(handle: DWORD, length: DWORD) -> BOOL;
pub type FnChannelSetAttribute =
    unsafe extern "system" fn(handle: DWORD, attrib: DWORD, value: f32) -> BOOL;
pub type FnChannelGetAttribute =
    unsafe extern "system" fn(handle: DWORD, attrib: DWORD, value: *mut f32) -> BOOL;
pub type FnChannelSlideAttribute =
    unsafe extern "system" fn(handle: DWORD, attrib: DWORD, value: f32, time: DWORD) -> BOOL;
pub type FnChannelIsSliding = unsafe extern "system" fn(handle: DWORD, attrib: DWORD) -> BOOL;
pub type FnChannelGetLength = unsafe extern "system" fn(handle: DWORD, mode: DWORD) -> QWORD;
pub type FnChannelGetPosition = unsafe extern "system" fn(handle: DWORD, mode: DWORD) -> QWORD;
pub type FnChannelSetPosition =
    unsafe extern "system" fn(handle: DWORD, pos: QWORD, mode: DWORD) -> BOOL;
pub type FnChannelBytes2Seconds = unsafe extern "system" fn(handle: DWORD, pos: QWORD) -> f64;
pub type FnChannelSeconds2Bytes = unsafe extern "system" fn(handle: DWORD, pos: f64) -> QWORD;
pub type FnChannelSetSync = unsafe extern "system" fn(
    handle: DWORD,
    type_: DWORD,
    param: QWORD,
    proc_: Option<SYNCPROC>,
    user: *mut c_void,
) -> HSYNC;
pub type FnChannelRemoveSync = unsafe extern "system" fn(handle: DWORD, sync: HSYNC) -> BOOL;
pub type FnChannelGetTags = unsafe extern "system" fn(handle: DWORD, tags: DWORD) -> *const c_char;
pub type FnChannelFlags =
    unsafe extern "system" fn(handle: DWORD, flags: DWORD, mask: DWORD) -> DWORD;
pub type FnChannelGetInfo =
    unsafe extern "system" fn(handle: DWORD, info: *mut BASS_CHANNELINFO) -> BOOL;
pub type FnChannelGetDevice = unsafe extern "system" fn(handle: DWORD) -> DWORD;

pub type FnSet3DFactors = unsafe extern "system" fn(distf: f32, rollf: f32, doppf: f32) -> BOOL;
pub type FnSet3DPosition = unsafe extern "system" fn(
    pos: *const BASS_3DVECTOR,
    vel: *const BASS_3DVECTOR,
    front: *const BASS_3DVECTOR,
    top: *const BASS_3DVECTOR,
) -> BOOL;
pub type FnApply3D = unsafe extern "system" fn();
pub type FnChannelSet3DAttributes = unsafe extern "system" fn(
    handle: DWORD,
    mode: c_int,
    min: f32,
    max: f32,
    iangle: c_int,
    oangle: c_int,
    outvol: f32,
) -> BOOL;
pub type FnChannelSet3DPosition = unsafe extern "system" fn(
    handle: DWORD,
    pos: *const BASS_3DVECTOR,
    orient: *const BASS_3DVECTOR,
    vel: *const BASS_3DVECTOR,
) -> BOOL;
pub type FnChannelSetFX =
    unsafe extern "system" fn(handle: DWORD, type_: DWORD, priority: c_int) -> HFX;
pub type FnChannelRemoveFX = unsafe extern "system" fn(handle: DWORD, fx: HFX) -> BOOL;
pub type FnFXSetParameters = unsafe extern "system" fn(handle: HFX, params: *const c_void) -> BOOL;

/// Is the BASS library loadable on this machine?
///
/// Cheap after the first call: the loader caches its result, success or
/// failure, for the life of the process.
pub fn native_available() -> bool {
    Api::get().is_ok()
}

/// The resolved BASS entry points, or why they could not be resolved.
///
/// Same cached result as [`Api::get`]; this is the spelling the safe layer and
/// the game use.
pub fn api() -> Result<&'static Api, LoadError> {
    Api::get()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn channelinfo_layout_matches_bass_h() {
        let ptr = size_of::<*const c_char>();
        // Seven DWORDs (28 bytes) then the pointer, aligned to pointer size.
        let expected = if ptr == 8 { 40 } else { 32 };
        assert_eq!(size_of::<BASS_CHANNELINFO>(), expected);
        assert_eq!(align_of::<BASS_CHANNELINFO>(), ptr);
        assert_eq!(offset_of!(BASS_CHANNELINFO, freq), 0);
        assert_eq!(offset_of!(BASS_CHANNELINFO, chans), 4);
        assert_eq!(offset_of!(BASS_CHANNELINFO, flags), 8);
        assert_eq!(offset_of!(BASS_CHANNELINFO, ctype), 12);
        assert_eq!(offset_of!(BASS_CHANNELINFO, origres), 16);
        assert_eq!(offset_of!(BASS_CHANNELINFO, plugin), 20);
        assert_eq!(offset_of!(BASS_CHANNELINFO, sample), 24);
        assert_eq!(
            offset_of!(BASS_CHANNELINFO, filename),
            if ptr == 8 { 32 } else { 28 }
        );
    }

    #[test]
    fn deviceinfo_layout_matches_bass_h() {
        let ptr = size_of::<*const c_char>();
        let expected = if ptr == 8 { 24 } else { 12 };
        assert_eq!(size_of::<BASS_DEVICEINFO>(), expected);
        assert_eq!(offset_of!(BASS_DEVICEINFO, name), 0);
        assert_eq!(offset_of!(BASS_DEVICEINFO, driver), ptr);
        assert_eq!(offset_of!(BASS_DEVICEINFO, flags), 2 * ptr);
    }

    #[test]
    fn error_names_cover_every_declared_code() {
        for code in [
            BASS_OK,
            BASS_ERROR_MEM,
            BASS_ERROR_HANDLE,
            BASS_ERROR_INIT,
            BASS_ERROR_ALREADY,
            BASS_ERROR_NOTAVAIL,
            BASS_ERROR_TIMEOUT,
            BASS_ERROR_DENIED,
            BASS_ERROR_UNKNOWN,
        ] {
            assert!(error_name(code).is_some(), "code {code} has no name");
        }
        assert_eq!(error_name(12345), None);
        assert_eq!(error_name(BASS_ERROR_HANDLE), Some("BASS_ERROR_HANDLE"));
    }

    #[test]
    fn vector_and_bqf_layouts_match_the_headers() {
        // bass.h: three floats, no padding.
        assert_eq!(size_of::<BASS_3DVECTOR>(), 12);
        assert_eq!(align_of::<BASS_3DVECTOR>(), 4);
        assert_eq!(offset_of!(BASS_3DVECTOR, x), 0);
        assert_eq!(offset_of!(BASS_3DVECTOR, y), 4);
        assert_eq!(offset_of!(BASS_3DVECTOR, z), 8);
        assert_eq!(BASS_3DVECTOR::ZERO, BASS_3DVECTOR::default());

        // bass_fx.h: seven 4-byte fields in declaration order, unpacked --
        // its `#pragma pack(push, 4)` covers only BASS_BFX_ENV_NODE.
        assert_eq!(size_of::<BASS_BFX_BQF>(), 28);
        assert_eq!(align_of::<BASS_BFX_BQF>(), 4);
        assert_eq!(offset_of!(BASS_BFX_BQF, l_filter), 0);
        assert_eq!(offset_of!(BASS_BFX_BQF, f_center), 4);
        assert_eq!(offset_of!(BASS_BFX_BQF, f_gain), 8);
        assert_eq!(offset_of!(BASS_BFX_BQF, f_bandwidth), 12);
        assert_eq!(offset_of!(BASS_BFX_BQF, f_q), 16);
        assert_eq!(offset_of!(BASS_BFX_BQF, f_s), 20);
        assert_eq!(offset_of!(BASS_BFX_BQF, l_channel), 24);
    }

    #[test]
    fn three_d_and_bfx_constants_match_the_headers() {
        assert_eq!(BASS_SAMPLE_MONO, 2);
        assert_eq!(BASS_SAMPLE_3D, 8);
        assert_eq!(BASS_STREAM_AUTOFREE, 0x40000);
        assert_eq!(BASS_DEVICE_3D, 4);
        assert_eq!(BASS_3DMODE_NORMAL, 0);
        assert_eq!(BASS_3DMODE_RELATIVE, 1);
        assert_eq!(BASS_3DMODE_OFF, 2);
        assert_eq!(BASS_CONFIG_3DALGORITHM, 10);
        assert_eq!(BASS_3DALG_DEFAULT, 0);
        // The twentieth BFX type, counting from BASS_FX_BFX_ROTATE = 0x10000.
        assert_eq!(BASS_FX_BFX_BQF, 0x10013);
        assert_eq!(BASS_FX_BFX_BQF, 0x10000 + 19);
        // bass_fx.h's enum starts at zero, so LOWPASS is the zeroth filter.
        assert_eq!(BASS_BFX_BQF_LOWPASS, 0);
        assert_eq!(BASS_BFX_BQF_HIGHSHELF, 8);
        assert_eq!(BASS_BFX_CHANALL, -1);
    }

    #[test]
    fn flag_values_match_bass_h() {
        // Spot checks against the values bass.h documents.
        assert_eq!(BASS_STREAM_AUTOFREE, 0x40000);
        assert_eq!(BASS_SAMPLE_LOOP, 4);
        assert_eq!(BASS_UNICODE, 0x8000_0000);
        assert_eq!(BASS_SYNC_POS | BASS_SYNC_MIXTIME, 0x4000_0000);
        assert_eq!(BASS_CONFIG_NET_READTIMEOUT, 37);
        assert_eq!(BASS_TAG_META, 5);
        assert_eq!(BASS_ATTRIB_PAN, 3);
        // AUTOFREE doubles as MUSIC_AUTOFREE.
        assert_eq!(BASS_MUSIC_PRESCAN, 0x20000);
        assert_eq!(BASS_MUSIC_RAMPS, 0x400);
        assert_eq!(BASS_MUSIC_SINCINTER, 0x80_0000);
        assert_eq!(BASS_MUSIC_STOPBACK, 0x8_0000);
    }

    #[test]
    fn stream_callback_constants_match_bass_h() {
        assert_eq!(BASS_STREAMPROC_END, 0x8000_0000);
        assert_eq!(BASS_DATA_FLOAT, 0x4000_0000);
        assert_eq!(BASS_STREAM_DECODE, 0x20_0000);
        assert_eq!(BASS_SAMPLE_FLOAT, 256);
    }
}
