//! A station that drops is called again after a wait; one that keeps
//! failing is reported failed.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hearken_radio::{Stream, StreamState};
use support::{failed, fast, listen, playing, wait_for, Reply, TestServer};

/// The tone from `tests/http.rs`; see there for how it is made.
const TONE: &[u8] = include_bytes!("fixtures/tone.mp3");

/// A station answering its first `n` calls with `first`, and every call
/// after with an endless stream.
fn station_after(n: usize, first: impl Fn() -> Reply + Send + Sync + 'static) -> TestServer {
    let calls = AtomicUsize::new(0);
    TestServer::start(move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) < n {
            first()
        } else {
            Reply::Endless("audio/mpeg", TONE.to_vec(), None)
        }
    })
}

/// The states `stream` passes through, in order, polled every millisecond
/// (a retry's wait is 50 ms) until `done` holds of them; panics after
/// `within`.
fn states(
    stream: &Stream,
    within: Duration,
    done: impl Fn(&[StreamState]) -> bool,
) -> Vec<StreamState> {
    let deadline = Instant::now() + within;
    let mut seen = vec![stream.state()];
    while !done(&seen) {
        assert!(Instant::now() < deadline, "{seen:?}: not within {within:?}");
        thread::sleep(Duration::from_millis(1));
        let state = stream.state();
        if seen.last() != Some(&state) {
            seen.push(state);
        }
    }
    seen
}

fn has_failed(seen: &[StreamState]) -> bool {
    matches!(seen.last(), Some(StreamState::Failed(_)))
}

#[test]
fn a_dropped_stream_reconnects_and_plays_again() {
    // Two seconds at the server's pace, then it hangs up.
    let server = station_after(1, || Reply::Cut("audio/mpeg", TONE.to_vec(), 64 * 1024));
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    let _listener = listen(&stream);
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    let seen = states(&stream, Duration::from_secs(10), |seen| {
        seen.len() == 3 || has_failed(seen)
    });
    use StreamState::{Playing, Reconnecting};
    assert_eq!(seen, [Playing, Reconnecting, Playing]);
}

#[test]
fn a_server_error_is_retried_then_reported() {
    let server = TestServer::start(|_| Reply::Body(503, "text/plain", b"busy".to_vec()));
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    wait_for("Failed", Duration::from_secs(3), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("HTTP 503"));
    assert_eq!(server.hits("/live.mp3"), 1 + fast().retry_delays.len());
}

#[test]
fn a_server_that_never_answers_is_retried() {
    let server = TestServer::start(|_| Reply::Hang);
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    // Three calls, each given up after the 1 s timeout.
    wait_for("Failed", Duration::from_secs(6), || {
        failed(&stream).is_some()
    });
    let reason = failed(&stream).unwrap();
    assert!(reason.contains("timed out"), "{reason}");
    assert_eq!(server.hits("/live.mp3"), 1 + fast().retry_delays.len());
}

#[test]
fn good_playback_resets_the_retry_count() {
    // About 1.5 s at the server's pace, past `healthy_after` (1 s), then
    // it hangs up, every time.
    let server = TestServer::start(|_| Reply::Cut("audio/mpeg", TONE.to_vec(), 48 * 1024));
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    let _listener = listen(&stream);
    // Two retries allow three calls; healthy calls earn them back.
    wait_for("5 calls", Duration::from_secs(15), || {
        server.hits("/live.mp3") >= 5 || failed(&stream).is_some()
    });
    assert_eq!(failed(&stream), None);
}

#[test]
fn the_first_connects_retries_stay_connecting() {
    let server = station_after(2, || Reply::Body(503, "text/plain", b"busy".to_vec()));
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    let _listener = listen(&stream);
    let seen = states(&stream, Duration::from_secs(5), |seen| {
        seen.last() == Some(&StreamState::Playing) || has_failed(seen)
    });
    assert_eq!(seen, [StreamState::Connecting, StreamState::Playing]);
    assert_eq!(server.hits("/live.mp3"), 3);
}

#[test]
fn a_redirect_loop_fails_at_once() {
    let server = TestServer::start(|path| match path {
        "/a" => Reply::Redirect(302, "/b".into()),
        _ => Reply::Redirect(302, "/a".into()),
    });
    let stream = Stream::open_with(&server.url("/a"), fast());
    wait_for("Failed", Duration::from_secs(3), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("too many redirects"));
    // One call follows 10 redirects at most: 5 of them to /a.
    let hits = server.hits("/a");
    assert!(hits <= 6, "{hits} requests for /a");
}
