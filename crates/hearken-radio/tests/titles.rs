//! Now-playing titles: ICY blocks between the audio, and the comments at
//! the head of each Ogg track.

mod support;

use std::time::Duration;

use hearken_radio::{RawTitle, Stream};
use support::{fast, wait_for, Reply, TestServer};

/// The tone from `tests/http.rs`; see there for how it is made.
const TONE: &[u8] = include_bytes!("fixtures/tone.mp3");

/// One second of a 440 Hz tone at half scale, as Vorbis with an artist and
/// a title, and the same with `title=Second` for `second.ogg`:
/// `ffmpeg -f lavfi -i sine=frequency=440:duration=1 -af volume=12dB -ac 1
/// -c:a libvorbis -metadata artist=Hearken -metadata title=First first.ogg`
const FIRST: &[u8] = include_bytes!("fixtures/first.ogg");
const SECOND: &[u8] = include_bytes!("fixtures/second.ogg");

fn icy_station(title: &[u8]) -> TestServer {
    let title = title.to_vec();
    TestServer::start(move |_| Reply::Endless("audio/mpeg", TONE.to_vec(), Some(title.clone())))
}

fn titled(stream: &Stream, title: &str) -> impl FnMut() -> bool {
    let stream = stream.clone();
    let title = title.to_owned();
    move || stream.title().as_deref() == Some(&title)
}

#[test]
fn an_icy_station_reports_its_title() {
    let server = icy_station(b"Band - Song");
    let stream = Stream::open(&server.url("/live.mp3"));
    wait_for(
        "the title",
        Duration::from_secs(5),
        titled(&stream, "Band - Song"),
    );
}

#[test]
fn a_latin1_icy_title_reads_as_latin1() {
    let server = icy_station(b"Caf\xe9");
    let stream = Stream::open(&server.url("/live.mp3"));
    wait_for("the title", Duration::from_secs(5), titled(&stream, "Café"));
}

#[test]
fn a_chained_ogg_station_reports_each_tracks_title() {
    let chained = [FIRST, SECOND].concat();
    let server = TestServer::start(move |_| Reply::Body(200, "application/ogg", chained.clone()));
    let stream = Stream::open_with(&server.url("/live.ogg"), fast());
    wait_for(
        "the second track's title",
        Duration::from_secs(5),
        titled(&stream, "Hearken - Second"),
    );
}

#[test]
fn an_icy_station_hands_over_its_block_as_sent() {
    // Latin-1, unparsed, the zero padding gone: a game with its own reading
    // of a title gets the bytes the station sent.
    let server = icy_station(b"Caf\xe9 - title=\"x\"");
    let stream = Stream::open(&server.url("/live.mp3"));
    wait_for("the raw title", Duration::from_secs(5), || {
        stream.raw_title().is_some()
    });
    assert_eq!(
        stream.raw_title(),
        Some(RawTitle::Icy(
            b"StreamTitle='Caf\xe9 - title=\"x\"';".to_vec()
        ))
    );
}

#[test]
fn an_ogg_station_hands_over_its_comments() {
    let server = TestServer::start(move |_| Reply::Body(200, "application/ogg", FIRST.to_vec()));
    let stream = Stream::open_with(&server.url("/live.ogg"), fast());
    wait_for("the raw title", Duration::from_secs(5), || {
        stream.raw_title().is_some()
    });
    let Some(RawTitle::Comments(comments)) = stream.raw_title() else {
        panic!("{:?}", stream.raw_title());
    };
    let has = |want: &str| comments.iter().any(|c| c.eq_ignore_ascii_case(want));
    assert!(has("artist=Hearken"), "{comments:?}");
    assert!(has("title=First"), "{comments:?}");
}
