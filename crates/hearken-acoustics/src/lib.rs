//! How sound travels through a game's world.
//!
//! A game describes its world as a [`Scene`]: the air ([`Atmosphere`]),
//! surface [`Material`]s and simple shapes ([`Shape`]: walls, boxes, upright
//! cylinders, grounds and ceilings). Queries are pure functions of the scene
//! and positions:
//!
//! - [`propagate`]: everything about one sound (straight path with
//!   transmission, the path round obstacles, early reflections), and
//!   [`Propagation::render`] turns it into a fixed-size [`SoundRender`] for
//!   an audio thread;
//! - [`direct_path`], [`diffracted_path`], [`early_reflections`]: the
//!   parts on their own;
//! - [`estimate_reverb`]: the late reverb round a listener;
//! - [`navigate`]: a walking route through open space;
//! - [`nearby_surfaces`]: walls and poles near the listener.
//!
//! Axes are metres, x east, y north, z up; headings are degrees clockwise
//! from north, as in `hearken-audio`.
//!
//! Results are the same bits on every platform, wasm32 included: all
//! transcendental maths goes through `libm`, nothing uses threads, random
//! numbers or hash order, and every search runs in a fixed order. Scenes
//! may come from files players can edit, so every input is checked and
//! every search is capped; a query never panics and reports when a cap cut
//! it short.

// `x.max(lo).min(hi)` is used on purpose: unlike `clamp` it turns NaN
// into a bound.
#![allow(clippy::manual_clamp)]

pub mod atmosphere;
pub mod bands;
pub mod direct;
pub mod geometry;
mod index;
pub mod material;
pub mod nearby;
pub mod options;
pub mod path;
pub mod reflections;
pub mod render;
pub mod reverb;
pub mod scene;
pub mod vec;

pub use atmosphere::Atmosphere;
pub use bands::{Bands, BAND_CENTRES_HZ, BAND_COUNT, BAND_EXACT_HZ};
pub use direct::{direct_path, DirectPath};
pub use geometry::{Shape, ShapeError, ShapeKind};
pub use material::{mass_law_tl, Builtin, Material, MaterialDesc, MaterialError};
pub use nearby::{nearby_surfaces, Nearby, NearbyOptions, NearbySurface};
pub use options::QueryOptions;
pub use path::{
    barrier_attenuation_db, diffracted_path, navigate, DiffractedPath, DiffractionSearch,
    NavOptions, NavPath,
};
pub use reflections::{early_reflections, Reflection, Reflections};
pub use render::{
    propagate, to_listener_frame, Propagation, RoomRender, SoundRender, Tap, MAX_TAPS,
};
pub use reverb::{estimate_reverb, ray_directions, ReverbEstimate, ReverbOptions};
pub use scene::{
    Dropped, Item, Limits, LoadError, MaterialId, Object, ObjectDesc, ObjectId, Report, Scene,
    SceneBuilder, SceneDesc, SceneError,
};
pub use vec::Vec3;
