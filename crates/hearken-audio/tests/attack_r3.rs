//! Attack round 3 repros for the rendering side. Run in release:
//! `cargo test --release -p hearken-audio --test attack_r3 -- --nocapture --test-threads=1`.

#![cfg(feature = "kira")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hearken_audio::acoustics::{Builtin, Scene, SceneBuilder, Shape};
use hearken_audio::{AcousticSettings, Audio, Bus, Feed, FeedRead, KiraBackend, LooseClips, Vec3};

const RATE: u32 = 48_000;

struct Silence(Mutex<()>);

impl Feed for Silence {
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead {
        let _g = self.0.lock().unwrap();
        for f in out.iter_mut() {
            *f = [0.0, 0.0];
        }
        FeedRead {
            frames: out.len(),
            sample_rate: RATE,
        }
    }
}

/// The round 2 frame-cost scene: a wall and 19 999 poles between the
/// listener and the sounds.
fn pole_stack() -> Arc<Scene> {
    let mut b = SceneBuilder::new();
    b.add(
        Shape::wall_between(5.0, -3.0, 5.0, 3.0, 0.0, 3.0),
        Builtin::Concrete.id(),
    )
    .unwrap();
    for i in 0..19_999 {
        let y = -2.0 + (i % 400) as f64 * 0.01;
        b.add(Shape::pole(5.5, y, 0.0, 0.2, 3.0), Builtin::Wood.id())
            .unwrap();
    }
    Arc::new(b.build())
}

fn audio_in(scene: &Arc<Scene>) -> Audio {
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(AcousticSettings::default()));
    audio.set_listener(Vec3::new(10.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(scene);
    audio
}

/// Currently fails: a newly started sound runs its full query at once,
/// outside the frame allowance (its cost is only charged to the next
/// frame). Starting 16 sounds in one frame (a burst of footsteps, an
/// explosion's debris) runs 16 full queries in that frame.
#[test]
fn starting_many_sounds_skips_the_frame_allowance() {
    let scene = pole_stack();
    let mut audio = audio_in(&scene);
    let t = Instant::now();
    for k in 0..16 {
        let y = -1.5 + k as f32 * 0.2;
        audio.start_feed(
            Arc::new(Silence(Mutex::new(()))),
            Some(Vec3::new(0.0, y, 1.5)),
            Bus::EFFECTS,
        );
    }
    audio.update_acoustic(&scene);
    let e = t.elapsed();
    eprintln!("16 starts + one update: {e:?}");
    assert!(e < Duration::from_millis(50), "{e:?}");
}

/// Currently fails: turning on the spot changes no path, but the cache
/// key holds the listener's facing, so every turning frame re-runs every
/// sound's query and spends the whole frame allowance.
#[test]
fn turning_on_the_spot_reruns_every_query() {
    let scene = pole_stack();
    let mut audio = audio_in(&scene);
    for k in 0..16 {
        let y = -1.5 + k as f32 * 0.2;
        audio.start_feed(
            Arc::new(Silence(Mutex::new(()))),
            Some(Vec3::new(0.0, y, 1.5)),
            Bus::EFFECTS,
        );
    }
    // Let every sound settle.
    for _ in 0..40 {
        audio.update_acoustic(&scene);
        let _ = audio.render(800).unwrap();
    }
    let t = Instant::now();
    for _ in 0..10 {
        audio.update_acoustic(&scene);
        let _ = audio.render(800).unwrap();
    }
    let still = t.elapsed() / 10;
    let t = Instant::now();
    for f in 0..10 {
        audio.set_listener(Vec3::new(10.0, 0.0, 1.5), f as f32 * 3.0);
        audio.update_acoustic(&scene);
        let _ = audio.render(800).unwrap();
    }
    let turning = t.elapsed() / 10;
    eprintln!("still frame {still:?}, turning frame {turning:?}");
    assert!(
        turning < still * 4 + Duration::from_millis(2),
        "{turning:?}"
    );
}

/// Two rows of thin poles with a lane between, a wall across the lane,
/// and the poles added in a scattered order so the path search's corners
/// spread along the whole lane.
fn lane_with_wall() -> Arc<Scene> {
    let mut b = SceneBuilder::new();
    b.add(
        Shape::wall_between(500.05, -1.5, 500.05, 1.5, 0.0, 3.0),
        Builtin::Concrete.id(),
    )
    .unwrap();
    for i in 0..9_999u64 {
        let x = ((i * 7919) % 9_999) as f64 * 0.1;
        b.add(Shape::pole(x, 0.5, 0.0, 0.01, 3.0), Builtin::Wood.id())
            .unwrap();
        b.add(Shape::pole(x, -0.5, 0.0, 0.01, 3.0), Builtin::Wood.id())
            .unwrap();
    }
    Arc::new(b.build())
}

/// Currently fails: the frame allowance (500 000) is smaller than one
/// query's own cap (1 000 000), and one sound is always worked out, so a
/// single sound overruns the allowance by 2x and takes ~70 ms a frame
/// while the listener walks.
#[test]
fn one_query_overruns_the_frame_allowance() {
    let scene = lane_with_wall();
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(AcousticSettings::default()));
    audio.set_listener(Vec3::new(1005.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    audio.start_feed(
        Arc::new(Silence(Mutex::new(()))),
        Some(Vec3::new(-5.0, 0.0, 1.5)),
        Bus::EFFECTS,
    );
    audio.update_acoustic(&scene);
    let mut worst = Duration::ZERO;
    for f in 1..6 {
        audio.set_listener(Vec3::new(1005.0 + f as f32 * 0.01, 0.0, 1.5), 0.0);
        let t = Instant::now();
        audio.update_acoustic(&scene);
        let e = t.elapsed();
        eprintln!("walking frame {f}: {e:?}");
        worst = worst.max(e);
        let _ = audio.render(800).unwrap();
    }
    assert!(worst < Duration::from_millis(33), "worst frame {worst:?}");
}

/// Currently fails: 20 000 small boxes below the floor, inside the
/// reflection radius. The reflection search gathers and sorts all their
/// faces for every sound without charging it, so the frame allowance
/// sees ~1 test per sound and works out all 16 sounds every walking
/// frame (~430 ms).
#[test]
fn uncharged_reflection_search_defeats_frame_allowance() {
    let mut b = SceneBuilder::new();
    for i in 0..20_000 {
        let x = (i % 141) as f64 * 0.14 - 10.0;
        let y = (i / 141) as f64 * 0.14 - 10.0;
        b.add(
            Shape::cuboid(
                hearken_audio::acoustics::Vec3::new(x, y, -5.0),
                hearken_audio::acoustics::Vec3::new(x + 0.1, y + 0.1, -4.9),
            ),
            Builtin::Brick.id(),
        )
        .unwrap();
    }
    let scene = Arc::new(b.build());
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(AcousticSettings {
        room: false,
        ..AcousticSettings::default()
    }));
    audio.set_listener(Vec3::new(3.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    for k in 0..16 {
        audio.start_feed(
            Arc::new(Silence(Mutex::new(()))),
            Some(Vec3::new(-3.0, k as f32 * 0.1, 1.5)),
            Bus::EFFECTS,
        );
    }
    audio.update_acoustic(&scene);
    let mut worst = Duration::ZERO;
    for f in 1..4 {
        audio.set_listener(Vec3::new(3.0 + f as f32 * 0.01, 0.0, 1.5), 0.0);
        let t = Instant::now();
        audio.update_acoustic(&scene);
        let e = t.elapsed();
        eprintln!("walking frame {f}: {e:?}");
        worst = worst.max(e);
        let _ = audio.render(800).unwrap();
    }
    assert!(worst < Duration::from_millis(33), "worst frame {worst:?}");
}
