//! Same inputs, same bits. The pinned values below were computed on
//! x86_64 Linux; a different result on any other target means the maths is
//! not portable and must be fixed. Re-pin only when a change to the model
//! is meant to change results.
//!
//! The scene and hash live in `examples/golden.rs`, which also builds as a
//! WebAssembly module so wasm32 can be checked against the same value:
//!
//! ```text
//! cargo build -p hearken-acoustics --example golden --target wasm32-unknown-unknown --release
//! node -e "const fs=require('fs');WebAssembly.instantiate(fs.readFileSync('target/wasm32-unknown-unknown/release/examples/golden.wasm')).then(m=>console.log('0x'+BigInt.asUintN(64,m.instance.exports.golden_hash()).toString(16)))"
//! ```

#[path = "../examples/golden.rs"]
mod golden;

use golden::{run_all, scene};
use hearken_acoustics::*;

#[test]
fn identical_runs_give_identical_bits() {
    let s = scene();
    assert_eq!(run_all(&s), run_all(&s));
    assert_eq!(run_all(&s), run_all(&scene()));
}

#[test]
fn json_scene_matches_built_scene() {
    let built = scene();
    let desc = SceneDesc {
        atmosphere: built.atmosphere(),
        materials: Vec::new(),
        objects: built
            .objects()
            .iter()
            .map(|o| ObjectDesc {
                shape: o.shape,
                material: built.material(o.material).unwrap().name.clone(),
            })
            .collect(),
    };
    let text = serde_json::to_string(&desc).unwrap();
    let (loaded, report) = Scene::from_json(&text, Limits::default()).unwrap();
    assert!(report.is_clean());
    assert_eq!(run_all(&loaded), run_all(&built));
}

#[test]
fn golden_physics_values() {
    let dry = Atmosphere::new(20.0, 0.0, 101.325);
    assert_eq!(dry.speed_of_sound().to_bits(), GOLDEN_SPEED_DRY_20C);
    let iso = Atmosphere::new(20.0, 70.0, 101.325);
    assert_eq!(iso.absorption_db_per_m(1000.0).to_bits(), GOLDEN_ALPHA_1K);
    assert_eq!(
        barrier_attenuation_db(0.7, 1, 0.0, 343.0).0[4].to_bits(),
        GOLDEN_BARRIER_2K
    );
    let dirs = ray_directions(256);
    assert_eq!(dirs[17].x.to_bits(), GOLDEN_RAY_17_X);
}

#[test]
fn golden_scene_hash() {
    assert_eq!(
        run_all(&scene()),
        GOLDEN_SCENE_HASH,
        "{:#x}",
        run_all(&scene())
    );
}

const GOLDEN_SPEED_DRY_20C: u64 = 0x407575c06052067c;
const GOLDEN_ALPHA_1K: u64 = 0x3f74639cecb5ff2e;
const GOLDEN_BARRIER_2K: u64 = 0x403343f4361efc85;
const GOLDEN_RAY_17_X: u64 = 0xbfe023295483fb25;
const GOLDEN_SCENE_HASH: u64 = 0xd50085c5795aff1f;
