use super::*;

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
