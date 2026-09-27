//! One running copy of a game per Windows session. Everything fails open: a
//! mutex the OS will not create never keeps a player out.

pub const ERROR_ALREADY_EXISTS: u32 = 183;

/// The two kernel32 calls the guard makes, behind a trait so a test
/// suite's fake has a Rust shape. Handles are plain integers.
pub trait MutexApi {
    /// `CreateMutexW(None, False, name)`: the handle (0 when the call
    /// failed) and the thread's last error right after the call.
    fn create_mutex(&self, name: &str) -> (usize, u32);
    fn close_handle(&self, handle: usize);
}

/// The real kernel32 on Windows; never constructed elsewhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinMutexApi;

#[cfg(windows)]
impl MutexApi for WinMutexApi {
    fn create_mutex(&self, name: &str) -> (usize, u32) {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the name is a valid NUL-terminated UTF-16 buffer that
        // outlives the call; a null security descriptor and no initial
        // ownership are the documented defaults.
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
            let err = GetLastError();
            (handle as usize, err)
        }
    }

    fn close_handle(&self, handle: usize) {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        // SAFETY: the handle came from CreateMutexW above and is closed once.
        unsafe {
            CloseHandle(handle as HANDLE);
        }
    }
}

#[cfg(not(windows))]
impl MutexApi for WinMutexApi {
    fn create_mutex(&self, _name: &str) -> (usize, u32) {
        (0, 0)
    }

    fn close_handle(&self, _handle: usize) {}
}

/// Coordinates one running copy of a game per Windows session.
pub struct SingleInstanceGuard<A: MutexApi = WinMutexApi> {
    pub mutex_name: String,
    api: A,
    windows: bool,
    mutex_handle: Option<usize>,
    acquired: bool,
}

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

impl<A: MutexApi> SingleInstanceGuard<A> {
    /// A guard over an injected API. `windows` is whether the guard should
    /// actually touch the API: off it, `acquire` always succeeds without
    /// touching the API.
    pub fn with_api(mutex_name: &str, api: A, windows: bool) -> Self {
        Self {
            mutex_name: mutex_name.to_string(),
            api,
            windows,
            mutex_handle: None,
            acquired: false,
        }
    }

    pub fn acquire(&mut self) -> bool {
        if !self.windows {
            self.acquired = true;
            return true;
        }
        let (handle, last_error) = self.api.create_mutex(&self.mutex_name);
        if handle == 0 {
            log::warn!("CreateMutexW failed; allowing startup to continue");
            return true;
        }
        if last_error == ERROR_ALREADY_EXISTS {
            self.api.close_handle(handle);
            self.mutex_handle = None;
            self.acquired = false;
            return false;
        }
        self.mutex_handle = Some(handle);
        self.acquired = true;
        true
    }

    pub fn release(&mut self) {
        let Some(handle) = self.mutex_handle.take() else {
            self.acquired = false;
            return;
        };
        self.api.close_handle(handle);
        self.acquired = false;
    }

    pub fn acquired(&self) -> bool {
        self.acquired
    }
}

impl<A: MutexApi> Drop for SingleInstanceGuard<A> {
    fn drop(&mut self) {
        self.release();
    }
}
