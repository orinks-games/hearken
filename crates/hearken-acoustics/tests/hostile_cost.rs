//! Hostile scenes and files: queries stay cheap and files read back
//! exactly. The timing checks are meant for release builds:
//! `cargo test --release -p hearken-acoustics --test hostile_cost -- --nocapture --test-threads=1`.

use hearken_acoustics::*;
use std::time::{Duration, Instant};

fn time<T>(label: &str, f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let r = f();
    let e = t.elapsed();
    eprintln!("{label}: {e:?}");
    (r, e)
}

/// A query a game may run from the frame loop should stay well under a
/// frame.
const FRAME_BUDGET: Duration = Duration::from_millis(50);

/// 20 000 thin poles stacked on one spot, the listener 0.3 m away, so
/// every ray and line from the listener lands in the cell that holds them
/// all.
fn stacked_poles() -> Scene {
    let mut b = SceneBuilder::new();
    for i in 0..20_000 {
        let x = 2.0 + (i % 10) as f64 * 0.003;
        let y = 2.0 + ((i / 10) % 10) as f64 * 0.003;
        b.add(Shape::pole(x, y, 0.0, 0.001, 3.0), Builtin::Wood.id())
            .unwrap();
    }
    b.build()
}

const LISTENER: Vec3 = Vec3 {
    x: 2.3,
    y: 2.2,
    z: 1.5,
};

/// The reverb estimate with the most rays the options allow (4096), next
/// to a pole stack. hearken-audio runs it on the game thread whenever the
/// listener moves `room_move_m`.
#[test]
fn reverb_max_rays_in_stacked_poles() {
    let s = stacked_poles();
    let opts = ReverbOptions {
        rays: usize::MAX,
        ..ReverbOptions::default()
    };
    let (_, e) = time("estimate_reverb 4096 rays", || {
        estimate_reverb(&s, LISTENER, &opts)
    });
    assert!(e < FRAME_BUDGET, "{e:?}");
}

/// The settings hearken-audio's obstacle sense uses (radius 4, 32
/// surfaces), next to a pole stack.
#[test]
fn nearby_surfaces_sense_settings_in_stacked_poles() {
    let s = stacked_poles();
    let opts = NearbyOptions {
        radius: 4.0,
        max: 32,
        include_planes: false,
    };
    let (_, e) = time("nearby_surfaces radius 4 max 32", || {
        nearby_surfaces(&s, LISTENER, &opts)
    });
    assert!(e < Duration::from_millis(5), "{e:?}");
}

/// navigate past a wall and a pole stack, with default options and at
/// the option caps. The retry without clearance shares the allowance.
#[test]
fn navigate_in_stacked_poles() {
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
    let s = b.build();
    let (_, e) = time("navigate default", || {
        navigate(
            &s,
            Vec3::new(0.0, 0.0, 1.5),
            Vec3::new(10.0, 0.0, 1.5),
            &NavOptions::default(),
        )
    });
    let opts = NavOptions {
        max_nodes: usize::MAX,
        max_visibility_tests: usize::MAX,
        max_shape_tests: usize::MAX,
        ..NavOptions::default()
    };
    let (_, e2) = time("navigate max options", || {
        navigate(
            &s,
            Vec3::new(0.0, 0.0, 1.5),
            Vec3::new(10.0, 0.0, 1.5),
            &opts,
        )
    });
    assert!(e < FRAME_BUDGET, "{e:?}");
    assert!(e2 < Duration::from_secs(2), "{e2:?}");
}

/// A scene written out with `SceneDesc`'s Serialize and read back with
/// `Scene::from_json` must be the same scene, bit for bit. Without
/// serde_json's `float_roundtrip` feature, 14.829037328432975 came back
/// one ulp off.
#[test]
fn scene_json_round_trips_bit_exact() {
    let x = f64::from_bits(0x402d_a877_94a9_90d0);
    let desc = SceneDesc {
        objects: vec![ObjectDesc {
            shape: Shape::cuboid(Vec3::new(x, 0.0, 0.0), Vec3::new(x + 1.0, 1.0, 3.0)),
            material: "brick".into(),
        }],
        ..SceneDesc::default()
    };
    let text = serde_json::to_string(&desc).unwrap();
    let back: SceneDesc = serde_json::from_str(&text).unwrap();
    eprintln!("{text}");
    let Shape::Box { min, .. } = back.objects[0].shape else {
        panic!("not a box");
    };
    assert_eq!(
        min.x.to_bits(),
        x.to_bits(),
        "{x:?} came back as {:?}",
        min.x
    );
    assert_eq!(back, desc);
}

/// A town of about 20 000 poles and short walls over 200 x 200 m, optionally with
/// one extra pole 100 km away.
fn town(outlier: bool) -> Scene {
    let mut b = SceneBuilder::new();
    for i in 0..9_999 {
        let x = (i % 100) as f64 * 2.0;
        let y = (i / 100) as f64 * 2.0;
        b.add(
            Shape::pole(x + 0.5, y + 0.5, 0.0, 0.1, 3.0),
            Builtin::Wood.id(),
        )
        .unwrap();
        b.add(
            Shape::wall_between(x + 1.0, y, x + 1.0, y + 1.0, 0.0, 3.0),
            Builtin::Brick.id(),
        )
        .unwrap();
    }
    if outlier {
        b.add(
            Shape::pole(99_999.0, 99_999.0, 0.0, 0.1, 3.0),
            Builtin::Wood.id(),
        )
        .unwrap();
    }
    b.build()
}

/// The per-frame queries should not depend on a shape nobody is near. A
/// uniform grid sized from the whole scene's span put the whole town in
/// one cell once one pole stood 100 km away.
#[test]
fn one_far_shape_does_not_flatten_the_grid() {
    let lis = Vec3::new(100.3, 100.2, 1.5);
    let src = Vec3::new(120.3, 110.7, 1.5);
    let run = |s: &Scene, label: &str| {
        let (prop, p) = time(&format!("{label} propagate"), || {
            propagate(s, src, lis, &QueryOptions::default())
        });
        eprintln!("{label} propagate truncated: {}", prop.truncated);
        let (_, r) = time(&format!("{label} estimate_reverb"), || {
            estimate_reverb(s, lis, &ReverbOptions::default())
        });
        let (_, n) = time(&format!("{label} 100 line_of_sight"), || {
            (0..100)
                .filter(|i| s.line_of_sight(lis, src + Vec3::new(*i as f64 * 0.1, 0.0, 0.0)))
                .count()
        });
        (p, r, n)
    };
    let (p0, r0, n0) = run(&town(false), "town");
    let (p1, r1, n1) = run(&town(true), "town + far pole");
    assert!(
        p1 < p0 * 4 + Duration::from_millis(2),
        "propagate {p0:?} -> {p1:?}"
    );
    assert!(
        r1 < r0 * 4 + Duration::from_millis(2),
        "reverb {r0:?} -> {r1:?}"
    );
    assert!(
        n1 < n0 * 4 + Duration::from_millis(2),
        "line of sight {n0:?} -> {n1:?}"
    );
}

/// A material name over 64 characters is cut to 64 when the material is
/// added; objects naming it in full must still find it.
#[test]
fn long_material_name_still_matches_its_objects() {
    let name = "x".repeat(70);
    let text = format!(
        r#"{{"materials":[{{"name":"{name}","absorption":[0.1,0.1,0.1,0.1,0.1,0.1,0.1]}}],
            "objects":[{{"kind":"box","min":[0,0,0],"max":[1,1,1],"material":"{name}"}}]}}"#
    );
    let (scene, report) = Scene::from_json(&text, Limits::default()).unwrap();
    eprintln!("{report:?}");
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(scene.objects().len(), 1);
}

/// 20 000 identical poles at one spot: no index can split them, so the
/// allowances must hold the work. Rays that cannot be afforded are left
/// out and the estimate still comes back usable.
#[test]
fn identical_poles_stay_within_the_allowances() {
    let mut b = SceneBuilder::new();
    for _ in 0..20_000 {
        b.add(Shape::pole(2.0, 2.0, 0.0, 0.5, 3.0), Builtin::Wood.id())
            .unwrap();
    }
    let s = b.build();
    let lis = Vec3::new(2.8, 2.0, 1.5);
    let opts = ReverbOptions {
        rays: usize::MAX,
        ..ReverbOptions::default()
    };
    let (e, t) = time("estimate_reverb identical poles", || {
        estimate_reverb(&s, lis, &opts)
    });
    assert!(t < FRAME_BUDGET, "{t:?}");
    assert!(e.truncated && e.rays < 4096 && e.rays > 0, "{e:?}");
    assert!(e.volume_m3.is_finite() && e.rt60().is_finite(), "{e:?}");
    let (n, t) = time("nearby_surfaces identical poles", || {
        nearby_surfaces(
            &s,
            lis,
            &NearbyOptions {
                radius: 4.0,
                max: 256,
                include_planes: false,
            },
        )
    });
    assert!(t < FRAME_BUDGET, "{t:?}");
    assert!(n.truncated);
    // A tiny allowance stops the corner checks at once.
    let (r, t) = time("navigate identical poles, tiny allowance", || {
        navigate(
            &s,
            Vec3::new(0.0, 2.0, 1.5),
            Vec3::new(4.0, 2.0, 1.5),
            &NavOptions {
                max_shape_tests: 10,
                ..NavOptions::default()
            },
        )
    });
    assert!(t < FRAME_BUDGET, "{t:?}");
    assert!(r.truncated && !r.reached, "{r:?}");
}
