//! What a station's reply is: audio to decode, a playlist of stations, or
//! live HLS; and the first station a playlist names.

use url::Url;

/// What a reply carries.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Audio to decode, with the file extension that names its format.
    Audio(Option<&'static str>),
    /// A PLS or M3U list of stations.
    Playlist,
    /// A live HLS playlist of segments.
    Hls,
}

/// Decides what a reply carries. A Content-Type that names a format is
/// trusted, but plenty of stations send audio as `application/octet-stream`
/// and playlists as `text/plain`, so anything else is sniffed from the
/// first bytes, then from the URL.
pub(crate) fn kind(content_type: Option<&str>, url: &str, head: &[u8]) -> Kind {
    // M3U also holds HLS, which its tags tell apart.
    let m3u = || {
        if head.windows(7).any(|w| w == b"#EXT-X-") {
            Kind::Hls
        } else {
            Kind::Playlist
        }
    };
    let essence = content_type.map(|t| {
        let essence = t.split(';').next().unwrap_or_default();
        essence.trim().to_ascii_lowercase()
    });
    match essence.as_deref() {
        Some("audio/x-scpls") => return Kind::Playlist,
        Some(
            "audio/x-mpegurl"
            | "audio/mpegurl"
            | "application/x-mpegurl"
            | "application/vnd.apple.mpegurl",
        ) => return m3u(),
        Some("audio/mpeg") => return Kind::Audio(Some("mp3")),
        Some("audio/aac" | "audio/aacp" | "audio/x-aac") => return Kind::Audio(Some("aac")),
        Some("application/ogg" | "audio/ogg" | "audio/opus") => return Kind::Audio(Some("ogg")),
        _ => {}
    }
    let text = head.strip_prefix(BOM).unwrap_or(head).trim_ascii_start();
    if starts_with_ignore_case(text, b"[playlist]") {
        Kind::Playlist
    } else if text.starts_with(b"#EXTM3U") {
        m3u()
    } else if starts_with_ignore_case(text, b"http") {
        Kind::Playlist
    } else {
        match extension(url).as_deref() {
            Some("pls") => Kind::Playlist,
            Some("m3u" | "m3u8") => m3u(),
            _ => Kind::Audio(None),
        }
    }
}

/// The first station a PLS or M3U playlist names, as an absolute URL:
/// entries are often relative to the playlist.
pub(crate) fn first_stream(text: &str, base: &Url) -> Option<String> {
    let text = text.trim_start_matches('\u{feff}');
    let entry = if starts_with_ignore_case(text.trim_start().as_bytes(), b"[playlist]") {
        text.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            let file = key.trim().to_ascii_lowercase().starts_with("file");
            file.then_some(value.trim())
                .filter(|value| !value.is_empty())
        })
    } else {
        text.lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
    };
    base.join(entry?).ok().map(String::from)
}

/// A UTF-8 byte order mark, which Windows editors put on playlists.
const BOM: &[u8] = b"\xef\xbb\xbf";

fn starts_with_ignore_case(bytes: &[u8], prefix: &[u8]) -> bool {
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// The lowercase extension of the URL's last path segment, if it has one.
fn extension(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let (_, extension) = url.path_segments()?.next_back()?.rsplit_once('.')?;
    Some(extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("http://h/radio/list.m3u").unwrap()
    }

    #[test]
    fn pls_gives_its_first_file() {
        let text = "[playlist]\nNumberOfEntries=2\nFile1=http://a/one.mp3\nTitle1=One\n\
                    Length1=-1\nFile2=http://a/two.mp3\n";
        assert_eq!(
            first_stream(text, &base()).as_deref(),
            Some("http://a/one.mp3")
        );
    }

    #[test]
    fn m3u_skips_comments_and_blank_lines() {
        let text = "\u{feff}#EXTM3U\n\n# a comment\n#EXTINF:-1,One\n  http://a/one.mp3  \nhttp://a/two.mp3\n";
        assert_eq!(
            first_stream(text, &base()).as_deref(),
            Some("http://a/one.mp3")
        );
        assert_eq!(first_stream("#EXTM3U\n# nothing\n", &base()), None);
    }

    #[test]
    fn a_relative_entry_resolves_against_the_playlist() {
        assert_eq!(
            first_stream("live.mp3\n", &base()).as_deref(),
            Some("http://h/radio/live.mp3")
        );
        assert_eq!(
            first_stream("[playlist]\nFile1=/live.mp3\n", &base()).as_deref(),
            Some("http://h/live.mp3")
        );
    }

    #[test]
    fn kind_from_content_type_ignores_case_and_parameters() {
        let kind = |content_type| kind(Some(content_type), "http://h/live", b"");
        assert_eq!(kind("Audio/MPEG; charset=x"), Kind::Audio(Some("mp3")));
        assert_eq!(kind("audio/aacp"), Kind::Audio(Some("aac")));
        assert_eq!(kind("application/ogg"), Kind::Audio(Some("ogg")));
        assert_eq!(kind("AUDIO/X-SCPLS"), Kind::Playlist);
        assert_eq!(kind("audio/x-mpegurl"), Kind::Playlist);
    }

    #[test]
    fn kind_sniffs_a_playlist_served_as_text_plain() {
        let kind = |url, head: &[u8]| kind(Some("text/plain"), url, head);
        assert_eq!(kind("http://h/a", b"[playlist]\nFile1=x"), Kind::Playlist);
        assert_eq!(kind("http://h/a", b"#EXTM3U\nhttp://x"), Kind::Playlist);
        assert_eq!(kind("http://h/a", b"http://x/live.mp3\n"), Kind::Playlist);
        assert_eq!(kind("http://h/a.pls?x=1", b"\xff\xfb\x90"), Kind::Playlist);
        assert_eq!(kind("http://h/a", b"\xff\xfb\x90"), Kind::Audio(None));
        // No Content-Type at all is sniffed the same way.
        assert_eq!(
            super::kind(None, "http://h/a", b"[playlist]"),
            Kind::Playlist
        );
    }

    #[test]
    fn kind_sniffs_hls_by_its_tags() {
        let live = b"#EXTM3U\n#EXT-X-TARGETDURATION:6\n";
        assert_eq!(kind(None, "http://h/a", live), Kind::Hls);
        assert_eq!(kind(Some("text/plain"), "http://h/a.m3u8", live), Kind::Hls);
        assert_eq!(
            kind(Some("application/vnd.apple.mpegurl"), "http://h/a", live),
            Kind::Hls
        );
        assert_eq!(
            kind(Some("audio/x-mpegurl"), "http://h/a", b"#EXTM3U\nhttp://x"),
            Kind::Playlist
        );
    }
}
