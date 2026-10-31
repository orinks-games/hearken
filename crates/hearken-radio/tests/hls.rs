//! Live HLS stations: a sliding window of ADTS or MPEG-TS segments, a
//! master playlist in front of one, and the ways one fails.

mod support;

use std::time::{Duration, Instant};

use hearken_radio::{RawTitle, Stream, StreamOptions};
use support::{drain, failed, fast, listen, playing, rms, wait_for, Reply, TestServer};

/// One second of a 440 Hz tone at half scale, as AAC in ADTS frames:
/// `ffmpeg -f lavfi -i sine=frequency=440:duration=1 -af volume=12dB -ac 1
/// -c:a aac -b:a 64k -f adts tone.aac`
const ADTS: &[u8] = include_bytes!("fixtures/tone.aac");

/// The same tone in MPEG-TS: the command above with `-f mpegts tone.ts`.
const TS: &[u8] = include_bytes!("fixtures/tone.ts");

/// The same again on packet id 0x200 rather than 0x100, as a station's
/// tracks may be renumbered after a discontinuity (an inserted ad): the
/// command above with `-mpegts_start_pid 0x200 -f mpegts tone-pid200.ts`.
const TS_PID_200: &[u8] = include_bytes!("fixtures/tone-pid200.ts");

/// The tone as fMP4 (CMAF): an init segment and one fragment,
/// `ffmpeg -f lavfi -i sine=frequency=440:duration=1 -af volume=12dB -ac 1
/// -c:a aac -b:a 64k -f hls -hls_time 1 -hls_segment_type fmp4
/// -hls_fmp4_init_filename init.mp4 -hls_segment_filename 'seg%d.m4s'
/// index.m3u8`, keeping `init.mp4` and `seg0.m4s`.
const FMP4_INIT: &[u8] = include_bytes!("fixtures/tone-init.mp4");
const FMP4: &[u8] = include_bytes!("fixtures/tone.m4s");

const M3U8: &str = "application/vnd.apple.mpegurl";

/// Video variants only, their audio in a separate rendition, as TV-style
/// stations publish.
const RENDITIONS: &str = "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Commentary\",DEFAULT=NO,URI=\"commentary.m3u8\"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"Main\",DEFAULT=YES,URI=\"live.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=800000,CODECS=\"avc1.64001f,mp4a.40.2\",AUDIO=\"aud\"
tv-low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2000000,CODECS=\"avc1.64001f,mp4a.40.2\",AUDIO=\"aud\"
tv.m3u8
";

/// Three fMP4 fragments behind one init segment, never ending.
fn fmp4_window(n: u64) -> Vec<u8> {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{n}\n#EXT-X-MAP:URI=\"init.mp4\"\n"
    );
    for sequence in n..n + 4 {
        text += &format!("#EXTINF:1.0,Hearken - Live\nseg{sequence}.m4s\n");
    }
    text.into_bytes()
}

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

/// A segment on packet id 0x100, then a discontinuity, then two on 0x200.
const RENUMBERED: &str = "#EXTM3U
#EXT-X-TARGETDURATION:1
#EXT-X-MEDIA-SEQUENCE:0
#EXTINF:1.0,
seg0.ts
#EXT-X-DISCONTINUITY
#EXTINF:1.0,
moved1.ts
#EXTINF:1.0,
moved2.ts
#EXT-X-ENDLIST
";

/// Four one-second segments from `seg{n}`.
fn window(n: u64, extension: &str) -> Vec<u8> {
    let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{n}\n");
    for sequence in n..n + 4 {
        text += &format!("#EXTINF:1.0,Hearken - Live\nseg{sequence}.{extension}\n");
    }
    text.into_bytes()
}

/// Segments in a window hours long, as YouTube publishes a live stream:
/// thousands of them, each named by a long signed address, so the list
/// runs to several MiB and its live edge sits at the very end.
const DVR_SEGMENTS: u64 = 3_000;

/// The DVR window from `n`, every segment URI padded to about 1,300 bytes.
fn dvr_window(n: u64) -> Vec<u8> {
    let padding = "x".repeat(1_280);
    let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{n}\n");
    for sequence in n..n + DVR_SEGMENTS {
        text += &format!("#EXTINF:1.0,\ndvr{sequence}.ts?sig={padding}\n");
    }
    text.into_bytes()
}

/// Four packed-audio segments from `n`, each named `seg.ts` in a folder of
/// its own, as YouTube's audio-only streams name them.
fn packed_window(n: u64) -> Vec<u8> {
    let mut text = format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:{n}\n");
    for sequence in n..n + 4 {
        text += &format!("#EXTINF:1.0,\npacked/sq/{sequence}/seg.ts\n");
    }
    text.into_bytes()
}

/// The ADTS tone behind an ID3v2.4 tag holding one PRIV timestamp frame,
/// the way packed audio opens every segment.
fn packed_segment() -> Vec<u8> {
    let owner = b"com.apple.streaming.transportStreamTimestamp\0";
    let mut frame = b"PRIV".to_vec();
    frame.extend(((owner.len() + 8) as u32).to_be_bytes());
    frame.extend([0, 0]);
    frame.extend(owner);
    frame.extend([0, 0, 0, 0, 0, 0x01, 0x5f, 0x90]);
    let mut tag = b"ID3\x04\x00\x00\x00\x00\x00".to_vec();
    tag.push(frame.len() as u8);
    tag.extend(frame);
    tag.extend(ADTS);
    tag
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
            "/dvr.m3u8" => Reply::Gzip(M3U8, dvr_window(n)),
            "/packed.m3u8" => playlist(packed_window(n)),
            _ if path.starts_with("/packed/") => Reply::Body(200, "video/mp2t", packed_segment()),
            _ if path.starts_with("/dvr") => Reply::Body(200, "video/mp2t", TS.to_vec()),
            "/ts.m3u8" => playlist(window(n, "ts")),
            "/master.m3u8" => playlist(MASTER.into()),
            "/renditions.m3u8" => playlist(RENDITIONS.into()),
            "/cmaf.m3u8" => playlist(fmp4_window(n)),
            "/init.mp4" => Reply::Body(200, "video/mp4", FMP4_INIT.to_vec()),
            _ if path.ends_with(".m4s") => Reply::Body(200, "video/iso.segment", FMP4.to_vec()),
            "/locked.m3u8" => playlist(LOCKED.into()),
            "/still.m3u8" => playlist(STILL.into()),
            "/renumbered.m3u8" => playlist(RENUMBERED.into()),
            _ if path.starts_with("/moved") => Reply::Body(200, "video/mp2t", TS_PID_200.to_vec()),
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

#[test]
fn a_live_hls_station_with_adts_segments_plays_with_its_title() {
    let server = station();
    let stream = Stream::open(&server.url("/live.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    assert_eq!(stream.title().as_deref(), Some("Hearken - Live"));
    assert_eq!(
        stream.raw_title(),
        Some(RawTitle::Extinf("1,Hearken - Live".into()))
    );
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
fn a_live_window_hours_long_starts_at_its_live_edge() {
    // Cut at 1 MiB, the list ended hours behind live in the middle of a
    // line, and the station took 19 s to be heard (YouTube, 2026-10-31).
    let server = station();
    let stream = Stream::open(&server.url("/dvr.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    let early: usize = (0..100).map(|n| hits_from(&server, n)).sum();
    assert_eq!(
        early, 0,
        "a segment from the start of the window was fetched"
    );
    let edge: usize = (DVR_SEGMENTS - 4..DVR_SEGMENTS + 20)
        .map(|n| hits_from(&server, n))
        .sum();
    assert!(edge > 0, "no segment from the live edge was fetched");
}

#[test]
fn packed_audio_named_like_ts_plays() {
    // YouTube's audio-only streams: ID3-tagged ADTS in segments named
    // `seg.ts`, which the reader demuxed as MPEG-TS and found empty.
    let server = station();
    let stream = Stream::open(&server.url("/packed.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    let level = rms(&drain(&stream));
    assert!(level > 0.1, "rms {level}");
}

#[test]
fn a_long_playlist_is_re_read_gzipped() {
    let server = station();
    let stream = Stream::open(&server.url("/dvr.m3u8"));
    let _listener = listen(&stream);
    wait_for("re-read", Duration::from_secs(10), || {
        server.hits("/dvr.m3u8") >= 3
    });
    let plain = dvr_window(0).len();
    let sent = server.bytes_sent("/dvr.m3u8");
    let reads = server.hits("/dvr.m3u8");
    // The first read is plain, before anyone knows it is a playlist; every
    // re-read after it comes gzipped, a small fraction of the text.
    assert!(
        sent < plain + (reads - 1) * plain / 10,
        "{sent} bytes for {reads} reads of a {plain}-byte playlist"
    );
}

/// Requests for `/dvr{n}.ts`, its signature whatever it is.
fn hits_from(server: &TestServer, n: u64) -> usize {
    server.hits(&format!("/dvr{n}.ts?sig={}", "x".repeat(1_280)))
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
    // No retries, so the stall itself is reported.
    let options = StreamOptions {
        retry_delays: Vec::new(),
        ..fast()
    };
    let stream = Stream::open_with(&server.url("/still.m3u8"), options);
    let _listener = listen(&stream);
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    // Three seconds of segments, heard as fast as they decode, then `stall`
    // (1 s) of re-reads that find nothing new.
    wait_for("Failed", Duration::from_secs(5), || {
        failed(&stream).is_some()
    });
    assert_eq!(failed(&stream).as_deref(), Some("stalled"));
}

#[test]
fn a_ts_station_renumbered_at_a_discontinuity_plays_on() {
    let server = station();
    // No retries: a retry would replay the first segment and hear enough.
    let options = StreamOptions {
        retry_delays: Vec::new(),
        ..fast()
    };
    let stream = Stream::open_with(&server.url("/renumbered.m3u8"), options);
    let listener = listen(&stream);
    // Three one-second segments; the first alone is about one second.
    wait_for("2.5 s of audio", Duration::from_secs(5), || {
        listener.heard() >= Duration::from_millis(2_500)
    });
}

#[test]
fn a_live_hls_station_with_fmp4_segments_plays() {
    let server = station();
    let stream = Stream::open_with(&server.url("/cmaf.m3u8"), fast());
    let listener = listen(&stream);
    wait_for("2.5 s of audio", Duration::from_secs(8), || {
        listener.heard() >= Duration::from_millis(2_500) || failed(&stream).is_some()
    });
    assert_eq!(failed(&stream), None);
    // The init segment once, however many fragments follow it.
    assert_eq!(server.hits("/init.mp4"), 1);
}

#[test]
fn an_fmp4_station_is_heard_as_the_tone() {
    let server = station();
    let stream = Stream::open(&server.url("/cmaf.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || {
        playing(&stream) || failed(&stream).is_some()
    });
    assert_eq!(failed(&stream), None);
    let level = rms(&drain(&stream));
    assert!(level > 0.1, "rms {level}");
}

#[test]
fn a_master_with_only_video_variants_plays_its_default_audio_rendition() {
    let server = station();
    let stream = Stream::open(&server.url("/renditions.m3u8"));
    wait_for("Playing", Duration::from_secs(5), || playing(&stream));
    assert_eq!(server.hits("/tv.m3u8"), 0);
    assert_eq!(server.hits("/tv-low.m3u8"), 0);
    assert_eq!(server.hits("/commentary.m3u8"), 0);
}
