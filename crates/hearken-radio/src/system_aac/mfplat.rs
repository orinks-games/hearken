//! The few `mfplat.dll` functions the decoder calls, found at run time.
//!
//! Linking them the usual way puts `mfplat.dll` in the program's import
//! table, and Windows then refuses to start the program at all where the
//! library is missing: the N and KN editions without the Media Feature Pack.
//! Loaded here instead, a missing Media Foundation is just an error from
//! `MFStartup`, and the radio falls back to Symphonia's AAC decoder.
//!
//! Everything else the decoder uses is a COM interface (called through its
//! vtable, no import) or a constant, so these six are all it needs.

#![allow(non_snake_case)] // Named as Microsoft names them, so mf.rs reads the same.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{s, w, Interface, GUID, HRESULT, PCSTR};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaBuffer, IMFMediaType, IMFSample, MFT_ENUM_FLAG, MFT_REGISTER_TYPE_INFO,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

/// What `GetProcAddress` returns, before the cast to the real signature.
type Proc = unsafe extern "system" fn() -> isize;
type Startup = unsafe extern "system" fn(u32, u32) -> HRESULT;
type Shutdown = unsafe extern "system" fn() -> HRESULT;
type CreateOut = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateBuffer = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;
type TEnumEx = unsafe extern "system" fn(
    GUID,
    u32,
    *const MFT_REGISTER_TYPE_INFO,
    *const MFT_REGISTER_TYPE_INFO,
    *mut *mut *mut c_void,
    *mut u32,
) -> HRESULT;

struct Api {
    startup: Startup,
    shutdown: Shutdown,
    create_media_type: CreateOut,
    create_memory_buffer: CreateBuffer,
    create_sample: CreateOut,
    t_enum_ex: TEnumEx,
}

/// The table, loaded on first use; `Err` says why it could not be.
fn api() -> windows::core::Result<&'static Api> {
    static API: OnceLock<Result<Api, windows::core::Error>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(Clone::clone)
}

fn load() -> windows::core::Result<Api> {
    // SAFETY: System32 only, so no copy beside the game is picked up. The
    // module is never freed, so the pointers stay valid for the process.
    // Each pointer is cast to the signature Microsoft documents for it.
    unsafe {
        let module = LoadLibraryExW(w!("mfplat.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
        let find = |name: PCSTR| {
            GetProcAddress(module, name).ok_or_else(windows::core::Error::from_thread)
        };
        Ok(Api {
            startup: std::mem::transmute::<Proc, Startup>(find(s!("MFStartup"))?),
            shutdown: std::mem::transmute::<Proc, Shutdown>(find(s!("MFShutdown"))?),
            create_media_type: std::mem::transmute::<Proc, CreateOut>(find(s!(
                "MFCreateMediaType"
            ))?),
            create_memory_buffer: std::mem::transmute::<Proc, CreateBuffer>(find(s!(
                "MFCreateMemoryBuffer"
            ))?),
            create_sample: std::mem::transmute::<Proc, CreateOut>(find(s!("MFCreateSample"))?),
            t_enum_ex: std::mem::transmute::<Proc, TEnumEx>(find(s!("MFTEnumEx"))?),
        })
    }
}

/// An interface a creation call handed back through `out`.
///
/// # Safety
/// `out` must be what a successful call wrote for an object of type `T`.
unsafe fn take<T: Interface>(hr: HRESULT, out: *mut c_void) -> windows::core::Result<T> {
    hr.ok()?;
    if out.is_null() {
        return Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_POINTER,
        ));
    }
    // SAFETY: per the caller, `out` holds one reference to a `T`.
    Ok(unsafe { T::from_raw(out) })
}

/// `MFStartup`, failing (rather than the program) without Media Foundation.
pub(super) unsafe fn MFStartup(version: u32, flags: u32) -> windows::core::Result<()> {
    // SAFETY: plain arguments.
    unsafe { (api()?.startup)(version, flags).ok() }
}

pub(super) unsafe fn MFShutdown() -> windows::core::Result<()> {
    // SAFETY: no arguments; the caller balances an MFStartup.
    unsafe { (api()?.shutdown)().ok() }
}

pub(super) unsafe fn MFCreateMediaType() -> windows::core::Result<IMFMediaType> {
    let mut out = std::ptr::null_mut();
    // SAFETY: `out` receives one media type on success.
    unsafe { take((api()?.create_media_type)(&mut out), out) }
}

pub(super) unsafe fn MFCreateMemoryBuffer(bytes: u32) -> windows::core::Result<IMFMediaBuffer> {
    let mut out = std::ptr::null_mut();
    // SAFETY: `out` receives one buffer on success.
    unsafe { take((api()?.create_memory_buffer)(bytes, &mut out), out) }
}

pub(super) unsafe fn MFCreateSample() -> windows::core::Result<IMFSample> {
    let mut out = std::ptr::null_mut();
    // SAFETY: `out` receives one sample on success.
    unsafe { take((api()?.create_sample)(&mut out), out) }
}

pub(super) unsafe fn MFTEnumEx(
    category: GUID,
    flags: MFT_ENUM_FLAG,
    input: Option<*const MFT_REGISTER_TYPE_INFO>,
    output: Option<*const MFT_REGISTER_TYPE_INFO>,
    list: *mut *mut Option<IMFActivate>,
    count: *mut u32,
) -> windows::core::Result<()> {
    // SAFETY: the caller's pointers, passed as the windows crate's own
    // wrapper passes them (`Option<IMFActivate>` is one interface pointer).
    unsafe {
        (api()?.t_enum_ex)(
            category,
            flags.0 as u32,
            input.unwrap_or(std::ptr::null()),
            output.unwrap_or(std::ptr::null()),
            list.cast(),
            count,
        )
        .ok()
    }
}

#[cfg(test)]
mod tests {
    /// The libraries the running test binary imports, from its PE header.
    fn imports() -> Vec<String> {
        let bytes = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let pe = u32_at(0x3c);
        let sections = u16_at(pe + 6);
        let optional = pe + 24;
        let magic = u16_at(optional);
        // The import directory's RVA: data directory 1.
        let dirs = optional + if magic == 0x20b { 112 } else { 96 };
        let import_rva = u32_at(dirs + 8);
        let table = optional + u16_at(pe + 20);
        let to_file = |rva: usize| {
            (0..sections)
                .map(|i| table + i * 40)
                .find_map(|s| {
                    let (va, size, raw) = (
                        u32_at(s + 12),
                        u32_at(s + 8).max(u32_at(s + 16)),
                        u32_at(s + 20),
                    );
                    (va..va + size).contains(&rva).then(|| rva - va + raw)
                })
                .unwrap()
        };
        let mut names = Vec::new();
        let mut entry = to_file(import_rva);
        while u32_at(entry + 12) != 0 {
            let name = to_file(u32_at(entry + 12));
            let end = bytes[name..].iter().position(|&b| b == 0).unwrap();
            names.push(String::from_utf8_lossy(&bytes[name..name + end]).to_ascii_lowercase());
            entry += 20;
        }
        names
    }

    #[test]
    fn mfplat_is_not_a_load_time_import() {
        let names = imports();
        assert!(
            names.iter().any(|n| n == "kernel32.dll"),
            "parsed {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "mfplat.dll"),
            "imports {names:?}"
        );
    }

    #[test]
    fn mfplat_loads_where_media_foundation_is() {
        super::api().expect("Media Foundation on the test machine");
    }
}
