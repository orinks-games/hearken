//! The listening pass: real stations over the real network, run by hand and
//! read by a person:
//! `cargo test -p hearken-radio --test live -- --ignored --nocapture`
//!
//! Ignored so CI never touches the network, and it asserts nothing: a
//! station that is down is that station's news, not a failure here.

use std::thread;
use std::time::{Duration, Instant};

use hearken_radio::Stream;

/// Freight Fate's `data/radio_catalog.json`, the first station of each kind.
/// Its first AAC entry is the HLS one, so the plain AAC slot takes the first
/// AAC that is not.
const STATIONS: [(&str, &str); 4] = [
    (
        "MP3",
        "http://13743.live.streamtheworld.com:3690/AFN_GRV_SC",
    ),
    ("AAC", "http://stream.xmission.com/krcl-high"),
    ("Ogg", "https://stream.wuvt.vt.edu/wuvt-hq.ogg"),
    (
        "HLS",
        "https://live.amperwave.net/manifest/mhcc-kmhchd2aac-hlsc1.m3u8",
    ),
];

const LISTEN: Duration = Duration::from_secs(10);

#[test]
#[ignore = "plays real stations over the network"]
fn listen_to_real_stations() {
    for (kind, url) in STATIONS {
        let stream = Stream::open(url);
        let feed = stream.feed();
        // Drained at the pace of real playback, as a game's audio thread
        // would: the buffer is only refilled as it is heard.
        let mut buffer = [[0.0_f32; 2]; 8_192];
        let (mut rate, mut heard, mut peak) = (44_100_u32, 0_usize, 0.0_f32);
        let (start, mut last) = (Instant::now(), Instant::now());
        while start.elapsed() < LISTEN {
            thread::sleep(Duration::from_millis(20));
            let due = (last.elapsed().as_secs_f64() * f64::from(rate)) as usize;
            last = Instant::now();
            let read = feed.read(&mut buffer[..due.min(8_192)]);
            if read.frames > 0 {
                rate = read.sample_rate;
                heard += read.frames;
                let loudest = buffer[..read.frames].iter().flatten();
                peak = loudest.fold(peak, |peak, sample| peak.max(sample.abs()));
            }
        }
        println!("{kind}: {url}");
        println!(
            "  state {:?}, title {:?}, heard {:.1} s at peak {peak:.2}",
            stream.state(),
            stream.title(),
            heard as f64 / f64::from(rate),
        );
    }
}
