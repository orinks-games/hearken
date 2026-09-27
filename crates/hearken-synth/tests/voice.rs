use hearken_synth::fundsp::prelude32::*;
use hearken_synth::{Params, Voice};

fn level(p: &Params) -> Box<dyn AudioUnit> {
    Box::new(var(&p.get("level")))
}

#[test]
fn a_voice_renders_its_parameter_and_follows_a_change_through_a_clone() {
    let mut voice = Voice::new(level, &[("level", 0.25)]);
    let mut out = [0.0f32; 8];
    voice.fill(&mut out);
    assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    let params = voice.params();
    assert!(params.set("level", 0.5));
    voice.fill(&mut out);
    assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    assert_eq!(params.value("level"), Some(0.5));
}

#[test]
fn an_unknown_parameter_is_refused_not_invented() {
    let voice = Voice::new(level, &[("level", 0.0)]);
    assert!(!voice.params().set("throttle", 1.0));
    assert_eq!(voice.params().value("throttle"), None);
}

#[test]
fn a_rebuilt_voice_shares_the_old_ones_values() {
    let first = Voice::new(level, &[("level", 0.1)]);
    let mut second = Voice::with_params(first.recipe(), first.params());
    first.params().set("level", 0.7);
    let mut out = [0.0f32; 2];
    second.fill(&mut out);
    assert!((out[0] - 0.7).abs() < 1e-6);
}

#[test]
fn a_recipe_that_needs_an_input_renders_silence_instead_of_panicking() {
    fn filter_only(_: &Params) -> Box<dyn AudioUnit> {
        Box::new(lowpass_hz(500.0, 0.7))
    }
    let mut voice = Voice::new(filter_only, &[]);
    let mut out = [1.0f32; 4];
    voice.fill(&mut out);
    assert_eq!(out, [0.0; 4]);
}
