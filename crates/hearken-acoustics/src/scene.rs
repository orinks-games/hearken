//! A scene: the air, the materials and the shapes, checked and indexed.

use crate::atmosphere::Atmosphere;
use crate::bands::Bands;
use crate::geometry::{Shape, ShapeError, ShapeFields};
use crate::index::Tree;
use crate::material::{Builtin, Material, MaterialDesc, MaterialError};
use crate::vec::Vec3;
use serde::{Deserialize, Serialize};

/// A material in a scene. The built-in materials come first, in
/// [`Builtin::ALL`] order, so [`Builtin::id`] works in every scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MaterialId(pub u16);

/// A shape in a scene, numbered in the order it was added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub u32);

impl Builtin {
    /// The id this built-in material has in every scene.
    pub fn id(self) -> MaterialId {
        let i = Builtin::ALL.iter().position(|b| *b == self).unwrap_or(0);
        MaterialId(i as u16)
    }
}

/// A shape and what it is made of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Object {
    pub shape: Shape,
    pub material: MaterialId,
}

/// Caps on what a scene may hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// Most shapes.
    pub max_objects: usize,
    /// Most materials, the built-in ones included.
    pub max_materials: usize,
    /// Farthest any coordinate may be from the origin, metres.
    pub max_extent_m: f64,
}

/// Highest `max_objects` allowed.
pub const MAX_OBJECTS: usize = 100_000;
/// Highest `max_materials` allowed.
pub const MAX_MATERIALS: usize = 4096;
/// Most bytes [`Scene::from_json`] reads.
pub const MAX_SCENE_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Most dropped items a [`Report`] lists one by one.
pub const MAX_REPORTED: usize = 64;

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_objects: 20_000,
            max_materials: 256,
            max_extent_m: 100_000.0,
        }
    }
}

impl Limits {
    /// These limits within the hard caps.
    pub fn clamped(self) -> Limits {
        let extent = if self.max_extent_m.is_finite() {
            self.max_extent_m.max(1.0).min(1.0e6)
        } else {
            1.0e6
        };
        Limits {
            max_objects: self.max_objects.min(MAX_OBJECTS),
            max_materials: self.max_materials.clamp(Builtin::ALL.len(), MAX_MATERIALS),
            max_extent_m: extent,
        }
    }
}

/// Why something was not added to a scene.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneError {
    Material(MaterialError),
    Shape(ShapeError),
    UnknownMaterial,
    TooManyObjects,
    TooManyMaterials,
}

impl core::fmt::Display for SceneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SceneError::Material(e) => e.fmt(f),
            SceneError::Shape(e) => e.fmt(f),
            SceneError::UnknownMaterial => write!(f, "no material by that name or id"),
            SceneError::TooManyObjects => write!(f, "the scene has too many shapes"),
            SceneError::TooManyMaterials => write!(f, "the scene has too many materials"),
        }
    }
}

/// What was left out of a scene and why.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// The first [`MAX_REPORTED`] items left out.
    pub dropped: Vec<Dropped>,
    /// How many items were left out in all.
    pub dropped_count: usize,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.dropped_count == 0
    }

    fn drop_item(&mut self, item: Item, error: SceneError) {
        self.dropped_count += 1;
        if self.dropped.len() < MAX_REPORTED {
            self.dropped.push(Dropped { item, error });
        }
    }
}

/// One item left out of a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Dropped {
    pub item: Item,
    pub error: SceneError,
}

/// Which item, by its position in the description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Material(usize),
    Object(usize),
}

/// A scene as written in a file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneDesc {
    pub atmosphere: Atmosphere,
    pub materials: Vec<MaterialDesc>,
    pub objects: Vec<ObjectDesc>,
}

/// A shape as written in a file, its material named. Unknown fields are
/// skipped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ShapeFields")]
pub struct ObjectDesc {
    #[serde(flatten)]
    pub shape: Shape,
    pub material: String,
}

impl TryFrom<ShapeFields> for ObjectDesc {
    type Error = String;
    fn try_from(f: ShapeFields) -> Result<ObjectDesc, String> {
        let shape = f.shape()?;
        let material = f.material.ok_or("missing field `material`")?;
        Ok(ObjectDesc { shape, material })
    }
}

/// Why [`Scene::from_json`] could not read a scene at all.
#[derive(Debug)]
pub enum LoadError {
    TooLarge,
    Json(serde_json::Error),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadError::TooLarge => write!(f, "scene file is too large"),
            LoadError::Json(e) => write!(f, "scene file is not valid: {e}"),
        }
    }
}

/// Collects materials and shapes, checking each, then builds a [`Scene`].
#[derive(Clone, Debug)]
pub struct SceneBuilder {
    limits: Limits,
    atmosphere: Atmosphere,
    materials: Vec<Material>,
    objects: Vec<Object>,
}

impl Default for SceneBuilder {
    fn default() -> Self {
        SceneBuilder::new()
    }
}

impl SceneBuilder {
    /// An empty scene with the built-in materials and default limits.
    pub fn new() -> SceneBuilder {
        SceneBuilder::with_limits(Limits::default())
    }

    pub fn with_limits(limits: Limits) -> SceneBuilder {
        SceneBuilder {
            limits: limits.clamped(),
            atmosphere: Atmosphere::default(),
            materials: Builtin::ALL.iter().map(|b| b.material()).collect(),
            objects: Vec::new(),
        }
    }

    pub fn atmosphere(&mut self, a: Atmosphere) -> &mut Self {
        self.atmosphere = a.clamped();
        self
    }

    /// Adds a material. One with the name of an existing material replaces
    /// it and keeps its id.
    pub fn add_material(&mut self, m: &Material) -> Result<MaterialId, SceneError> {
        let m = m.sanitized().map_err(SceneError::Material)?;
        if let Some(id) = self.material_id(&m.name) {
            self.materials[id.0 as usize] = m;
            return Ok(id);
        }
        if self.materials.len() >= self.limits.max_materials {
            return Err(SceneError::TooManyMaterials);
        }
        self.materials.push(m);
        Ok(MaterialId((self.materials.len() - 1) as u16))
    }

    pub fn material_id(&self, name: &str) -> Option<MaterialId> {
        let name = crate::material::material_name(name);
        self.materials
            .iter()
            .position(|m| m.name == name)
            .map(|i| MaterialId(i as u16))
    }

    /// Adds a shape.
    pub fn add(&mut self, shape: Shape, material: MaterialId) -> Result<ObjectId, SceneError> {
        if material.0 as usize >= self.materials.len() {
            return Err(SceneError::UnknownMaterial);
        }
        shape
            .validate(self.limits.max_extent_m)
            .map_err(SceneError::Shape)?;
        if self.objects.len() >= self.limits.max_objects {
            return Err(SceneError::TooManyObjects);
        }
        self.objects.push(Object { shape, material });
        Ok(ObjectId((self.objects.len() - 1) as u32))
    }

    /// Indexes the shapes and gives the finished scene.
    pub fn build(&self) -> Scene {
        let bounds: Vec<_> = self.objects.iter().map(|o| o.shape.bounds()).collect();
        let mut s = Scene {
            atmosphere: self.atmosphere,
            speed: 0.0,
            air_db: Bands::ZERO,
            materials: self.materials.clone(),
            objects: self.objects.clone(),
            index: Tree::build(&bounds),
            transmission: self
                .materials
                .iter()
                .map(|m| m.transmission_gain())
                .collect(),
            specular: self.materials.iter().map(|m| m.specular_gain()).collect(),
        };
        s.set_atmosphere(self.atmosphere);
        s
    }
}

/// Scratch space and an allowance of shape tests for one query. Each line
/// or point test charges the number of shapes it looks at; once the
/// allowance is spent, tests give up and the query reports it was cut short.
#[derive(Clone, Debug)]
pub(crate) struct Probe {
    pub buf: Vec<u32>,
    limit: usize,
    used: usize,
    spent: bool,
}

impl Probe {
    pub fn new(limit: usize) -> Probe {
        Probe {
            buf: Vec::new(),
            limit,
            used: 0,
            spent: false,
        }
    }

    pub fn unlimited() -> Probe {
        Probe::new(usize::MAX)
    }

    /// Takes `n` from the allowance; false if it does not cover them.
    pub fn charge(&mut self, n: usize) -> bool {
        // Every test costs at least one, so empty tests are bounded too.
        let n = n.max(1);
        if self.spent || n > self.limit - self.used {
            self.spent = true;
            return false;
        }
        self.used += n;
        true
    }

    pub fn used(&self) -> usize {
        self.used
    }

    /// What is left of the allowance.
    pub fn left(&self) -> usize {
        if self.spent {
            0
        } else {
            self.limit - self.used
        }
    }

    /// Marks the allowance as spent: a test was refused.
    pub fn refuse(&mut self) {
        self.spent = true;
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Whether some test was refused.
    pub fn spent(&self) -> bool {
        self.spent
    }
}

/// The world as acoustics sees it. Build one with [`SceneBuilder`] or
/// [`Scene::from_desc`]; it does not change afterwards apart from the air.
#[derive(Clone, Debug)]
pub struct Scene {
    atmosphere: Atmosphere,
    speed: f64,
    air_db: Bands,
    materials: Vec<Material>,
    objects: Vec<Object>,
    index: Tree,
    transmission: Vec<Bands>,
    specular: Vec<Bands>,
}

impl Default for Scene {
    fn default() -> Self {
        SceneBuilder::new().build()
    }
}

impl Scene {
    /// Builds a scene from a description, leaving out (and reporting) what
    /// does not pass the checks.
    pub fn from_desc(desc: &SceneDesc, limits: Limits) -> (Scene, Report) {
        let mut b = SceneBuilder::with_limits(limits);
        let mut report = Report::default();
        b.atmosphere(desc.atmosphere);
        for (i, m) in desc.materials.iter().enumerate() {
            let r = m
                .to_material()
                .map_err(SceneError::Material)
                .and_then(|m| b.add_material(&m));
            if let Err(e) = r {
                report.drop_item(Item::Material(i), e);
            }
        }
        for (i, o) in desc.objects.iter().enumerate() {
            let r = match b.material_id(&o.material) {
                Some(id) => b.add(o.shape, id),
                None => Err(SceneError::UnknownMaterial),
            };
            if let Err(e) = r {
                report.drop_item(Item::Object(i), e);
            }
        }
        (b.build(), report)
    }

    /// Reads a scene from JSON text (see [`SceneDesc`]).
    pub fn from_json(text: &str, limits: Limits) -> Result<(Scene, Report), LoadError> {
        if text.len() > MAX_SCENE_JSON_BYTES {
            return Err(LoadError::TooLarge);
        }
        let desc: SceneDesc = serde_json::from_str(text).map_err(LoadError::Json)?;
        Ok(Scene::from_desc(&desc, limits))
    }

    pub fn atmosphere(&self) -> Atmosphere {
        self.atmosphere
    }

    /// Changes the air (weather) without rebuilding anything else.
    pub fn set_atmosphere(&mut self, a: Atmosphere) {
        self.atmosphere = a.clamped();
        self.speed = self.atmosphere.speed_of_sound();
        self.air_db = self.atmosphere.absorption_bands_db_per_m();
    }

    /// Speed of sound in this scene's air, m/s.
    pub fn speed_of_sound(&self) -> f64 {
        self.speed
    }

    /// Air absorption per band, dB per metre.
    pub fn air_absorption_db_per_m(&self) -> Bands {
        self.air_db
    }

    /// Amplitude left after `distance` metres of air, per band.
    pub fn air_gain(&self, distance: f64) -> Bands {
        let d = distance.max(0.0);
        self.air_db.map(|db| crate::bands::db_loss_to_gain(db * d))
    }

    pub fn objects(&self) -> &[Object] {
        &self.objects
    }

    pub fn object(&self, id: ObjectId) -> Option<&Object> {
        self.objects.get(id.0 as usize)
    }

    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    pub fn material(&self, id: MaterialId) -> Option<&Material> {
        self.materials.get(id.0 as usize)
    }

    pub fn material_id(&self, name: &str) -> Option<MaterialId> {
        let name = crate::material::material_name(name);
        self.materials
            .iter()
            .position(|m| m.name == name)
            .map(|i| MaterialId(i as u16))
    }

    pub(crate) fn transmission_gain(&self, id: MaterialId) -> Bands {
        self.transmission
            .get(id.0 as usize)
            .copied()
            .unwrap_or(Bands::ZERO)
    }

    pub(crate) fn specular_gain(&self, id: MaterialId) -> Bands {
        self.specular
            .get(id.0 as usize)
            .copied()
            .unwrap_or(Bands::ZERO)
    }

    /// Shapes that might touch the segment from `p` to `q`, in id order;
    /// false (and `out` empty) as soon as more than `limit` turn up, so a
    /// refused test costs little.
    fn candidates_along_capped(&self, p: Vec3, q: Vec3, limit: usize, out: &mut Vec<u32>) -> bool {
        self.index.along_capped(p, q, limit, out) && out.len() <= limit
    }

    pub(crate) fn candidates_within(&self, lo: Vec3, hi: Vec3, out: &mut Vec<u32>) {
        self.index
            .within((lo.x, lo.y), (hi.x, hi.y), usize::MAX, out);
    }

    /// Calls `f` with each shape the segment from `p` to `q` passes through
    /// or into, in id order, skipping contact within a hair of either end.
    /// `f` returns false to stop early. Returns false, calling `f` for
    /// nothing, when the probe's allowance runs out.
    pub(crate) fn for_each_crossing(
        &self,
        p: Vec3,
        q: Vec3,
        probe: &mut Probe,
        mut f: impl FnMut(u32, &Object) -> bool,
    ) -> bool {
        let len = (q - p).length();
        if len <= 0.0 || !len.is_finite() {
            return true;
        }
        let eps = (1e-4 / len).min(0.25);
        let room = probe.left();
        let found = self.candidates_along_capped(p, q, room, &mut probe.buf);
        if !found || !probe.charge(probe.buf.len()) {
            probe.refuse();
            return false;
        }
        for &i in probe.buf.iter() {
            let o = &self.objects[i as usize];
            if let Some((t0, t1)) = o.shape.segment_overlap(p, q) {
                if t1 > eps && t0 < 1.0 - eps && !f(i, o) {
                    break;
                }
            }
        }
        true
    }

    /// Whether anything at all stands between `p` and `q`, apart from
    /// shape `skip` (and the endless planes when `planes` is false). Counts
    /// as blocked when the probe's allowance runs out.
    pub(crate) fn blocked(
        &self,
        p: Vec3,
        q: Vec3,
        skip: &[u32],
        planes: bool,
        probe: &mut Probe,
    ) -> bool {
        let mut hit = false;
        let done = self.for_each_crossing(p, q, probe, |i, o| {
            if skip.contains(&i) || (!planes && o.shape.is_unbounded()) {
                true
            } else {
                hit = true;
                false
            }
        });
        hit || !done
    }

    /// Whether `p` is inside a box or cylinder, charging the probe. Counts
    /// as inside when the allowance runs out.
    pub(crate) fn inside_solid(&self, p: Vec3, probe: &mut Probe) -> bool {
        let e = Vec3::new(1e-6, 1e-6, 0.0);
        let (lo, hi) = (p - e, p + e);
        let room = probe.left();
        let found = self
            .index
            .within((lo.x, lo.y), (hi.x, hi.y), room, &mut probe.buf)
            && probe.buf.len() <= room;
        if !found || !probe.charge(probe.buf.len()) {
            probe.refuse();
            return true;
        }
        probe
            .buf
            .iter()
            .any(|&j| self.objects[j as usize].shape.contains(p))
    }

    /// Whether nothing stands between `a` and `b`.
    pub fn line_of_sight(&self, a: Vec3, b: Vec3) -> bool {
        !self.blocked(a, b, &[], true, &mut Probe::unlimited())
    }

    /// The nearest surface a ray from `p` along unit `dir` meets within
    /// `max_distance`: distance, normal facing back along the ray, and
    /// which shape. Ties go to the lower id.
    pub fn ray_cast(&self, p: Vec3, dir: Vec3, max_distance: f64) -> Option<(f64, Vec3, ObjectId)> {
        let mut buf = Vec::new();
        self.ray_cast_with(p, dir, max_distance, &mut buf, usize::MAX)
            .flatten()
    }

    /// As [`Scene::ray_cast`], but gives `None` without testing anything
    /// when the ray has more than `limit` shapes near it, and `Some(hit)`
    /// otherwise. `buf` then holds the shapes it tested.
    pub(crate) fn ray_cast_with(
        &self,
        p: Vec3,
        dir: Vec3,
        max_distance: f64,
        buf: &mut Vec<u32>,
        limit: usize,
    ) -> Option<Option<(f64, Vec3, ObjectId)>> {
        buf.clear();
        if !(p.is_finite() && dir.is_finite() && max_distance > 0.0 && max_distance.is_finite()) {
            return Some(None);
        }
        if !self.candidates_along_capped(p, p + dir * max_distance, limit, buf) {
            return None;
        }
        let mut best: Option<(f64, Vec3, ObjectId)> = None;
        for &i in buf.iter() {
            if let Some((t, n)) = self.objects[i as usize].shape.ray_hit(p, dir, max_distance) {
                if t > 1e-6 && best.is_none_or(|b| t < b.0) {
                    best = Some((t, n, ObjectId(i)));
                }
            }
        }
        Some(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_ids_are_stable() {
        let s = Scene::default();
        for b in Builtin::ALL {
            assert_eq!(s.material(b.id()).unwrap().name, b.name());
            assert_eq!(s.material_id(b.name()), Some(b.id()));
        }
    }

    #[test]
    fn builder_refuses_bad_input() {
        let mut b = SceneBuilder::new();
        assert_eq!(
            b.add(Shape::Ground { z: f64::NAN }, Builtin::Grass.id()),
            Err(SceneError::Shape(ShapeError::NotFinite))
        );
        assert_eq!(
            b.add(Shape::Ground { z: 0.0 }, MaterialId(9999)),
            Err(SceneError::UnknownMaterial)
        );
        assert!(b.add(Shape::Ground { z: 0.0 }, Builtin::Grass.id()).is_ok());
    }

    #[test]
    fn object_cap_is_enforced() {
        let mut b = SceneBuilder::with_limits(Limits {
            max_objects: 3,
            ..Limits::default()
        });
        for _ in 0..3 {
            b.add(Shape::Ground { z: 0.0 }, Builtin::Grass.id())
                .unwrap();
        }
        assert_eq!(
            b.add(Shape::Ground { z: 0.0 }, Builtin::Grass.id()),
            Err(SceneError::TooManyObjects)
        );
        // An absurd cap is held to the hard one.
        let l = Limits {
            max_objects: usize::MAX,
            max_materials: 0,
            max_extent_m: f64::NAN,
        }
        .clamped();
        assert_eq!(l.max_objects, MAX_OBJECTS);
        assert_eq!(l.max_materials, Builtin::ALL.len());
        assert!(l.max_extent_m.is_finite());
    }

    #[test]
    fn material_replacement_keeps_id() {
        let mut b = SceneBuilder::new();
        let mut m = Builtin::Wood.material();
        m.absorption = Bands::splat(0.5);
        assert_eq!(b.add_material(&m), Ok(Builtin::Wood.id()));
        let s = b.build();
        assert_eq!(s.material(Builtin::Wood.id()).unwrap().absorption.0[0], 0.5);
    }

    #[test]
    fn loads_json_and_reports_drops() {
        let text = r#"{
            "atmosphere": {"temperature_c": 5, "relative_humidity": 80},
            "materials": [
                {"name": "moss", "absorption": [0.2,0.3,0.4,0.5,0.6,0.7,0.7]},
                {"name": "", "absorption": [0,0,0,0,0,0,0]}
            ],
            "objects": [
                {"kind": "ground", "z": 0, "material": "moss"},
                {"kind": "box", "min": {"x":0,"y":0,"z":0}, "max": {"x":1,"y":1,"z":1}, "material": "brick"},
                {"kind": "box", "min": {"x":0,"y":0,"z":0}, "max": {"x":-1,"y":1,"z":1}, "material": "brick"},
                {"kind": "cylinder", "base": {"x":0,"y":0,"z":0}, "radius": 1, "height": 1, "material": "nope"}
            ]
        }"#;
        let (s, r) = Scene::from_json(text, Limits::default()).unwrap();
        assert_eq!(s.objects().len(), 2);
        assert_eq!(r.dropped_count, 3);
        assert_eq!(r.dropped[0].item, Item::Material(1));
        assert_eq!(r.dropped[1].item, Item::Object(2));
        assert_eq!(r.dropped[2].error, SceneError::UnknownMaterial);
        assert_eq!(s.atmosphere().temperature_c, 5.0);
        assert_eq!(s.atmosphere().pressure_kpa, 101.325);
    }

    #[test]
    fn object_fields_round_trip_and_unknown_ones_are_skipped() {
        let o = ObjectDesc {
            shape: Shape::pole(1.0, 2.0, 0.0, 0.3, 3.0),
            material: "wood".into(),
        };
        let text = serde_json::to_string(&o).unwrap();
        assert!(text.contains("\"kind\":\"cylinder\""), "{text}");
        assert_eq!(serde_json::from_str::<ObjectDesc>(&text).unwrap(), o);
        let t = r#"{"junk":[[1],{"a":2}],"kind":"ground","z":1,"material":"grass"}"#;
        let o: ObjectDesc = serde_json::from_str(t).unwrap();
        assert_eq!(o.shape, Shape::Ground { z: 1.0 });
        for bad in [
            r#"{"kind":"ground","material":"grass"}"#,
            r#"{"kind":"ground","z":1}"#,
            r#"{"kind":"blob","z":1,"material":"grass"}"#,
            r#"{"kind":"ground","z":1,"z":2,"material":"grass"}"#,
        ] {
            assert!(serde_json::from_str::<ObjectDesc>(bad).is_err(), "{bad}");
        }
        let s: Shape = serde_json::from_str(r#"{"kind":"ceiling","z":3}"#).unwrap();
        assert_eq!(s, Shape::Ceiling { z: 3.0 });
    }

    #[test]
    fn garbage_json_is_an_error_not_a_panic() {
        for t in ["", "{", "[1,2,3]", "{\"objects\": 5}", &"[".repeat(100_000)] {
            assert!(Scene::from_json(t, Limits::default()).is_err());
        }
        let big = " ".repeat(MAX_SCENE_JSON_BYTES + 1);
        assert!(matches!(
            Scene::from_json(&big, Limits::default()),
            Err(LoadError::TooLarge)
        ));
    }

    #[test]
    fn report_list_is_bounded() {
        let mut desc = SceneDesc::default();
        for _ in 0..1000 {
            desc.objects.push(ObjectDesc {
                shape: Shape::Ground { z: f64::INFINITY },
                material: "brick".into(),
            });
        }
        let (_, r) = Scene::from_desc(&desc, Limits::default());
        assert_eq!(r.dropped_count, 1000);
        assert_eq!(r.dropped.len(), MAX_REPORTED);
    }

    #[test]
    fn line_of_sight_and_ray_cast() {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(5.0, -5.0, 5.0, 5.0, 0.0, 3.0),
            Builtin::Brick.id(),
        )
        .unwrap();
        let s = b.build();
        let a = Vec3::new(0.0, 0.0, 1.5);
        assert!(!s.line_of_sight(a, Vec3::new(10.0, 0.0, 1.5)));
        assert!(s.line_of_sight(a, Vec3::new(4.0, 0.0, 1.5)));
        assert!(s.line_of_sight(a, Vec3::new(10.0, 0.0, 7.0)));
        let (d, n, id) = s.ray_cast(a, Vec3::new(1.0, 0.0, 0.0), 100.0).unwrap();
        assert_eq!((d, n, id), (5.0, Vec3::new(-1.0, 0.0, 0.0), ObjectId(0)));
        assert!(s.ray_cast(a, Vec3::new(-1.0, 0.0, 0.0), 100.0).is_none());
    }
}
