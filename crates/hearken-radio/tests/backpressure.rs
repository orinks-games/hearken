//! A full buffer holds the worker back: a station is read no faster than it
//! is heard, as BASS did.

mod support;

use std::thread;
use std::time::{Duration, Instant};

use hearken_radio::{Stream, StreamOptions};
use support::{fast, listen, playing, wait_for, Reply, TestServer};

/// The tone from `tests/http.rs`; see there for how it is made.
const TONE: &[u8] = include_bytes!("fixtures/tone.mp3");

fn station() -> TestServer {
    TestServer::start(|_| Reply::Endless("audio/mpeg", TONE.to_vec(), None))
}

#[test]
fn an_unheard_stream_stops_reading_when_its_buffer_is_full() {
    // As fast as it is read, so the sockets' buffers fill at once.
    let server = TestServer::start(|_| Reply::Flood("audio/mpeg", TONE.to_vec()));
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    // Once the buffers hold what the worker leaves unread, the server's
    // writes wait too: half a second with nothing sent.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let sent = server.bytes_sent("/live.mp3");
        thread::sleep(Duration::from_millis(500));
        if server.bytes_sent("/live.mp3") == sent {
            break;
        }
        assert!(Instant::now() < deadline, "still sending at {sent} bytes");
    }
}

#[test]
fn a_paused_stream_resumes_where_it_left_off() {
    let server = station();
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    let listener = listen(&stream);
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    drop(listener);
    // Twice the read timeout: waiting for room is not a read that hangs.
    let paused = Instant::now();
    while paused.elapsed() < Duration::from_secs(2) {
        assert!(playing(&stream), "paused: {:?}", stream.state());
        thread::sleep(Duration::from_millis(10));
    }
    let listener = listen(&stream);
    // Twice what the buffer holds, so the worker woke and read on.
    wait_for("2 s of audio", Duration::from_secs(5), || {
        assert!(playing(&stream), "resumed: {:?}", stream.state());
        listener.heard() >= Duration::from_secs(2)
    });
}

#[test]
fn dropping_a_stream_blocked_on_a_full_buffer_ends_the_worker() {
    let server = station();
    let stream = Stream::open_with(&server.url("/live.mp3"), fast());
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    // The 1 s buffer fills in about a quarter second at the server's pace.
    thread::sleep(Duration::from_millis(500));
    drop(stream);
    wait_for("the connection to close", Duration::from_secs(3), || {
        server.open_connections() == 0
    });
}

#[test]
fn a_prebuffer_longer_than_the_buffer_still_plays() {
    let server = station();
    let options = StreamOptions {
        prebuffer: Duration::from_secs(2),
        ..fast()
    };
    let stream = Stream::open_with(&server.url("/live.mp3"), options);
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
}
