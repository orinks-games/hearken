//! A station that drops is called again after a wait; one that keeps
//! failing is reported failed, and one that never played, sooner.

mod support;

use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hearken_radio::{Stream, StreamOptions, StreamState};
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
/// until `done` holds of them; panics after `within`.
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
    // A wait long enough that polling cannot miss `Reconnecting` on a busy
    // machine.
    let options = StreamOptions {
        retry_delays: vec![Duration::from_millis(500); 2],
        ..fast()
    };
    let stream = Stream::open_with(&server.url("/live.mp3"), options);
    let _listener = listen(&stream);
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    let seen = states(&stream, Duration::from_secs(10), |seen| {
        seen.len() == 3 || has_failed(seen)
    });
    use StreamState::{Playing, Reconnecting};
    assert_eq!(seen, [Playing, Reconnecting, Playing]);
}

#[test]
fn a_server_error_before_playing_is_retried_once_then_reported() {
    let server = TestServer::start(|_| Reply::Body(503, "text/plain", b"busy".to_vec()));
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    wait_for("Failed", Duration::from_secs(3), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("HTTP 503"));
    assert_eq!(server.hits("/live.mp3"), 2);
}

#[test]
fn a_server_that_never_answers_is_retried_once() {
    let server = TestServer::start(|_| Reply::Hang);
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    // Two calls, each given up after the 1 s timeout.
    wait_for("Failed", Duration::from_secs(4), || {
        failed(&stream).is_some()
    });
    let reason = failed(&stream).unwrap();
    assert!(reason.contains("timed out"), "{reason}");
    assert_eq!(server.hits("/live.mp3"), 2);
}

#[test]
fn a_refused_connection_fails_without_a_retry() {
    // A port that was free a moment ago and has nothing listening now.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    // A retry would wait out a delay longer than the test waits. Windows
    // takes about 2 s to report a refusal, so the timeout is above that.
    let options = StreamOptions {
        timeout: Duration::from_secs(5),
        retry_delays: vec![Duration::from_secs(10)],
        ..fast()
    };
    let stream = Stream::open_with(&format!("http://127.0.0.1:{port}/live.mp3"), options);
    wait_for("Failed", Duration::from_secs(4), || {
        failed(&stream).is_some()
    });
    let reason = failed(&stream).unwrap();
    assert!(reason.contains("refused"), "{reason}");
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
fn dead_air_before_a_timeout_is_not_good_playback() {
    // Half a second of audio, then silence until the client gives up.
    let server = TestServer::start(|_| Reply::Stall("audio/mpeg", TONE.to_vec(), 4 * 1024));
    // The timeout alone outlasts `healthy_after` (1 s); the audio does not.
    let options = StreamOptions {
        timeout: Duration::from_secs(2),
        stall: Duration::from_secs(2),
        ..fast()
    };
    let stream = Stream::open_with(&server.url("/live.mp3"), options);
    let _listener = listen(&stream);
    wait_for("Failed", Duration::from_secs(10), || {
        failed(&stream).is_some()
    });
    let reason = failed(&stream).unwrap();
    assert!(reason.contains("timed out"), "{reason}");
    assert_eq!(server.hits("/live.mp3"), 1 + fast().retry_delays.len());
}

#[test]
fn a_station_that_played_once_then_fails_is_not_retried_forever() {
    // Every call counts as healthy at zero; only one that played does.
    let calls = AtomicUsize::new(0);
    let server = TestServer::start(move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::Cut("audio/mpeg", TONE.to_vec(), 16 * 1024)
        } else {
            Reply::Body(503, "text/plain", b"busy".to_vec())
        }
    });
    let options = StreamOptions {
        healthy_after: Duration::ZERO,
        ..fast()
    };
    let stream = Stream::open_with(&server.url("/live.mp3"), options);
    let _listener = listen(&stream);
    wait_for("Failed", Duration::from_secs(5), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("HTTP 503"));
    // The call that played, and the two retries.
    assert_eq!(server.hits("/live.mp3"), 3);
}

#[test]
fn the_first_connects_retry_stays_connecting() {
    let server = station_after(1, || Reply::Body(503, "text/plain", b"busy".to_vec()));
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    let _listener = listen(&stream);
    let seen = states(&stream, Duration::from_secs(5), |seen| {
        seen.last() == Some(&StreamState::Playing) || has_failed(seen)
    });
    assert_eq!(seen, [StreamState::Connecting, StreamState::Playing]);
    assert_eq!(server.hits("/live.mp3"), 2);
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
