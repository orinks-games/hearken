use super::*;

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
    let bytes = std::mem::size_of_val(out) as DWORD;
    // SAFETY: `out` is a writable buffer of exactly `bytes` bytes.
    let got =
        unsafe { (a.channel_get_data)(handle, out.as_mut_ptr().cast(), bytes | BASS_DATA_FLOAT) };
    if got == u32::MAX {
        Err(BassError::last())
    } else {
        Ok(got as usize / std::mem::size_of::<f32>())
    }
}
