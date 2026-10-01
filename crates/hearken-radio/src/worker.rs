//! The thread behind a [`Stream`](crate::Stream): connect, decode, report.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use icy_metadata::error::MetadataParseError;
use icy_metadata::IcyMetadataReader;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::ReadOnlySource;

use crate::stream::{Fault, Shared, StreamOptions, StreamState};
use crate::{decode, http, title};

/// Plays `url` until it fails or every handle is dropped. A panic in here
/// is a bug, reported as `Failed` rather than taking the game down.
pub(crate) fn run(url: &str, shared: &Arc<Shared>, opts: &StreamOptions) {
    let fault = panic::catch_unwind(AssertUnwindSafe(|| play(url, shared, opts)))
        .unwrap_or_else(|_| Fault::Fatal("internal error".into()));
    let (Fault::Fatal(reason) | Fault::Retry(reason)) = fault;
    shared.set_state(StreamState::Failed(reason));
}

/// One connection, from the request to why it ended.
fn play(url: &str, shared: &Arc<Shared>, opts: &StreamOptions) -> Fault {
    let response = match http::open(url, opts) {
        Ok(response) => response,
        Err(fault) => return fault,
    };
    // Title blocks come out of the body before anything else reads it.
    let titles = shared.clone();
    let body = IcyMetadataReader::new(response.body, response.icy_metaint, move |block| {
        let current = match block {
            Ok(metadata) => metadata.stream_title().and_then(title::tidy),
            Err(MetadataParseError::InvalidUtf8(error)) => title::icy(&error.into_bytes()),
            Err(MetadataParseError::Empty(_)) => return,
        };
        titles.set_title(current);
    });
    let source = Box::new(ReadOnlySource::new(body));
    decode::pump(source, Hint::new(), shared, opts)
}
