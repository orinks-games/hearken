//! Renders a song to a WAV file, to hear a style without a game:
//!
//!     cargo run -p hearken-band --example render -- \
//!         BANK.json STYLE.json SOUNDS_DIR SEED SECONDS OUT.wav [arc|calm|full]
//!
//! `arc` (the default) plays a run: calm at first, every layer in by two
//! thirds of the way, hurry for the last sixth, and danger now and then.

use hearken_audio::{Audio, KiraBackend, LooseClips};
use hearken_band::{Band, Bank, Feel, Song, Style};
use std::io::Write;

const RATE: u32 = 44_100;
const CHUNK: usize = 1024;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 7 {
        eprintln!("usage: render BANK STYLE SOUNDS SEED SECONDS OUT [arc|calm|full]");
        std::process::exit(2);
    }
    let bank: Bank = serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let style: Style = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    for p in style.problems(&bank) {
        eprintln!("style: {p}");
    }
    let seed: u64 = args[4].parse().unwrap();
    let seconds: f64 = args[5].parse().unwrap();
    let shape = args.get(7).map(String::as_str).unwrap_or("arc");
    let song = Song::compose(&style, &bank, seed);
    println!("{}", song.describe());
    let backend = KiraBackend::offline(RATE).unwrap();
    let mut audio = Audio::new(Box::new(backend), Box::new(LooseClips::at(&args[3])));
    let feel_at = |t: f64| -> Feel {
        let x = (t / seconds) as f32;
        match shape {
            "calm" => Feel::new(0.2),
            "full" => Feel::new(1.0),
            _ => Feel {
                intensity: (x * 1.5).min(1.0),
                hurry: x > 0.83,
                danger: (0.45..0.55).contains(&x),
            },
        }
    };
    let mut band = Band::new(song, bank);
    band.start(&mut audio, feel_at(0.0));
    let dt = CHUNK as f64 / f64::from(RATE);
    let mut t = 0.0;
    let mut out: Vec<[f32; 2]> = Vec::new();
    while t < seconds + 2.0 {
        if t < seconds {
            band.follow(&mut audio, feel_at(t), dt);
        } else if t - dt < seconds {
            band.stop(&mut audio, 1500);
        }
        audio.advance_take(dt);
        audio.update(&|_| false);
        out.extend(audio.render(CHUNK).unwrap());
        t += dt;
    }
    let mut f = std::fs::File::create(&args[6]).unwrap();
    let data_len = (out.len() * 4) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&2u16.to_le_bytes()).unwrap();
    f.write_all(&RATE.to_le_bytes()).unwrap();
    f.write_all(&(RATE * 4).to_le_bytes()).unwrap();
    f.write_all(&4u16.to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    for [l, r] in out {
        for s in [l, r] {
            f.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
                .unwrap();
        }
    }
}
