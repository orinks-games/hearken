use super::*;

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
