//! The thread behind a [`Stream`](crate::Stream): connect, decode, report.

use std::io::{Cursor, Read};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use icy_metadata::error::MetadataParseError;
use icy_metadata::IcyMetadataReader;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::ReadOnlySource;

use crate::playlist::Kind;
use crate::stream::{Fault, Shared, StreamOptions, StreamState};
use crate::{decode, hls, http, playlist, title};

/// Plays `url` until it fails or every handle is dropped. A panic in here
/// is a bug, reported as `Failed` rather than taking the game down.
pub(crate) fn run(url: &str, shared: &Arc<Shared>, opts: &StreamOptions) {
    let fault = panic::catch_unwind(AssertUnwindSafe(|| play(url, shared, opts)))
        .unwrap_or_else(|_| Fault::Fatal("internal error".into()));
    let (Fault::Fatal(reason) | Fault::Retry(reason)) = fault;
    shared.set_state(StreamState::Failed(reason));
}

/// Playlists a station may lead through, counted from its own URL.
const MAX_HOPS: usize = 3;

/// How much of a reply to read before deciding what it is.
const HEAD: u64 = 1024;

/// The most of a playlist worth reading; past it, it is not one.
pub(crate) const MAX_PLAYLIST: u64 = 1 << 20;

/// Plays `url`, following playlists to the station they name.
fn play(url: &str, shared: &Arc<Shared>, opts: &StreamOptions) -> Fault {
    let mut url = url.to_owned();
    for _ in 0..=MAX_HOPS {
        match open_station(&url, shared, opts) {
            Ok(next) => url = next,
            Err(fault) => return fault,
        }
    }
    Fault::Fatal("too many playlist hops".into())
}

/// One connection, from the request to why it ended. A playlist ends it at
/// once with the URL to try next, as `Ok`.
fn open_station(url: &str, shared: &Arc<Shared>, opts: &StreamOptions) -> Result<String, Fault> {
    let response = http::open(url, opts)?;
    let mut body = response.body;
    // Kept, so the decoder starts from the first byte.
    let head = read_up_to(&mut body, HEAD)?;
    let kind = playlist::kind(
        response.content_type.as_deref(),
        response.url.as_str(),
        &head,
    );
    let mut body = Cursor::new(head).chain(body);
    if let Kind::Audio(extension) = kind {
        return Err(listen(body, response.icy_metaint, extension, shared, opts));
    }
    let text = read_up_to(&mut body, MAX_PLAYLIST)?;
    let text = String::from_utf8_lossy(&text);
    if kind == Kind::Hls {
        return Err(hls::play(response.url.as_str(), &text, shared, opts));
    }
    playlist::first_stream(&text, &response.url)
        .ok_or_else(|| Fault::Fatal("empty playlist".into()))
}

/// At most `limit` bytes of `body`, fewer if it ends first.
pub(crate) fn read_up_to(body: &mut impl Read, limit: u64) -> Result<Vec<u8>, Fault> {
    let mut bytes = Vec::new();
    body.take(limit).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Decodes `body` as audio until it ends.
fn listen(
    body: impl Read + Send + Sync + 'static,
    icy_metaint: Option<NonZeroUsize>,
    extension: Option<&str>,
    shared: &Arc<Shared>,
    opts: &StreamOptions,
) -> Fault {
    // Title blocks come out of the body before the decoder reads it.
    let titles = shared.clone();
    let body = IcyMetadataReader::new(body, icy_metaint, move |block| {
        let current = match block {
            Ok(metadata) => metadata.stream_title().and_then(title::tidy),
            Err(MetadataParseError::InvalidUtf8(error)) => title::icy(&error.into_bytes()),
            Err(MetadataParseError::Empty(_)) => return,
        };
        titles.set_title(current);
    });
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }
    let source = Box::new(ReadOnlySource::new(body));
    decode::pump(source, hint, shared, opts)
}
