//! Plain HTTP stations: connecting, playing, failing and letting go.

mod support;

use std::thread;
use std::time::Duration;

use hearken_radio::{Stream, StreamOptions};
use support::{drain, failed, fast, listen, playing, rms, wait_for, Reply, TestServer};

/// Two seconds of a 440 Hz tone at half scale, as bare MP3 frames, so that
/// repeating it end to end is a clean endless stream:
/// `ffmpeg -f lavfi -i sine=frequency=440:duration=2 -af volume=12dB -ac 1
/// -c:a libmp3lame -b:a 64k -write_xing 0 -id3v2_version 0 tone.mp3`
const TONE: &[u8] = include_bytes!("fixtures/tone.mp3");

/// The tone as AAC in ADTS frames, from `tests/hls.rs`; see there for how
/// it is made.
const ADTS: &[u8] = include_bytes!("fixtures/tone.aac");

/// One second of the tone as Ogg Opus, from hearken-audio's tests; see
/// `crates/hearken-audio/tests/kira.rs` for how it is made.
const OPUS: &[u8] = include_bytes!("../../hearken-audio/tests/fixtures/tone.opus");

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
fn an_ogg_opus_station_plays() {
    let server = TestServer::start(|_| Reply::Endless("audio/ogg", OPUS.to_vec(), None));
    let stream = Stream::open_with(&server.url("/live.opus"), fast());
    wait_for("Playing", Duration::from_secs(5), || {
        playing(&stream) || failed(&stream).is_some()
    });
    assert_eq!(failed(&stream), None);
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
    assert_eq!(failed(&stream).as_deref(), Some("bad URL"));
}

/// The tone's ADTS frames with `bits` set in byte `at` of the 20th frame's
/// header, served endlessly: heard for two seconds, so past the bad frame
/// twice, without a failure or a second call.
fn plays_through_a_corrupt_adts_header(at: usize, bits: u8) {
    let mut body = ADTS.to_vec();
    let mut frame = 0;
    for _ in 0..20 {
        // A frame's length is 13 bits across header bytes 3 to 5.
        let header = &body[frame..frame + 6];
        let length = usize::from(header[3] & 0x03) << 11
            | usize::from(header[4]) << 3
            | usize::from(header[5]) >> 5;
        frame += length;
    }
    body[frame + at] |= bits;
    let server = TestServer::start(move |_| Reply::Endless("audio/aac", body.clone(), None));
    let stream = Stream::open_with(&server.url("/live.aac"), fast());
    let listener = listen(&stream);
    wait_for("2 s of audio", Duration::from_secs(5), || {
        listener.heard() >= Duration::from_secs(2) || failed(&stream).is_some()
    });
    assert_eq!(failed(&stream), None);
    assert_eq!(server.hits("/live.aac"), 1);
}

#[test]
fn a_corrupt_adts_frame_header_is_skipped_without_reconnecting() {
    // The sample rate index set to 15, which ADTS forbids: a decode error.
    plays_through_a_corrupt_adts_header(2, 0x3C);
}

#[test]
fn an_adts_header_claiming_several_blocks_is_skipped_without_reconnecting() {
    // The raw data block bits set, which the demuxer reports as unsupported:
    // what most false syncs in the middle of a payload look like.
    plays_through_a_corrupt_adts_header(6, 0x03);
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

#[test]
fn the_request_names_the_game_when_it_says_who_it_is() {
    let server = station();
    let options = StreamOptions {
        user_agent: "SomeGame/1.2 (+https://example.org/game)".into(),
        ..StreamOptions::default()
    };
    let _stream = Stream::open_with(&server.url("/live.mp3"), options);
    wait_for("a request", Duration::from_secs(5), || {
        server.hits("/live.mp3") == 1
    });
    let request = server.last_request();
    assert!(
        request.contains("User-Agent: SomeGame/1.2 (+https://example.org/game)\r\n"),
        "{request}"
    );
    assert!(!request.contains("Hearken/"), "{request}");
}
