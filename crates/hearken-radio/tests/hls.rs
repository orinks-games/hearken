//! Live HLS stations: a sliding window of ADTS or MPEG-TS segments, a
//! master playlist in front of one, and the ways one fails.

mod support;

use std::thread;
use std::time::{Duration, Instant};

use hearken_radio::{Stream, StreamState};
use support::{drain, fast, rms, wait_for, Reply, TestServer};

/// One second of a 440 Hz tone at half scale, as AAC in ADTS frames:
/// `ffmpeg -f lavfi -i sine=frequency=440:duration=1 -af volume=12dB -ac 1
/// -c:a aac -b:a 64k -f adts tone.aac`
const ADTS: &[u8] = include_bytes!("fixtures/tone.aac");

/// The same tone in MPEG-TS: the command above with `-f mpegts tone.ts`.
const TS: &[u8] = include_bytes!("fixtures/tone.ts");

const M3U8: &str = "application/vnd.apple.mpegurl";

const MASTER: &str = "#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.2\"
low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"
live.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2000000,CODECS=\"avc1.64001f,mp4a.40.2\"
tv.m3u8
";

const LOCKED: &str = "#EXTM3U
#EXT-X-TARGETDURATION:1
#EXT-X-MEDIA-SEQUENCE:0
#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"
#EXTINF:1.0,Hearken - Live
seg0.aac
#EXTINF:1.0,Hearken - Live
seg1.aac
#EXTINF:1.0,Hearken - Live
seg2.aac
";

/// Three segments that never move on, with no `#EXT-X-ENDLIST` to say so.
const STILL: &str = "#EXTM3U
#EXT-X-TARGETDURATION:1
#EXT-X-MEDIA-SEQUENCE:0
#EXTINF:1.0,Hearken - Live
seg0.aac
#EXTINF:1.0,Hearken - Live
seg1.aac
#EXTINF:1.0,Hearken - Live
seg2.aac
";

/// Four one-second segments from `seg{n}`.
fn window(n: u64, extension: &str) -> Vec<u8> {
    let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{n}\n");
    for sequence in n..n + 4 {
        text += &format!("#EXTINF:1.0,Hearken - Live\nseg{sequence}.{extension}\n");
    }
    text.into_bytes()
}

/// `/live.m3u8` (ADTS) and `/ts.m3u8` (MPEG-TS) are live windows that move
/// on one segment for every whole second the server has been up.
fn station() -> TestServer {
    let started = Instant::now();
    TestServer::start(move |path| {
        let n = started.elapsed().as_secs();
        let playlist = |text: Vec<u8>| Reply::Body(200, M3U8, text);
        match path {
            "/live.m3u8" => playlist(window(n, "aac")),
            "/ts.m3u8" => playlist(window(n, "ts")),
            "/master.m3u8" => playlist(MASTER.into()),
            "/locked.m3u8" => playlist(LOCKED.into()),
            "/still.m3u8" => playlist(STILL.into()),
            _ if path.ends_with(".aac") => Reply::Body(200, "audio/aac", ADTS.to_vec()),
            _ if path.ends_with(".ts") => Reply::Body(200, "video/mp2t", TS.to_vec()),
            _ => Reply::Body(404, "text/plain", b"not here".to_vec()),
        }
    })
}

/// Requests for any `/seg{n}.{extension}` the tests' playlists name.
fn segment_hits(server: &TestServer, extension: &str) -> usize {
    (0..20)
        .map(|n| server.hits(&format!("/seg{n}.{extension}")))
        .sum()
}

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
fn a_live_hls_station_with_adts_segments_plays_with_its_title() {
    let server = station();
    let stream = Stream::open(&server.url("/live.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    assert_eq!(stream.title().as_deref(), Some("Hearken - Live"));
    let level = rms(&drain(&stream));
    assert!(level > 0.1, "rms {level}");
}

#[test]
fn a_live_hls_station_with_ts_segments_plays() {
    let server = station();
    let stream = Stream::open(&server.url("/ts.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    let level = rms(&drain(&stream));
    assert!(level > 0.1, "rms {level}");
}

#[test]
fn a_master_playlist_plays_its_best_audio_variant() {
    let server = station();
    let stream = Stream::open(&server.url("/master.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    assert_eq!(server.hits("/low.m3u8"), 0);
    assert_eq!(server.hits("/tv.m3u8"), 0);
}

#[test]
fn an_encrypted_hls_station_fails_at_once() {
    let server = station();
    let stream = Stream::open(&server.url("/locked.m3u8"));
    wait_for("Failed", Duration::from_secs(2), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("encrypted HLS"));
    assert_eq!(segment_hits(&server, "aac"), 0);
}

#[test]
fn an_hls_playlist_that_stops_growing_counts_as_a_stall() {
    let server = station();
    let stream = Stream::open_with(&server.url("/still.m3u8"), fast());
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    // Three seconds of segments, served half a second ahead of real time,
    // then `stall` (1 s) of re-reads that find nothing new: about 3.5 s.
    wait_for("leaving Playing", Duration::from_secs(5), || {
        !playing(&stream)
    });
}

#[test]
fn a_live_hls_station_fetches_each_segment_as_it_comes_due() {
    let server = station();
    let stream = Stream::open_with(&server.url("/live.m3u8"), fast());
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    thread::sleep(Duration::from_millis(900));
    // fast() lets the reader run half a second ahead of real time: the
    // first segment at once, the second at 0.5 s, the third not until
    // 1.5 s. All at once would overflow the ring.
    assert_eq!(segment_hits(&server, "aac"), 2);
}
