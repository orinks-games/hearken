//! Scenes and positions a player or a mod could feed in: garbage numbers,
//! huge counts, worst-case options. Nothing may panic, hang or return a
//! non-finite number.

use hearken_acoustics::*;
use std::time::{Duration, Instant};

/// A small fixed generator, so the test is the same every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }
    fn unit(&mut self) -> f64 {
        self.next() as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    /// Mostly sane numbers, sometimes poison.
    fn wild(&mut self, lo: f64, hi: f64) -> f64 {
        match self.next() % 40 {
            0 => f64::NAN,
            1 => f64::INFINITY,
            2 => f64::NEG_INFINITY,
            3 => 1e300,
            4 => -1e300,
            5 => 0.0,
            6 => -1.0,
            _ => self.range(lo, hi),
        }
    }
    fn vec(&mut self, lo: f64, hi: f64) -> Vec3 {
        Vec3::new(self.wild(lo, hi), self.wild(lo, hi), self.wild(0.0, 4.0))
    }
}

fn random_shape(r: &mut Lcg) -> Shape {
    match r.next() % 6 {
        0 => Shape::Wall {
            origin: r.vec(-50.0, 50.0),
            u: r.vec(-10.0, 10.0),
            v: r.vec(-3.0, 3.0),
        },
        1 => Shape::wall_between(
            r.wild(-50.0, 50.0),
            r.wild(-50.0, 50.0),
            r.wild(-50.0, 50.0),
            r.wild(-50.0, 50.0),
            0.0,
            r.wild(0.0, 4.0),
        ),
        2 => Shape::cuboid(r.vec(-50.0, 50.0), r.vec(-50.0, 50.0)),
        3 => Shape::pole(
            r.wild(-50.0, 50.0),
            r.wild(-50.0, 50.0),
            0.0,
            r.wild(0.0, 2.0),
            r.wild(0.0, 5.0),
        ),
        4 => Shape::Ground {
            z: r.wild(-1.0, 0.0),
        },
        _ => Shape::Ceiling {
            z: r.wild(3.0, 5.0),
        },
    }
}

fn check_bands(b: Bands) {
    assert!(b.is_finite(), "{b:?}");
}

fn check_all(s: &Scene, r: &mut Lcg, opts: &QueryOptions) {
    for _ in 0..20 {
        let a = r.vec(-60.0, 60.0);
        let b = r.vec(-60.0, 60.0);
        let p = propagate(s, a, b, opts);
        check_bands(p.direct.gain);
        assert!(p.direct.delay_s.is_finite());
        if let Some(d) = &p.diffracted {
            check_bands(d.gain);
            assert!(d.length.is_finite());
        }
        for x in &p.reflections {
            check_bands(x.gain);
            assert!(x.delay_s.is_finite());
        }
        let room = estimate_reverb(s, b, &ReverbOptions::default());
        check_bands(room.rt60());
        check_bands(room.wet_gain);
        let out = p.render(r.wild(-720.0, 720.0), Some(&room));
        assert!(out.gains.iter().all(|g| g.is_finite()));
        assert!(out.direction.iter().all(|g| g.is_finite()));
        assert!(out.reverb_send.is_finite() && out.azimuth_deg.is_finite());
        let n = navigate(s, a, b, &NavOptions::default());
        assert!(n.length.is_finite());
        let near = nearby_surfaces(s, b, &NearbyOptions::default());
        for x in &near.surfaces {
            assert!(x.distance.is_finite());
        }
    }
}

#[test]
fn random_garbage_scenes() {
    let mut r = Lcg(0x5eed);
    for _ in 0..30 {
        let mut b = SceneBuilder::new();
        b.atmosphere(Atmosphere::new(
            r.wild(-80.0, 80.0),
            r.wild(-10.0, 120.0),
            r.wild(0.0, 200.0),
        ));
        for _ in 0..200 {
            let m = Builtin::ALL[(r.next() % Builtin::ALL.len() as u64) as usize].id();
            let _ = b.add(random_shape(&mut r), m);
        }
        let s = b.build();
        assert!(s.speed_of_sound().is_finite());
        check_all(&s, &mut r, &QueryOptions::default());
    }
}

#[test]
fn worst_case_options_stay_bounded() {
    // A dense grid of poles and walls, the largest caps, timed.
    let mut b = SceneBuilder::new();
    for i in 0..60 {
        for j in 0..60 {
            let (x, y) = (i as f64 * 2.0, j as f64 * 2.0);
            if (i + j) % 3 == 0 {
                b.add(
                    Shape::wall_between(x, y, x + 1.5, y + 0.5, 0.0, 3.0),
                    Builtin::Brick.id(),
                )
                .unwrap();
            } else {
                b.add(Shape::pole(x, y, 0.0, 0.3, 3.0), Builtin::Wood.id())
                    .unwrap();
            }
        }
    }
    b.add(Shape::Ground { z: 0.0 }, Builtin::Concrete.id())
        .unwrap();
    b.add(Shape::Ceiling { z: 3.5 }, Builtin::Concrete.id())
        .unwrap();
    let s = b.build();
    let opts = QueryOptions {
        reflection_order: u8::MAX,
        max_reflections: usize::MAX,
        reflection_radius: f64::INFINITY,
        max_reflection_faces: usize::MAX,
        diffraction: true,
        max_path_nodes: usize::MAX,
        max_visibility_tests: usize::MAX,
        path_margin: f64::MAX,
        reference_distance: 1.0,
        max_shape_tests: usize::MAX,
    };
    let start = Instant::now();
    let p = propagate(
        &s,
        Vec3::new(1.0, 1.0, 1.5),
        Vec3::new(115.0, 117.0, 1.5),
        &opts,
    );
    check_bands(p.direct.gain);
    let room = estimate_reverb(
        &s,
        Vec3::new(60.0, 61.0, 1.5),
        &ReverbOptions {
            rays: usize::MAX,
            max_distance: f64::MAX,
            reference_distance: 1.0,
        },
    );
    check_bands(room.rt60());
    let n = navigate(
        &s,
        Vec3::new(1.0, 1.0, 1.5),
        Vec3::new(115.0, 117.0, 1.5),
        &NavOptions {
            clearance: 0.2,
            max_nodes: usize::MAX,
            max_visibility_tests: usize::MAX,
            margin: f64::MAX,
            max_shape_tests: usize::MAX,
        },
    );
    assert!(n.length.is_finite());
    // Generous: the point is that it finishes, even in a debug build.
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn absurd_counts_are_capped() {
    let mut desc = SceneDesc::default();
    for i in 0..120_000 {
        desc.objects.push(ObjectDesc {
            shape: Shape::pole((i % 400) as f64, (i / 400) as f64, 0.0, 0.1, 2.0),
            material: "wood".into(),
        });
    }
    for i in 0..5000 {
        desc.materials.push(MaterialDesc {
            name: format!("m{i}"),
            absorption: Bands::splat(0.5),
            scattering: None,
            transmission_loss_db: None,
            surface_density_kg_m2: None,
        });
    }
    let (s, report) = Scene::from_desc(&desc, Limits::default());
    assert_eq!(s.objects().len(), Limits::default().max_objects);
    assert_eq!(s.materials().len(), Limits::default().max_materials);
    assert_eq!(
        report.dropped_count,
        120_000 - Limits::default().max_objects + 5000 + Builtin::ALL.len()
            - Limits::default().max_materials
    );
    assert!(report.dropped.len() <= 64);
    let p = propagate(
        &s,
        Vec3::new(-1.0, 10.0, 1.0),
        Vec3::new(401.0, 10.5, 1.0),
        &QueryOptions::default(),
    );
    assert!(p.direct.truncated);
}
