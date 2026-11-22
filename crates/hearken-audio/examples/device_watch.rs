//! Plays a quiet tone on the default sound device and prints the device
//! watcher's log lines, for trying device changes by hand.
//!
//! ```text
//! cargo run -p hearken-audio --example device_watch -- [SECONDS]
//! ```
//!
//! Run it, connect or disconnect a headset (or change the default output
//! device), and read the `audio:` lines it prints. It runs for `SECONDS`, or
//! until Enter is pressed or standard input closes when none is given.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_audio::{Backend, KiraBackend, Mix, Source};

const RATE: u32 = 44_100;

/// Prints `audio:` and `kira:` lines with the seconds since start.
struct Printer(Instant);

impl log::Log for Printer {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        let text = record.args().to_string();
        if text.starts_with("audio:") || text.starts_with("kira:") {
            println!(
                "{:8.2} {:5} {text}",
                self.0.elapsed().as_secs_f32(),
                record.level()
            );
        }
    }

    fn flush(&self) {}
}

fn main() {
    let seconds: Option<f32> = std::env::args().nth(1).and_then(|s| s.parse().ok());
    let started = Instant::now();
    log::set_logger(Box::leak(Box::new(Printer(started)))).expect("the only logger");
    log::set_max_level(log::LevelFilter::Debug);

    let mut audio = match KiraBackend::open() {
        Ok(audio) => audio,
        Err(e) => {
            eprintln!("could not open the sound device: {e}");
            std::process::exit(1);
        }
    };
    // One second of 440 Hz, looped, at a low level.
    let tone: Vec<f32> = (0..RATE)
        .map(|i| (i as f32 * 440.0 * TAU / RATE as f32).sin() * 0.5)
        .collect();
    let clip = Source::Clip(hearken_synth::wav_bytes(&tone, RATE).into());
    let mix = Mix {
        gain: 0.05,
        ..Mix::UNITY
    };
    if let Err(e) = audio.play("watch/tone", clip, true, mix) {
        eprintln!("could not play the tone: {e}");
    }

    let done = Arc::new(AtomicBool::new(false));
    if seconds.is_none() {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            done.store(true, Ordering::SeqCst);
        });
    }
    while !done.load(Ordering::SeqCst)
        && seconds.is_none_or(|s| started.elapsed().as_secs_f32() < s)
    {
        audio.update();
        std::thread::sleep(Duration::from_millis(50));
    }
    audio.shutdown();
}
