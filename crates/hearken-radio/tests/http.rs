//! Plain HTTP stations: connecting, playing, failing and letting go.

mod support;

use std::thread;
use std::time::Duration;

use hearken_radio::Stream;
use support::{drain, failed, fast, playing, rms, wait_for, Reply, TestServer};

/// Two seconds of a 440 Hz tone at half scale, as bare MP3 frames, so that
/// repeating it end to end is a clean endless stream:
/// `ffmpeg -f lavfi -i sine=frequency=440:duration=2 -af volume=12dB -ac 1
/// -c:a libmp3lame -b:a 64k -write_xing 0 -id3v2_version 0 tone.mp3`
const TONE: &[u8] = include_bytes!("fixtures/tone.mp3");

fn station() -> TestServer {
    TestServer::start(|_| Reply::Endless("audio/mpeg", TONE.to_vec(), None))
}

#[test]
fn an_mp3_station_plays() {
    let server = station();
    let stream = Stream::open(&server.url("/live.mp3"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    let level = rms(&drain(&stream));
    assert!(level > 0.1, "rms {level}");
}

#[test]
fn the_request_names_hearken_and_nothing_else() {
    let server = station();
    let stream = Stream::open(&server.url("/live.mp3"));
    wait_for("a request", Duration::from_secs(5), || {
        server.hits("/live.mp3") == 1
    });
    let request = server.last_request();
    assert!(request.contains("User-Agent: Hearken/"), "{request}");
    assert!(request.contains("Icy-MetaData: 1"), "{request}");
    assert!(!request.contains('@'), "{request}");
    drop(stream);
}

#[test]
fn a_missing_station_fails_at_once() {
    let server = TestServer::start(|_| Reply::Body(404, "text/html", b"no such station".to_vec()));
    let stream = Stream::open(&server.url("/gone.mp3"));
    wait_for("Failed", Duration::from_secs(2), || {
        failed(&stream).is_some()
    });
    let reason = failed(&stream).unwrap();
    assert!(reason.contains("404"), "{reason}");
    assert_eq!(server.hits("/gone.mp3"), 1);
}

#[test]
fn a_url_that_is_not_one_fails_without_panicking() {
    let stream = Stream::open("not a url");
    wait_for("Failed", Duration::from_secs(1), || {
        failed(&stream).is_some()
    });
}

#[test]
fn a_shoutcast_v1_station_plays() {
    let server = TestServer::start(|_| Reply::IcyV1(TONE.to_vec()));
    let stream = Stream::open(&server.url("/;"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
}

#[test]
fn dropping_every_handle_stops_the_worker() {
    let server = station();
    let stream = Stream::open(&server.url("/live.mp3"));
    let feed = stream.feed();
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    assert_eq!(server.open_connections(), 1);
    drop(stream);
    // The pool may still be playing the feed: it alone keeps the station.
    thread::sleep(Duration::from_millis(300));
    assert_eq!(server.open_connections(), 1);
    drop(feed);
    wait_for("the connection to close", Duration::from_secs(3), || {
        server.open_connections() == 0
    });
}

#[test]
fn dropping_while_connecting_to_a_silent_server_ends_within_the_timeout() {
    let server = TestServer::start(|_| Reply::Hang);
    let stream = Stream::open_with(&server.url("/"), fast());
    // Dropped once the worker is waiting on the server, not before it calls.
    wait_for("a request", Duration::from_secs(3), || {
        server.hits("/") == 1
    });
    drop(stream);
    wait_for("the connection to close", Duration::from_secs(3), || {
        server.open_connections() == 0
    });
}
