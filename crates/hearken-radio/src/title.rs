//! What a station says is playing, tidied the same way however it says it.

use std::borrow::Cow;

use icy_metadata::IcyMetadata;

/// The title in an ICY metadata block, read as UTF-8, or as Latin-1 when it
/// is not UTF-8: many Shoutcast stations send Latin-1.
pub(crate) fn icy(block: &[u8]) -> Option<String> {
    let text = match str::from_utf8(block) {
        Ok(text) => Cow::Borrowed(text),
        // ponytail: Latin-1, not Windows-1252; the 0x80-0x9F punctuation reads
        // as controls. Use encoding_rs if a station is heard doing that.
        Err(_) => Cow::Owned(block.iter().map(|&byte| char::from(byte)).collect()),
    };
    // A block is padded with zeros to a multiple of 16 bytes.
    let metadata: IcyMetadata = text.trim_end_matches('\0').parse().ok()?;
    tidy(metadata.stream_title()?)
}

/// A track's artist and title, joined; whichever is missing is left out.
pub(crate) fn ogg(artist: Option<&str>, title: Option<&str>) -> Option<String> {
    let parts: Vec<String> = [artist, title]
        .into_iter()
        .flatten()
        .filter_map(tidy)
        .collect();
    (!parts.is_empty()).then(|| parts.join(" - "))
}

/// An HLS `#EXTINF` title. Some encoders write "no desc" for none.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read once HLS playlists are parsed")
)]
pub(crate) fn hls(extinf_title: &str) -> Option<String> {
    tidy(extinf_title).filter(|title| !title.eq_ignore_ascii_case("no desc"))
}

/// `title` trimmed, with each run of whitespace made one space; `None` if
/// nothing is left.
pub(crate) fn tidy(title: &str) -> Option<String> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icy_reads_the_stream_title() {
        let block = b"StreamTitle='Band - Song';StreamUrl='';";
        assert_eq!(icy(block).as_deref(), Some("Band - Song"));
    }

    #[test]
    fn an_empty_icy_title_is_none() {
        assert_eq!(icy(b"StreamTitle='';"), None);
    }

    #[test]
    fn latin1_icy_bytes_read_as_latin1() {
        let block = b"StreamTitle='Caf\xe9 del Mar';";
        assert_eq!(icy(block).as_deref(), Some("Café del Mar"));
    }

    #[test]
    fn icy_ignores_the_zero_padding_of_a_block() {
        let mut block = b"StreamTitle='Band - Song';".to_vec();
        block.resize(32, 0);
        assert_eq!(icy(&block).as_deref(), Some("Band - Song"));
    }

    #[test]
    fn ogg_joins_artist_and_title() {
        assert_eq!(
            ogg(Some("Band"), Some("Song")).as_deref(),
            Some("Band - Song")
        );
        assert_eq!(ogg(None, Some("Song")).as_deref(), Some("Song"));
        assert_eq!(ogg(Some("  "), None), None);
    }

    #[test]
    fn titles_are_trimmed_and_their_whitespace_collapsed() {
        assert_eq!(
            ogg(Some(" Band\t"), Some("Two  \n Words")).as_deref(),
            Some("Band - Two Words")
        );
    }

    #[test]
    fn hls_drops_no_desc() {
        assert_eq!(hls("no desc"), None);
        assert_eq!(hls("  "), None);
        assert_eq!(hls(" Band - Song ").as_deref(), Some("Band - Song"));
    }
}
