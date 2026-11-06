//! Settings and caps for the propagation queries.

/// Highest reflection order the image-source search supports.
pub const MAX_REFLECTION_ORDER: u8 = 2;
/// Most early reflections a query returns.
pub const MAX_REFLECTIONS: usize = 64;
/// Most faces the image-source search considers.
pub const MAX_REFLECTION_FACES: usize = 256;
/// Most corner points a path search considers.
pub const MAX_PATH_NODES: usize = 2048;
/// Most line-of-sight tests a path search makes.
pub const MAX_VISIBILITY_TESTS: usize = 200_000;
/// Most shape tests one search makes (see [`QueryOptions::max_shape_tests`]).
pub const MAX_SHAPE_TESTS: usize = 50_000_000;
/// Most shapes one straight path is counted as crossing; past this the path
/// counts as fully blocked.
pub const MAX_CROSSINGS: u32 = 64;

/// How much work a propagation query does. Every field is clamped by
/// [`QueryOptions::clamped`] before use, so any values are safe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QueryOptions {
    /// Distance at which spreading loss is 0 dB, metres. Closer sounds are
    /// not made louder.
    pub reference_distance: f64,
    /// Early reflection order: 0 (none), 1 or 2.
    pub reflection_order: u8,
    /// Most early reflections returned, strongest first.
    pub max_reflections: usize,
    /// Faces farther than this from the listener give no reflections, metres.
    pub reflection_radius: f64,
    /// Most faces the reflection search looks at (the nearest ones).
    pub max_reflection_faces: usize,
    /// Whether to search for a path round obstacles when the straight line
    /// is blocked.
    pub diffraction: bool,
    /// Most corner points the path search uses.
    pub max_path_nodes: usize,
    /// Most line-of-sight tests the path search makes.
    pub max_visibility_tests: usize,
    /// How far beyond the box around source and listener the path search
    /// looks for corners, metres.
    pub path_margin: f64,
    /// Most shape tests the path search makes, and again the reflection
    /// search. Each line or point test counts the shapes near it, so this
    /// bounds the work when many shapes crowd one spot. Corner checks in
    /// the path search use at most a quarter of it.
    pub max_shape_tests: usize,
}

impl Default for QueryOptions {
    fn default() -> Self {
        QueryOptions {
            reference_distance: 1.0,
            reflection_order: 2,
            max_reflections: 16,
            reflection_radius: 30.0,
            max_reflection_faces: 48,
            diffraction: true,
            max_path_nodes: 256,
            max_visibility_tests: 20_000,
            path_margin: 20.0,
            max_shape_tests: 1_000_000,
        }
    }
}

pub(crate) fn clamp_f(v: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v.max(lo).min(hi)
    } else {
        fallback
    }
}

impl QueryOptions {
    /// These options within the hard caps.
    pub fn clamped(self) -> QueryOptions {
        let d = QueryOptions::default();
        QueryOptions {
            reference_distance: clamp_f(self.reference_distance, 0.01, 100.0, d.reference_distance),
            reflection_order: self.reflection_order.min(MAX_REFLECTION_ORDER),
            max_reflections: self.max_reflections.min(MAX_REFLECTIONS),
            reflection_radius: clamp_f(self.reflection_radius, 0.0, 1000.0, d.reflection_radius),
            max_reflection_faces: self.max_reflection_faces.min(MAX_REFLECTION_FACES),
            diffraction: self.diffraction,
            max_path_nodes: self.max_path_nodes.min(MAX_PATH_NODES),
            max_visibility_tests: self.max_visibility_tests.min(MAX_VISIBILITY_TESTS),
            path_margin: clamp_f(self.path_margin, 0.0, 1000.0, d.path_margin),
            max_shape_tests: self.max_shape_tests.min(MAX_SHAPE_TESTS),
        }
    }
}

/// Spreading loss as an amplitude: `reference / distance`, never above 1.
pub(crate) fn spreading(distance: f64, reference: f64) -> f64 {
    reference / distance.max(reference)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamping() {
        let o = QueryOptions {
            reference_distance: f64::NAN,
            reflection_order: 200,
            max_reflections: usize::MAX,
            reflection_radius: -5.0,
            max_reflection_faces: usize::MAX,
            diffraction: true,
            max_path_nodes: usize::MAX,
            max_visibility_tests: usize::MAX,
            path_margin: f64::INFINITY,
            max_shape_tests: usize::MAX,
        }
        .clamped();
        assert_eq!(o.reference_distance, 1.0);
        assert_eq!(o.reflection_order, MAX_REFLECTION_ORDER);
        assert_eq!(o.max_reflections, MAX_REFLECTIONS);
        assert_eq!(o.reflection_radius, 0.0);
        assert_eq!(o.max_path_nodes, MAX_PATH_NODES);
        assert_eq!(o.path_margin, 20.0);
        assert_eq!(o.max_shape_tests, MAX_SHAPE_TESTS);
    }

    #[test]
    fn spreading_is_inverse_distance() {
        assert_eq!(spreading(2.0, 1.0), 0.5);
        assert_eq!(spreading(0.1, 1.0), 1.0);
        assert_eq!(spreading(0.0, 1.0), 1.0);
    }
}
