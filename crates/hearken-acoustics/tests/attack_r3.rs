//! Attack round 3 repros. Run in release:
//! `cargo test --release -p hearken-acoustics --test attack_r3 -- --nocapture --test-threads=1`.

use hearken_acoustics::*;
use std::time::{Duration, Instant};

fn time<T>(label: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let r = f();
    let e = t.elapsed();
    eprintln!("{label}: {e:?}");
    (r, e)
}

/// Two rows of thin poles 1 m apart with a lane between, a wall across
/// the lane, and the poles added in a scattered order so the path
/// search's corners spread along the whole 1 km lane.
fn lane_with_wall(wall_len: f64) -> Scene {
    let mut b = SceneBuilder::new();
    b.add(
        Shape::wall_between(500.05, -wall_len / 2.0, 500.05, wall_len / 2.0, 0.0, 3.0),
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
    b.build()
}

/// Currently fails: with default options one propagate spends its whole
/// 1M shape-test allowance (~65 ms in release) and still finds nothing.
/// hearken-audio works out at least one such sound every frame.
#[test]
fn default_query_allowance_is_several_frames() {
    let s = lane_with_wall(3.0);
    let a = Vec3::new(-5.0, 0.0, 1.5);
    let l = Vec3::new(1005.0, 0.0, 1.5);
    let (p, e) = time("propagate, default options", || {
        propagate(&s, a, l, &QueryOptions::default())
    });
    eprintln!(
        "charged {} truncated {} path found {}",
        p.shape_tests,
        p.truncated,
        p.diffracted.is_some()
    );
    assert!(e < Duration::from_millis(16), "{e:?}");
}

/// A scene file near the 16 MB cap: many materials whose names share 63
/// characters, and as many objects as fit, each naming the last material.
fn many_names_json(materials: usize) -> String {
    let prefix = "x".repeat(63);
    let mut s = String::from("{\"materials\":[");
    for i in 0..materials {
        if i > 0 {
            s.push(',');
        }
        // The last character tells them apart.
        let c = char::from_u32(0x4e00 + i as u32).unwrap();
        s.push_str(&format!(
            "{{\"name\":\"{prefix}{c}\",\"absorption\":[0,0,0,0,0,0,0]}}"
        ));
    }
    s.push_str("],\"objects\":[");
    let last = char::from_u32(0x4e00 + materials as u32 - 1).unwrap();
    let obj = format!("{{\"kind\":\"ground\",\"z\":0,\"material\":\"{prefix}{last}\"}}");
    let mut first = true;
    while s.len() + obj.len() + 4 < 16 * 1024 * 1024 {
        if !first {
            s.push(',');
        }
        first = false;
        s.push_str(&obj);
    }
    s.push_str("]}");
    s
}

/// Currently fails: each object looks its material up by a linear scan of
/// every material name. With the highest limits a game may set (4096
/// materials, 100 000 objects) a 16 MB file takes seconds to load, and the
/// objects past the object cap are still looked up before being dropped.
#[test]
fn material_lookup_is_linear_per_object() {
    let limits = Limits {
        max_objects: usize::MAX,
        max_materials: usize::MAX,
        max_extent_m: 1e6,
    };
    let text = many_names_json(4000);
    eprintln!("file {} bytes", text.len());
    let (r, e) = time("from_json, 4000 materials, highest limits", || {
        Scene::from_json(&text, limits).unwrap()
    });
    eprintln!(
        "objects kept {}, dropped {}",
        r.0.objects().len(),
        r.1.dropped_count
    );
    let text = many_names_json(236);
    let (_, e2) = time("from_json, 236 materials, default limits", || {
        Scene::from_json(&text, Limits::default()).unwrap()
    });
    let _ = e2;
    assert!(e < Duration::from_millis(500), "{e:?}");
}

/// 20 000 small boxes in a 20 m square below the floor, inside the
/// reflection radius but out of every line between source and listener.
fn boxes_below() -> Scene {
    let mut b = SceneBuilder::new();
    for i in 0..20_000 {
        let x = (i % 141) as f64 * 0.14 - 10.0;
        let y = (i / 141) as f64 * 0.14 - 10.0;
        b.add(
            Shape::cuboid(Vec3::new(x, y, -5.0), Vec3::new(x + 0.1, y + 0.1, -4.9)),
            Builtin::Brick.id(),
        )
        .unwrap();
    }
    b.build()
}

/// Currently fails: the reflection search gathers every face within the
/// reflection radius (6 per box) and sorts them all before keeping the
/// nearest 48, and none of it is charged as shape tests. Each propagate
/// costs several ms while reporting a few hundred tests, so
/// hearken-audio's frame allowance lets many such sounds run in a frame.
#[test]
fn reflection_face_gathering_is_not_charged() {
    let s = boxes_below();
    let a = Vec3::new(-3.0, 0.0, 1.5);
    let l = Vec3::new(3.0, 0.0, 1.5);
    let opts = QueryOptions::default();
    let _ = propagate(&s, a, l, &opts);
    let n = 16;
    let t = Instant::now();
    let mut charged = 0;
    for k in 0..n {
        let src = Vec3::new(-3.0, k as f64 * 0.1, 1.5);
        charged += propagate(&s, src, l, &opts).shape_tests;
    }
    let e = t.elapsed();
    eprintln!("16 propagates: {e:?}, {charged} shape tests charged");
    // At the ~70 ns a charged test costs elsewhere, the charge would
    // allow about this long:
    let fair = Duration::from_nanos(70 * charged as u64) + Duration::from_millis(1);
    assert!(e < fair * 4, "{e:?} for {charged} charged tests");
}
