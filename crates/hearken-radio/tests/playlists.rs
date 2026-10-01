//! Stations behind a playlist, and stations that do not say what they send.

mod support;

use std::time::Duration;

use hearken_radio::{Stream, StreamState};
use support::{fast, wait_for, Reply, TestServer};

/// The tone from `tests/http.rs`; see there for how it is made.
const TONE: &[u8] = include_bytes!("fixtures/tone.mp3");

fn playing(stream: &Stream) -> bool {
    stream.state() == StreamState::Playing
}

fn failed(stream: &Stream) -> Option<String> {
    match stream.state() {
        StreamState::Failed(reason) => Some(reason),
        _ => None,
    }
}

#[test]
fn a_pls_link_plays_its_first_station() {
    let server = TestServer::start(|path| match path {
        "/list.pls" => Reply::Body(
            200,
            "audio/x-scpls",
            b"[playlist]\nNumberOfEntries=1\nFile1=/live.mp3\n".to_vec(),
        ),
        _ => Reply::Endless("audio/mpeg", TONE.to_vec(), None),
    });
    let stream = Stream::open(&server.url("/list.pls"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    assert_eq!(server.hits("/live.mp3"), 1);
}

#[test]
fn an_mp3_served_as_octet_stream_plays() {
    let server =
        TestServer::start(|_| Reply::Endless("application/octet-stream", TONE.to_vec(), None));
    let stream = Stream::open(&server.url("/live"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
}

#[test]
fn a_playlist_served_as_text_plain_is_followed() {
    let server = TestServer::start(|path| match path {
        "/list" => Reply::Body(200, "text/plain", b"[playlist]\nFile1=/live.mp3\n".to_vec()),
        _ => Reply::Endless("audio/mpeg", TONE.to_vec(), None),
    });
    let stream = Stream::open(&server.url("/list"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
}

#[test]
fn a_playlist_loop_fails_after_three_hops() {
    let server = TestServer::start(|path| {
        let next = match path {
            "/a.m3u" => "b.m3u",
            "/b.m3u" => "c.m3u",
            "/c.m3u" => "d.m3u",
            _ => "a.m3u",
        };
        Reply::Body(200, "audio/x-mpegurl", next.as_bytes().to_vec())
    });
    let stream = Stream::open_with(&server.url("/a.m3u"), fast());
    wait_for("Failed", Duration::from_secs(5), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("too many playlist hops"));
    let hits: usize = ["/a.m3u", "/b.m3u", "/c.m3u", "/d.m3u"]
        .iter()
        .map(|path| server.hits(path))
        .sum();
    assert!(hits <= 4, "{hits} requests");
}

#[test]
fn a_playlist_with_no_stream_fails() {
    let server = TestServer::start(|_| {
        Reply::Body(200, "audio/x-mpegurl", b"#EXTM3U\n# nothing\n".to_vec())
    });
    let stream = Stream::open_with(&server.url("/empty.m3u"), fast());
    wait_for("Failed", Duration::from_secs(5), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("empty playlist"));
}
