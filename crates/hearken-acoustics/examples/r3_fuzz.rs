//! Random hostile scenes and queries; panics or slow calls are printed.
use hearken_acoustics::*;
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f64 {
        match self.next() % 16 {
            0 => 0.0,
            1 => 1e-9,
            2 => -1e-9,
            3 => 1e5,
            4 => -1e5,
            5 => 99_999.999,
            6 => 1e-3,
            7 => 3.0,
            _ => {
                ((self.next() % 2_000_000) as f64 / 1000.0 - 1000.0)
                    * if self.next().is_multiple_of(4) {
                        0.001
                    } else {
                        1.0
                    }
            }
        }
    }
    fn v(&mut self) -> Vec3 {
        Vec3::new(self.f(), self.f(), self.f())
    }
}

fn main() {
    let seeds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let mut worst = (0.0f64, 0u64, "");
    for seed in 1..=seeds {
        let mut r = Rng(seed * 0x9E37_79B9_7F4A_7C15);
        let mut b = SceneBuilder::new();
        let n = (r.next() % 60) as usize;
        for _ in 0..n {
            let shape = match r.next() % 5 {
                0 => Shape::Wall {
                    origin: r.v(),
                    u: r.v(),
                    v: r.v(),
                },
                1 => Shape::cuboid(r.v(), r.v()),
                2 => Shape::Cylinder {
                    base: r.v(),
                    radius: r.f().abs(),
                    height: r.f(),
                },
                3 => Shape::Ground { z: r.f() },
                _ => Shape::Ceiling { z: r.f() },
            };
            let m = MaterialId((r.next() % 20) as u16);
            let _ = b.add(shape, m);
        }
        let s = b.build();
        for _ in 0..20 {
            let (a, l) = (r.v(), r.v());
            let opts = QueryOptions {
                max_path_nodes: 2048,
                max_visibility_tests: 200_000,
                max_shape_tests: 50_000_000,
                ..QueryOptions::default()
            };
            let res = std::panic::catch_unwind(|| {
                let t = Instant::now();
                let p = propagate(&s, a, l, &opts);
                let r1 = p.render(37.0, None);
                let tp = t.elapsed().as_secs_f64();
                let t = Instant::now();
                let e = estimate_reverb(&s, l, &ReverbOptions::default());
                let _ = e.render(s.speed_of_sound());
                let tr = t.elapsed().as_secs_f64();
                let t = Instant::now();
                let nv = navigate(&s, a, l, &NavOptions::default());
                let tn = t.elapsed().as_secs_f64();
                let _ = nearby_surfaces(&s, l, &NearbyOptions::default());
                for g in r1.gains {
                    assert!(g.is_finite() && (0.0..=1.0001).contains(&g), "gain {g}");
                }
                assert!(r1.delay_s.is_finite() && r1.delay_s >= 0.0);
                for t in &r1.taps[..r1.tap_count as usize] {
                    for g in t.gains {
                        assert!(g.is_finite() && (0.0..=1.0001).contains(&g), "tap gain {g}");
                    }
                }
                for v in e.rt60().0 {
                    assert!(v.is_finite() && v >= 0.0);
                }
                for v in e.wet_gain.0 {
                    assert!(v.is_finite() && v >= 0.0);
                }
                let _ = nv;
                (tp, tr, tn)
            });
            match res {
                Err(_) => println!("PANIC seed {seed} a={a:?} l={l:?}"),
                Ok((tp, tr, tn)) => {
                    for (t, k) in [(tp, "propagate"), (tr, "reverb"), (tn, "navigate")] {
                        if t > worst.0 {
                            worst = (t, seed, k);
                        }
                    }
                }
            }
        }
    }
    println!("worst {worst:?}");
}
