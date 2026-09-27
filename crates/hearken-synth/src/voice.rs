//! Live-voice recipes.
//!
//! A recipe is a plain function from a small set of named, shared
//! parameters to a fundsp graph. A [`Voice`] owns one, rebuilding or
//! refilling it as the game runs:
//!
//! ```
//! use hearken_synth::fundsp::prelude32::*;
//! use hearken_synth::Params;
//!
//! fn engine(p: &Params) -> Box<dyn AudioUnit> {
//!     Box::new(var(&p.get("throttle")) >> lowpass_hz(800.0, 0.7))
//! }
//! ```

use crate::SAMPLE_RATE;
use fundsp::prelude32::*;

/// Builds a fundsp graph from a voice's current parameters.
pub type Recipe = fn(&Params) -> Box<dyn AudioUnit>;

/// Named, shared control values a [`Recipe`] reads with [`Params::get`].
/// Cloning a `Params` shares the same underlying values -- setting one
/// through the clone is visible through the original.
#[derive(Clone, Default)]
pub struct Params(Vec<(&'static str, Shared)>);

impl Params {
    /// Starts a set of parameters at their initial values.
    pub fn new(initial: &[(&'static str, f32)]) -> Params {
        Params(
            initial
                .iter()
                .map(|&(name, value)| (name, Shared::new(value)))
                .collect(),
        )
    }

    fn find(&self, name: &str) -> Option<&Shared> {
        self.0
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, shared)| shared)
    }

    /// The shared cell for `name`, to read live inside a recipe with
    /// `var(&p.get(name))`. An unknown name is a game bug, not a crash: it
    /// warns and returns an unattached cell fixed at `0.0`.
    pub fn get(&self, name: &str) -> Shared {
        match self.find(name) {
            Some(shared) => shared.clone(),
            None => {
                log::warn!("unknown synth parameter {name:?}; using an unattached 0.0");
                Shared::new(0.0)
            }
        }
    }

    /// Sets a known parameter and returns `true`, or refuses an unknown one
    /// (logging at debug level) and returns `false`.
    pub fn set(&self, name: &str, value: f32) -> bool {
        match self.find(name) {
            Some(shared) => {
                shared.set(value);
                true
            }
            None => {
                log::debug!("unknown synth parameter {name:?}; ignoring set");
                false
            }
        }
    }

    /// The current value of a known parameter, or `None` for an unknown one.
    pub fn value(&self, name: &str) -> Option<f32> {
        self.find(name).map(Shared::value)
    }
}

/// A recipe's rendered graph, together with the parameters driving it.
pub struct Voice {
    recipe: Recipe,
    params: Params,
    unit: Box<dyn AudioUnit>,
}

impl Voice {
    /// Builds a voice from a recipe and its initial parameter values.
    pub fn new(recipe: Recipe, initial: &[(&'static str, f32)]) -> Voice {
        Voice::with_params(recipe, Params::new(initial))
    }

    /// Builds a voice from a recipe and an existing [`Params`], sharing its
    /// current values instead of starting fresh.
    pub fn with_params(recipe: Recipe, params: Params) -> Voice {
        let mut unit = recipe(&params);
        if unit.inputs() != 0 || !matches!(unit.outputs(), 1 | 2) {
            log::error!(
                "synth recipe has {} input(s) and {} output(s); a voice needs 0 inputs and 1 or 2 outputs -- rendering silence instead",
                unit.inputs(),
                unit.outputs()
            );
            unit = Box::new(dc(0.0));
        }
        unit.set_sample_rate(SAMPLE_RATE as f64);
        Voice {
            recipe,
            params,
            unit,
        }
    }

    /// The recipe this voice was built from.
    pub fn recipe(&self) -> Recipe {
        self.recipe
    }

    /// A clone of this voice's parameters, sharing the same values.
    pub fn params(&self) -> Params {
        self.params.clone()
    }

    /// Re-targets the voice's graph to a new sample rate.
    pub fn set_sample_rate(&mut self, hz: f64) {
        self.unit.set_sample_rate(hz);
    }

    /// Fills `out` with one mono sample per element.
    pub fn fill(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = self.unit.get_mono();
        }
    }
}
