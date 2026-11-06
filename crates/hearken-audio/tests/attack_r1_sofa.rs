//! Attack round 1: SOFA files whose sampling rate field is hostile. Each
//! case is the bundled MIT KEMAR file with its Data.SamplingRate chunk
//! rewritten (byte patches made with h5py). Each case runs in a child
//! process, because an allocation failure aborts instead of panicking.
//! Run (cap memory first so the big allocations fail fast):
//! ulimit -v 8000000; cargo test --release -p hearken-audio --features hrtf --test attack_r1_sofa -- --nocapture
//! FAILS: see the per-case notes in the output.

#![cfg(feature = "hrtf")]

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_audio::{HrtfSet, HrtfSource, BUILTIN_SOFA};

type Patch = (&'static str, &'static [(usize, u8)]);

const CASES: &[Patch] = &[
    (
        "rate_1hz",
        &[
            (472346, 0x0d),
            (1173135, 0x00),
            (1173136, 0x81),
            (1173137, 0x0f),
            (1173138, 0xf6),
            (1173139, 0x00),
            (1173140, 0x02),
            (1173141, 0x27),
            (1173142, 0x01),
            (1173143, 0x30),
        ],
    ),
    (
        "rate_zero",
        &[
            (472346, 0x0b),
            (1173135, 0x80),
            (1173136, 0x00),
            (1173137, 0x00),
            (1173138, 0x00),
            (1173139, 0x08),
            (1173140, 0x00),
            (1173141, 0x01),
        ],
    ),
    (
        "rate_nan",
        &[
            (472346, 0x0d),
            (1173135, 0x00),
            (1173136, 0x81),
            (1173137, 0x1f),
            (1173138, 0xf5),
            (1173139, 0x00),
            (1173140, 0x02),
            (1173141, 0x77),
            (1173142, 0x01),
            (1173143, 0x78),
        ],
    ),
    (
        "rate_negative",
        &[
            (1173140, 0x7a),
            (1173143, 0x06),
            (1173144, 0x2a),
            (1173146, 0xae),
        ],
    ),
    (
        "rate_huge",
        &[
            (1173136, 0x58),
            (1173137, 0x34),
            (1173138, 0x45),
            (1173139, 0x2a),
            (1173140, 0xd7),
            (1173141, 0x09),
            (1173143, 0x06),
            (1173144, 0xec),
            (1173146, 0x00),
        ],
    ),
];

fn patched(p: &[(usize, u8)]) -> Arc<[u8]> {
    let mut b = BUILTIN_SOFA.to_vec();
    for &(i, v) in p {
        b[i] = v;
    }
    b.into()
}

/// Child side: load one case and print what happened.
#[test]
fn child_case() {
    let Ok(name) = std::env::var("ATTACK_SOFA_CASE") else {
        return;
    };
    let (_, p) = CASES.iter().find(|c| c.0 == name).expect("case");
    let t = Instant::now();
    let r = HrtfSet::load(&HrtfSource::Sofa(patched(p)), 48_000);
    println!(
        "RESULT {name}: {:?} in {:?}",
        r.map(|_| "loaded"),
        t.elapsed()
    );
}

/// A hostile file must give an error (or a usable set) quickly, never abort
/// the process or eat all memory.
#[test]
fn hostile_sampling_rates_do_not_kill_the_process() {
    if std::env::var("ATTACK_SOFA_CASE").is_ok() {
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let mut bad = Vec::new();
    for (name, _) in CASES {
        let t = Instant::now();
        let out = Command::new(&exe)
            .args(["child_case", "--exact", "--nocapture", "--test-threads=1"])
            .env("ATTACK_SOFA_CASE", name)
            .output()
            .unwrap();
        let e = t.elapsed();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let line = stdout
            .lines()
            .find_map(|l| l.find("RESULT").map(|i| &l[i..]))
            .unwrap_or("");
        eprintln!("{name}: status {:?} {e:?} {line}", out.status.code());
        if !out.status.success() || line.is_empty() || e > Duration::from_secs(10) {
            let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
            bad.push(format!("{name}: {:?} {e:?} {tail}", out.status));
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}
