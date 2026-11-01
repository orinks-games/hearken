//! Render a line to a WAV file: `cargo run -p hearken-genny --example say -- "US female" out.wav "Hello there."`

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, voice, out, text, ..] = args.as_slice() else {
        eprintln!("usage: say VOICE OUT.wav TEXT [RATE]");
        std::process::exit(2);
    };
    let rate: f64 = args.get(4).and_then(|r| r.parse().ok()).unwrap_or(1.0);
    let v = hearken_genny::voice(voice).expect("a voice from VOICE_NAMES");
    let sr = 44_100u32;
    let start = std::time::Instant::now();
    let samples = hearken_genny::speak(text, &v, sr as f64, rate);
    eprintln!(
        "{} samples ({:.2}s) in {:?}",
        samples.len(),
        samples.len() as f64 / sr as f64,
        start.elapsed()
    );
    let mut f = std::fs::File::create(out).expect("create the WAV");
    let data_len = (samples.len() * 2) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&sr.to_le_bytes()).unwrap();
    f.write_all(&(sr * 2).to_le_bytes()).unwrap();
    f.write_all(&2u16.to_le_bytes()).unwrap();
    f.write_all(&16u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    for s in samples {
        f.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .unwrap();
    }
}
