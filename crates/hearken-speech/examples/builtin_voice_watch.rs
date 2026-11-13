//! Keeps the built-in voice reading a passage on the default sound device
//! and prints its log lines, for trying device changes by hand.
//!
//! ```text
//! cargo run -p hearken-speech --features genny-voice --example builtin_voice_watch -- [SECONDS]
//! ```
//!
//! Run it, connect or disconnect a headset (or change the default output
//! device), and read the `Built-in voice` lines it prints. It runs for
//! `SECONDS`, or until Enter is pressed or standard input closes when none
//! is given.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_speech::builtin::{WithBuiltin, BUILTIN_ID};
use hearken_speech::VoiceRegistry;

const PASSAGE: &str = "The truck leaves the yard at dawn. Fog lies along the river \
    road, and the first town is forty miles out. Check the mirrors, ease onto the \
    highway, and keep to the right lane until the bridge.";

/// Prints `Built-in voice` lines with the seconds since start.
struct Printer(Instant);

impl log::Log for Printer {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        let text = record.args().to_string();
        if text.starts_with("Built-in voice") {
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

    let registry = WithBuiltin::new(None);
    let mut voice = match registry.acquire(BUILTIN_ID) {
        Ok(voice) => voice,
        Err(e) => {
            eprintln!("no built-in voice: {e:?}");
            std::process::exit(1);
        }
    };

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
        // Start the passage again whenever it is not speaking.
        if !voice.is_speaking().unwrap_or(true) {
            let _ = voice.speak(PASSAGE, false);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
