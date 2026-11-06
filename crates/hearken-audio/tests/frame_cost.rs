//! Game-thread cost of `Audio::update_acoustic` in hostile scenes. The
//! timing checks are meant for release builds:
//! `cargo test --release -p hearken-audio --test frame_cost -- --nocapture`.

#![cfg(feature = "kira")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hearken_audio::acoustics::{Builtin, SceneBuilder, Shape};
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

/// The round 1 pole stack (a wall and 20 000 poles at one spot) between
/// the listener and 16 sounds. The listener walks 1 cm a frame, so every
/// sound needs working out again; the frame allowance spreads them over
/// frames.
#[test]
fn many_sounds_behind_a_pole_stack_fit_a_frame() {
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
    let scene = Arc::new(b.build());
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(AcousticSettings::default()));
    audio.set_listener(Vec3::new(10.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    for k in 0..16 {
        let y = -1.5 + k as f32 * 0.2;
        audio.start_feed(
            Arc::new(Silence(Mutex::new(()))),
            Some(Vec3::new(0.0, y, 1.5)),
            Bus::EFFECTS,
        );
    }
    let mut worst = Duration::ZERO;
    for f in 0..5 {
        audio.set_listener(Vec3::new(10.0 + f as f32 * 0.01, 0.0, 1.5), 0.0);
        let t = Instant::now();
        audio.update_acoustic(&scene);
        let e = t.elapsed();
        eprintln!("frame {f}: {e:?}");
        worst = worst.max(e);
        let _ = audio.render(800).unwrap();
    }
    assert!(worst < Duration::from_millis(50), "worst frame {worst:?}");
}

/// Obstacle sense does not query again while the listener stands still.
#[test]
fn obstacle_sense_each_frame_in_a_pole_stack() {
    let mut b = SceneBuilder::new();
    for i in 0..20_000 {
        let x = 2.0 + (i % 10) as f64 * 0.003;
        let y = 2.0 + ((i / 10) % 10) as f64 * 0.003;
        b.add(Shape::pole(x, y, 0.0, 0.001, 3.0), Builtin::Wood.id())
            .unwrap();
    }
    let scene = Arc::new(b.build());
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(AcousticSettings {
        obstacle_sense: 1.0,
        room: false,
        ..AcousticSettings::default()
    }));
    audio.set_listener(Vec3::new(2.3, 2.2, 1.5), 0.0);
    audio.update_acoustic(&scene);
    let mut worst = Duration::ZERO;
    for f in 0..10 {
        let t = Instant::now();
        audio.update_acoustic(&scene);
        let e = t.elapsed();
        if f < 3 {
            eprintln!("still frame {f}: {e:?}");
        }
        worst = worst.max(e);
        let _ = audio.render(800).unwrap();
    }
    assert!(
        worst < Duration::from_millis(4),
        "worst still frame {worst:?}"
    );
}

/// A negative `room_move_m` is clamped: unchecked, it re-estimated the
/// room every frame and, through the generation bump, re-ran every sound's
/// query though nothing moved.
#[test]
fn negative_room_move_is_clamped() {
    let mut b = SceneBuilder::new();
    let (w, d, h) = (6.0, 5.0, 3.0);
    for (x0, y0, x1, y1) in [
        (0.0, 0.0, w, 0.0),
        (w, 0.0, w, d),
        (w, d, 0.0, d),
        (0.0, d, 0.0, 0.0),
    ] {
        b.add(
            Shape::wall_between(x0, y0, x1, y1, 0.0, h),
            Builtin::Plaster.id(),
        )
        .unwrap();
    }
    b.add(Shape::Ground { z: 0.0 }, Builtin::Wood.id()).unwrap();
    b.add(Shape::Ceiling { z: h }, Builtin::Plaster.id())
        .unwrap();
    let scene = Arc::new(b.build());
    let still_frame = |room_move_m: f64| {
        let mut audio = Audio::new(
            Box::new(KiraBackend::offline(RATE).unwrap()),
            Box::new(LooseClips::none()),
        );
        audio.set_acoustics(Some(AcousticSettings {
            room_move_m,
            ..AcousticSettings::default()
        }));
        audio.set_listener(Vec3::new(2.0, 2.0, 1.5), 0.0);
        audio.update_acoustic(&scene);
        for k in 0..16 {
            audio.start_feed(
                Arc::new(Silence(Mutex::new(()))),
                Some(Vec3::new(1.0 + k as f32 * 0.25, 4.0, 1.5)),
                Bus::EFFECTS,
            );
        }
        audio.update_acoustic(&scene);
        let t = Instant::now();
        for _ in 0..20 {
            audio.update_acoustic(&scene);
        }
        t.elapsed() / 20
    };
    let normal = still_frame(0.5);
    let negative = still_frame(-1.0);
    eprintln!("still frame: room_move_m 0.5 {normal:?}, -1 {negative:?}");
    assert!(
        negative < normal * 5 + Duration::from_micros(100),
        "{normal:?} -> {negative:?}"
    );
}
