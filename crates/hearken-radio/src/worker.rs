//! The thread behind a [`Stream`](crate::Stream): connect, decode, report.

use std::io::{Cursor, Read};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::time::Duration;

use icy_metadata::IcyMetadata;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::ReadOnlySource;

use crate::icy::IcyReader;
use crate::playlist::Kind;
use crate::stream::{Fault, RawTitle, Shared, StreamOptions, StreamState};
use crate::{decode, hls, http, playlist, title};

/// Plays `url` until it fails or every handle is dropped, calling it again
/// after each drop while the retries last. A station that has never played
/// gets one retry, not the whole schedule, so a dead one is reported in
/// seconds rather than a minute and more. A panic in here is a bug,
/// reported as `Failed` rather than taking the game down.
pub(crate) fn run(url: &str, shared: &Arc<Shared>, opts: &StreamOptions) {
    let mut attempt = 0;
    let reason = loop {
        shared.calling();
        let fault = panic::catch_unwind(AssertUnwindSafe(|| play(url, shared, opts)))
            .unwrap_or_else(|_| Fault::Fatal("internal error".into()));
        // Nobody is left to hear a retry or be told of a failure. A closed
        // body's read fails as a Retry, so this comes first.
        if shared.closed() {
            return;
        }
        // Until it first plays, a station is still connecting.
        let connecting = shared.state() == StreamState::Connecting;
        let reason = match fault {
            Fault::Fatal(reason) => break reason,
            Fault::Unreachable(reason) if connecting => break reason,
            Fault::Unreachable(reason) | Fault::Retry(reason) => reason,
        };
        // A call that never played reports zero, which is not good playback
        // even at a `healthy_after` of zero.
        let played = shared.played();
        if played > Duration::ZERO && played >= opts.healthy_after {
            attempt = 0;
        }
        let delay = opts.retry_delays.get(attempt);
        let Some(&delay) = delay.filter(|_| !connecting || attempt == 0) else {
            break reason;
        };
        attempt += 1;
        let secs = delay.as_secs_f32();
        log::info!("radio: {url} lost ({reason}); retry {attempt} in {secs} s");
        if !connecting {
            shared.set_state(StreamState::Reconnecting);
        }
        if !shared.sleep(delay) {
            return;
        }
    };
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
    let response = http::open(url, shared, opts)?;
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
    let body = IcyReader::new(body, icy_metaint, move |block| {
        titles.set_raw_title(RawTitle::Icy(block.to_vec()));
        let current = match str::from_utf8(block) {
            Ok(text) => match text.parse::<IcyMetadata>() {
                Ok(metadata) => metadata.stream_title().and_then(title::tidy),
                // Nothing in it to read; the last title stands.
                Err(_) => return,
            },
            Err(_) => title::icy(block),
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
