//! The thread behind a [`Stream`](crate::Stream): connect, decode, report.

use std::panic::{self, AssertUnwindSafe};

use symphonia::core::formats::probe::Hint;
use symphonia::core::io::ReadOnlySource;

use crate::stream::{Fault, Shared, StreamOptions, StreamState};
use crate::{decode, http};

/// Plays `url` until it fails or every handle is dropped. A panic in here
/// is a bug, reported as `Failed` rather than taking the game down.
pub(crate) fn run(url: &str, shared: &Shared, opts: &StreamOptions) {
    let fault = panic::catch_unwind(AssertUnwindSafe(|| play(url, shared, opts)))
        .unwrap_or_else(|_| Fault::Fatal("internal error".into()));
    let (Fault::Fatal(reason) | Fault::Retry(reason)) = fault;
    shared.set_state(StreamState::Failed(reason));
}

/// One connection, from the request to why it ended.
fn play(url: &str, shared: &Shared, opts: &StreamOptions) -> Fault {
    let response = match http::open(url, opts) {
        Ok(response) => response,
        Err(fault) => return fault,
    };
    let source = Box::new(ReadOnlySource::new(response.body));
    decode::pump(source, Hint::new(), shared, opts)
}
