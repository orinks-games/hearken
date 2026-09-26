use super::*;

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
