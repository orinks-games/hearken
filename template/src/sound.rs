//! The practice room's own sound: a synth-voice recipe, written the way a
//! game would write its own.

use hearken_app::synth::fundsp::prelude32::*;
use hearken_app::synth::Params;

/// A low engine hum whose pitch follows `throttle`, 0 to 1.
pub fn engine(p: &Params) -> Box<dyn AudioUnit> {
    Box::new((var(&p.get("throttle")) * 110.0 + 55.0) >> saw() >> (lowpass_hz(700.0, 0.8) * 0.3))
}
