use super::*;

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
