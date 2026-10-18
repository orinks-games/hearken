//! Renders a WAV through the cab-speaker voicing, block by block as a
//! backend's audio thread runs it, and times it.
//!
//! ```text
//! cargo run --release -p hearken-audio --example cab_render -- IN.wav OUT.wav [MODE]
//! ```
//!
//! `MODE` is `up` (windows up, the default), `down` (window down), `clean`
//! (the voicing off), or `glass` (up, then the window going down over 2 s
//! from a third of the way in, and back up over 2 s from two thirds), or
//! `speakers` (up, without the room, to check against a reference). The
//! input is 16-bit PCM WAV, mono or stereo, at any rate; the output is
//! 16-bit stereo at the same rate. The time it took is printed against the
//! audio's length.

use std::time::Instant;

use hearken_audio::{CabSpeakers, CabSpeakersParams};

/// What a backend's audio thread hands an effect at a time.
const BLOCK: usize = 128;

fn read_wav(bytes: &[u8]) -> (u32, Vec<[f32; 2]>) {
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    assert_eq!(&bytes[..4], b"RIFF", "not a WAV");
    let (mut at, mut channels, mut rate, mut bits) = (12, 0, 0, 0);
    while at + 8 <= bytes.len() {
        let (id, len) = (&bytes[at..at + 4], u32_at(at + 4) as usize);
        let body = at + 8;
        if id == b"fmt " {
            channels = u16_at(body + 2) as usize;
            rate = u32_at(body + 4);
            bits = u16_at(body + 14);
        } else if id == b"data" {
            assert_eq!(bits, 16, "16-bit PCM only");
            let end = (body + len).min(bytes.len());
            let samples: Vec<f32> = bytes[body..end]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&s| i16::from_le_bytes(s) as f32 / 32_768.0)
                .collect();
            let frames = samples
                .chunks_exact(channels)
                .map(|f| [f[0], f[channels.min(2) - 1]])
                .collect();
            return (rate, frames);
        }
        at = body + len + (len & 1);
    }
    panic!("no data chunk");
}

fn write_wav(path: &str, rate: u32, frames: &[[f32; 2]]) {
    let data_len = (frames.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for f in frames {
        for s in f {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
        }
    }
    std::fs::write(path, out).expect("write the output");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = match (args.first(), args.get(1)) {
        (Some(i), Some(o)) => (i.clone(), o.clone()),
        _ => {
            eprintln!("usage: cab_render IN.wav OUT.wav [up|down|clean|glass]");
            std::process::exit(2);
        }
    };
    let mode = args.get(2).map_or("up", String::as_str);
    let (rate, mut frames) = read_wav(&std::fs::read(&input).expect("read the input"));
    let mut params = CabSpeakersParams::default();
    if mode == "speakers" {
        params.room_wet = 0.0;
    }
    let cab = CabSpeakers::new(params);
    match mode {
        "up" | "glass" | "speakers" => {}
        "down" => cab.set_window(1.0, 0),
        "clean" => cab.set_amount(0.0, 0),
        other => panic!("unknown mode {other}"),
    }
    let mut dsp = cab.processor();
    let (third, two_thirds) = (frames.len() / 3, 2 * frames.len() / 3);
    let started = Instant::now();
    for (i, block) in frames.chunks_mut(BLOCK).enumerate() {
        if mode == "glass" {
            let at = i * BLOCK;
            if at <= third && third < at + BLOCK {
                cab.set_window(1.0, 2_000);
            } else if at <= two_thirds && two_thirds < at + BLOCK {
                cab.set_window(0.0, 2_000);
            }
        }
        dsp.process(block, rate as f32);
    }
    let took = started.elapsed().as_secs_f64();
    let length = frames.len() as f64 / f64::from(rate);
    println!(
        "{mode}: {length:.1} s of stereo at {rate} Hz in {:.1} ms, {:.3}% of one core",
        took * 1000.0,
        100.0 * took / length
    );
    write_wav(&output, rate, &frames);
}
