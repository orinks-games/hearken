//! Attack round 1: hostile scenes and inputs. Run with
//! `cargo test --release -p hearken-acoustics --test attack_r1 -- --nocapture`.
//! Tests marked FAILS fail on the build they were written against.

use hearken_acoustics::*;
use std::time::{Duration, Instant};

fn time<T>(label: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let r = f();
    let e = t.elapsed();
    eprintln!("{label}: {e:?}");
    (r, e)
}

/// A per-frame query from a scene file should stay well under a frame.
const FRAME_BUDGET: Duration = Duration::from_millis(50);

/// Nested boxes round the source: every corner of an inner box lies inside
/// an outer one, so it is skipped without counting against the node cap.
/// Each skip costs a grid query over the "always" list, so the node phase
/// is quadratic in the object count with default options.
/// FAILS: one diffracted_path call takes seconds.
#[test]
fn nested_boxes_make_path_search_quadratic() {
    let mut b = SceneBuilder::new();
    let n = 20_000;
    for i in 0..n {
        let h = 50.0 + i as f64 * 0.02;
        b.add(
            Shape::cuboid(Vec3::new(-h, -h, 0.0), Vec3::new(h, h, 3.0)),
            Builtin::Brick.id(),
        )
        .unwrap();
    }
    let s = b.build();
    let src = Vec3::new(0.0, 0.0, 1.5);
    let lis = Vec3::new(10.0, 0.0, 1.5);
    let (_, e) = time("nested boxes diffracted_path", || {
        diffracted_path(&s, src, lis, &QueryOptions::default())
    });
    assert!(e < FRAME_BUDGET, "{e:?}");
}

/// Many identical small poles in one spot: every visibility test walks all
/// of them. Bounded by the caps but far over a frame with default options.
/// FAILS.
#[test]
fn stacked_poles_make_each_visibility_test_cost_n() {
    let mut b = SceneBuilder::new();
    // A wall between the ends so the path search runs.
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
    let s = b.build();
    let (p, e) = time("stacked poles propagate", || {
        propagate(
            &s,
            Vec3::new(0.0, 0.0, 1.5),
            Vec3::new(10.0, 0.0, 1.5),
            &QueryOptions::default(),
        )
    });
    eprintln!("truncated {}", p.truncated);
    assert!(e < FRAME_BUDGET, "{e:?}");
}

/// Long walls far away each span more than 64 grid cells, so they all go
/// in the list every query checks, and every line-of-sight test copies and
/// sorts that list. A pole field between the ends makes the path search
/// run its default 20 000 tests. Default options.
/// FAILS: one propagate call takes seconds.
#[test]
fn large_shapes_defeat_the_grid() {
    let mut b = SceneBuilder::new();
    for i in 0..19_000 {
        let y = 5000.0 + i as f64 * 0.1;
        b.add(
            Shape::wall_between(-1500.0, y, 1500.0, y, 0.0, 3.0),
            Builtin::Brick.id(),
        )
        .unwrap();
    }
    for i in 0..400 {
        let x = (i % 20) as f64 * 3.0;
        let y = (i / 20) as f64 * 3.0;
        b.add(Shape::pole(x, y, 0.0, 0.5, 3.0), Builtin::Wood.id())
            .unwrap();
    }
    let s = b.build();
    let (p, e) = time("far long walls + pole field propagate", || {
        propagate(
            &s,
            Vec3::new(-2.0, 0.0, 1.0),
            Vec3::new(60.0, 0.0, 1.0),
            &QueryOptions::default(),
        )
    });
    eprintln!("truncated {}", p.truncated);
    let (_, e2) = time("same scene estimate_reverb", || {
        estimate_reverb(&s, Vec3::new(30.0, 30.0, 1.5), &ReverbOptions::default())
    });
    assert!(e < FRAME_BUDGET && e2 < FRAME_BUDGET, "{e:?} {e2:?}");
}

/// Boxes crowded round the listener fill the 48 nearest faces, and the
/// second-order search tries every pair with three blocked tests each.
#[test]
fn crowded_faces_reflection_cost() {
    let mut b = SceneBuilder::new();
    for i in 0..200 {
        let x = (i % 20) as f64 * 1.5 - 15.0;
        let y = (i / 20) as f64 * 1.5 - 7.5;
        if x.abs() < 1.0 && y.abs() < 1.0 {
            continue;
        }
        b.add(
            Shape::cuboid(Vec3::new(x, y, 0.0), Vec3::new(x + 0.5, y + 0.5, 3.0)),
            Builtin::Tile.id(),
        )
        .unwrap();
    }
    b.add(Shape::Ground { z: 0.0 }, Builtin::Tile.id()).unwrap();
    b.add(Shape::Ceiling { z: 3.0 }, Builtin::Tile.id())
        .unwrap();
    let s = b.build();
    let max = QueryOptions {
        max_reflection_faces: usize::MAX,
        max_reflections: usize::MAX,
        ..QueryOptions::default()
    };
    let (_, e) = time("crowded faces, max reflection options", || {
        early_reflections(
            &s,
            Vec3::new(0.2, 0.3, 1.5),
            Vec3::new(-0.3, 0.1, 1.6),
            &max,
        )
    });
    assert!(e < Duration::from_secs(2), "{e:?}");
}

/// Material names are compared one by one for each entry, so a scene file
/// with many materials is quadratic. With the hard cap of 4096 materials
/// and 16 MB of JSON.
#[test]
fn many_materials_load_time() {
    let mut text = String::from("{\"materials\":[");
    let mut i = 0;
    while text.len() < 15 * 1024 * 1024 {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&format!(
            "{{\"name\":\"m{:060}\",\"absorption\":[0,0,0,0,0,0,0]}}",
            i % 5000
        ));
        i += 1;
    }
    text.push_str("]}");
    let limits = Limits {
        max_materials: usize::MAX,
        ..Limits::default()
    };
    let (r, e) = time("16 MB of materials", || Scene::from_json(&text, limits));
    assert!(r.is_ok());
    assert!(e < Duration::from_secs(5), "{e:?}");
}

/// Same scene and query twice gives the same bits (render included).
#[test]
fn render_is_repeatable() {
    let mut b = SceneBuilder::new();
    for i in 0..50 {
        let x = (i % 10) as f64 * 3.0;
        let y = (i / 10) as f64 * 3.0;
        b.add(Shape::pole(x, y, 0.0, 0.3, 3.0), Builtin::Wood.id())
            .unwrap();
        b.add(
            Shape::wall_between(x + 1.0, y, x + 1.0, y + 2.0, 0.0, 3.0),
            Builtin::Brick.id(),
        )
        .unwrap();
    }
    b.add(Shape::Ground { z: 0.0 }, Builtin::Grass.id())
        .unwrap();
    let s = b.build();
    let run = || {
        let p = propagate(
            &s,
            Vec3::new(-2.0, 1.0, 1.5),
            Vec3::new(31.0, 13.0, 1.6),
            &QueryOptions::default(),
        );
        let rv = estimate_reverb(&s, Vec3::new(12.0, 7.0, 1.6), &ReverbOptions::default());
        format!("{:?}{:?}", p.render(33.0, Some(&rv)), rv)
    };
    assert_eq!(run(), run());
}

/// ISO 9613-2 Table 2 values (20 C, 70 % RH, dB/km).
#[test]
fn iso_9613_table_2_20c_70rh() {
    let a = Atmosphere::new(20.0, 70.0, 101.325).absorption_bands_db_per_m();
    let want = [0.3, 1.1, 2.8, 5.0, 9.0, 22.9, 76.6];
    for (got, want) in a.0.iter().zip(want) {
        let got = got * 1000.0;
        assert!((got - want).abs() <= 0.06 * want + 0.05, "{got} vs {want}");
    }
}

/// A doorway in a wall: sound from behind the wall should come from the
/// doorway side, and the bent path should be found with default options.
#[test]
fn doorway_arrival_is_found() {
    let mut b = SceneBuilder::new();
    let c = Builtin::Concrete.id();
    b.add(Shape::wall_between(5.0, -50.0, 5.0, 2.0, 0.0, 3.0), c)
        .unwrap();
    b.add(Shape::wall_between(5.0, 3.0, 5.0, 50.0, 0.0, 3.0), c)
        .unwrap();
    let s = b.build();
    let p = propagate(
        &s,
        Vec3::new(8.0, -4.0, 1.5),
        Vec3::new(2.0, -4.0, 1.5),
        &QueryOptions::default(),
    );
    let r = p.render(0.0, None);
    assert!(p.diffracted.is_some());
    // Facing north, the doorway (y 2..3) is ahead and to the right.
    assert!(r.direction[1] > 0.0, "{:?}", r.direction);
}

/// The reverb estimate from inside a closed concrete box should not make
/// wet gain or RT60 non-finite, and Sabine >= Eyring.
#[test]
fn sealed_reflective_room_reverb_is_sane() {
    let mut b = SceneBuilder::new();
    let mut m = Builtin::Concrete.material();
    m.name = "mirror".into();
    m.absorption = Bands::ZERO;
    let id = b.add_material(&m).unwrap();
    b.add(
        Shape::cuboid(Vec3::new(-0.01, -0.01, -0.01), Vec3::new(0.01, 0.01, 0.01)),
        id,
    )
    .unwrap();
    for (a, c) in [
        (Vec3::new(-5.0, -5.0, -1.0), Vec3::new(5.0, 5.0, 0.0)),
        (Vec3::new(-5.0, -5.0, 3.0), Vec3::new(5.0, 5.0, 4.0)),
        (Vec3::new(-6.0, -5.0, 0.0), Vec3::new(-5.0, 5.0, 3.0)),
        (Vec3::new(5.0, -5.0, 0.0), Vec3::new(6.0, 5.0, 3.0)),
        (Vec3::new(-5.0, -6.0, 0.0), Vec3::new(5.0, -5.0, 3.0)),
        (Vec3::new(-5.0, 5.0, 0.0), Vec3::new(5.0, 6.0, 3.0)),
    ] {
        b.add(Shape::cuboid(a, c), id).unwrap();
    }
    let s = b.build();
    let r = estimate_reverb(&s, Vec3::new(1.0, 1.0, 1.5), &ReverbOptions::default());
    for v in r.rt60_eyring.0.iter().chain(&r.wet_gain.0) {
        assert!(v.is_finite() && *v >= 0.0, "{r:?}");
    }
    for (s, e) in r.rt60_sabine.0.iter().zip(&r.rt60_eyring.0) {
        assert!(s >= e, "{r:?}");
    }
}

/// A plain field of 400 poles, nothing else: a realistic forest. The path
/// search spends its whole default test budget when no way round exists
/// near the line. Prints the time; fails over a frame budget.
#[test]
fn pole_field_alone_cost() {
    let mut b = SceneBuilder::new();
    for i in 0..400 {
        let x = (i % 20) as f64 * 3.0;
        let y = (i / 20) as f64 * 3.0;
        b.add(Shape::pole(x, y, 0.0, 0.5, 3.0), Builtin::Wood.id())
            .unwrap();
    }
    let s = b.build();
    let (p, e) = time("400-pole field propagate", || {
        propagate(
            &s,
            Vec3::new(-2.0, 0.0, 1.0),
            Vec3::new(60.0, 0.0, 1.0),
            &QueryOptions::default(),
        )
    });
    eprintln!(
        "truncated {} diffracted {}",
        p.truncated,
        p.diffracted.is_some()
    );
    assert!(e < FRAME_BUDGET, "{e:?}");
}

/// Peak resident memory of this process, kB (Linux only).
fn peak_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// An object's shape is `#[serde(flatten)]`, so serde buffers every field
/// of the object, unknown ones included, as generic values before picking
/// the shape. Junk in an object costs about 16 bytes of buffer for each
/// 2 bytes of text. Under the 16 MB cap that is a few hundred MB.
/// Run alone for a clean number:
/// `cargo test --release -p hearken-acoustics --test attack_r1 flatten -- --nocapture --test-threads=1`
/// FAILS when run alone: peak memory goes past 20 times the file size
/// (other tests in the same process raise the starting peak and hide it).
#[test]
fn flatten_buffers_unknown_fields() {
    let mut text =
        String::from(r#"{"objects":[{"kind":"ground","z":0,"material":"grass","junk":["#);
    while text.len() < 15 * 1024 * 1024 {
        text.push_str("[],");
    }
    text.push_str("[]]}]}");
    let before = peak_kb();
    let r = Scene::from_json(&text, Limits::default());
    let after = peak_kb();
    eprintln!(
        "ok {} peak before {} MB after {} MB, text {} MB",
        r.is_ok(),
        before / 1024,
        after / 1024,
        text.len() / (1024 * 1024)
    );
    assert!(after.saturating_sub(before) < 20 * 16 * 1024, "{after} kB");
}
