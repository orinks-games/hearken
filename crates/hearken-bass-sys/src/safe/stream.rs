use super::*;

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
