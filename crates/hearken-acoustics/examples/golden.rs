//! A fixed scene and a hash over every query's output bits, shared by the
//! determinism test and a WebAssembly build of the same check. Exports
//! `golden_hash` for a WebAssembly host; see `tests/determinism.rs`.

use hearken_acoustics::*;

/// FNV-1a over the bits of every number fed in.
struct Hash(u64);

impl Hash {
    fn new() -> Hash {
        Hash(0xcbf2_9ce4_8422_2325)
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn f(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn v(&mut self, v: Vec3) {
        self.f(v.x);
        self.f(v.y);
        self.f(v.z);
    }
    fn b(&mut self, b: Bands) {
        for v in b.0 {
            self.f(v);
        }
    }
}

/// A house with two rooms joined by a doorway, a pillar, a table, and a
/// tree in the yard.
pub fn scene() -> Scene {
    let mut b = SceneBuilder::new();
    b.atmosphere(Atmosphere::new(12.5, 63.0, 99.8));
    let plaster = Builtin::Plaster.id();
    let brick = Builtin::Brick.id();
    b.add(Shape::Ground { z: 0.0 }, Builtin::Grass.id())
        .unwrap();
    // Outer walls of a 12 x 8 house, open door in the south wall.
    b.add(Shape::wall_between(0.0, 0.0, 5.0, 0.0, 0.0, 3.0), brick)
        .unwrap();
    b.add(Shape::wall_between(6.0, 0.0, 12.0, 0.0, 0.0, 3.0), brick)
        .unwrap();
    b.add(Shape::wall_between(12.0, 0.0, 12.0, 8.0, 0.0, 3.0), brick)
        .unwrap();
    b.add(Shape::wall_between(12.0, 8.0, 0.0, 8.0, 0.0, 3.0), brick)
        .unwrap();
    b.add(Shape::wall_between(0.0, 8.0, 0.0, 0.0, 0.0, 3.0), brick)
        .unwrap();
    // Inner wall with a doorway between y = 3 and y = 4.
    b.add(Shape::wall_between(6.0, 0.0, 6.0, 3.0, 0.0, 3.0), plaster)
        .unwrap();
    b.add(Shape::wall_between(6.0, 4.0, 6.0, 8.0, 0.0, 3.0), plaster)
        .unwrap();
    // Roof.
    b.add(
        Shape::Box {
            min: Vec3::new(0.0, 0.0, 3.0),
            max: Vec3::new(12.0, 8.0, 3.3),
        },
        Builtin::Wood.id(),
    )
    .unwrap();
    b.add(
        Shape::pole(3.0, 5.0, 0.0, 0.25, 3.0),
        Builtin::Concrete.id(),
    )
    .unwrap();
    b.add(
        Shape::cuboid(Vec3::new(8.0, 5.0, 0.0), Vec3::new(10.0, 6.0, 0.9)),
        Builtin::Wood.id(),
    )
    .unwrap();
    b.add(Shape::pole(4.0, -6.0, 0.0, 1.5, 6.0), Builtin::Foliage.id())
        .unwrap();
    b.build()
}

/// Runs every query on `s` and hashes the bits of the results.
pub fn run_all(s: &Scene) -> u64 {
    let mut h = Hash::new();
    h.f(s.speed_of_sound());
    h.b(s.air_absorption_db_per_m());
    let lis = Vec3::new(2.5, 4.0, 1.6);
    let sources = [
        Vec3::new(9.0, 2.0, 1.2),   // next room, through the doorway
        Vec3::new(4.0, 6.5, 0.3),   // same room, behind the pillar
        Vec3::new(5.5, -10.0, 1.0), // outside, behind the tree
        Vec3::new(2.5, 4.0, 1.6),   // at the listener
    ];
    let opts = QueryOptions::default();
    let room = estimate_reverb(s, lis, &ReverbOptions::default());
    for src in sources {
        let p = propagate(s, src, lis, &opts);
        h.f(p.direct.distance);
        h.f(p.direct.delay_s);
        h.b(p.direct.gain);
        h.u64(p.direct.obstacles as u64);
        if let Some(d) = &p.diffracted {
            for c in &d.corners {
                h.v(*c);
            }
            h.f(d.length);
            h.b(d.gain);
        }
        for r in &p.reflections {
            h.u64(r.order as u64);
            h.v(r.points[0]);
            h.v(r.points[1]);
            h.v(r.direction);
            h.f(r.delay_s);
            h.b(r.gain);
        }
        let rr = p.render(37.0, Some(&room));
        for g in rr.gains {
            h.u64(g.to_bits() as u64);
        }
        h.u64(rr.azimuth_deg.to_bits() as u64);
        h.u64(rr.elevation_deg.to_bits() as u64);
        h.u64(rr.reverb_send.to_bits() as u64);
    }
    h.f(room.volume_m3);
    h.f(room.surface_m2);
    h.b(room.rt60_sabine);
    h.b(room.rt60_eyring);
    h.b(room.wet_gain);
    let n = navigate(s, lis, Vec3::new(9.0, -3.0, 1.6), &NavOptions::default());
    for p in &n.points {
        h.v(*p);
    }
    h.f(n.length);
    let near = nearby_surfaces(s, lis, &NearbyOptions::default());
    for x in &near.surfaces {
        h.u64(x.object.0 as u64);
        h.v(x.point);
        h.f(x.distance);
    }
    h.0
}

/// The hash of [`run_all`] over [`scene`], for a WebAssembly host.
#[no_mangle]
pub extern "C" fn golden_hash() -> u64 {
    run_all(&scene())
}
