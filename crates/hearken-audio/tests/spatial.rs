use hearken_audio::{place, Listener, SpatialConfig, Vec3};

fn at(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, 0.0)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_sound_on_the_listener_is_full_and_centred() {
    let p = place(
        &Listener::default(),
        Vec3::ZERO,
        false,
        &SpatialConfig::default(),
    );
    assert_eq!((p.gain, p.pan, p.lowpass_hz), (1.0, 0.0, None));
}

#[test]
fn facing_north_east_is_right_and_west_is_left() {
    let cfg = SpatialConfig::default();
    let l = Listener::default();
    assert!(near(place(&l, at(5.0, 0.0), false, &cfg).pan, 1.0));
    assert!(near(place(&l, at(-5.0, 0.0), false, &cfg).pan, -1.0));
    assert!(near(place(&l, at(0.0, 5.0), false, &cfg).pan, 0.0));
}

#[test]
fn turning_to_face_east_puts_north_on_the_left_and_ahead_becomes_east() {
    let cfg = SpatialConfig::default();
    let l = Listener {
        pos: Vec3::ZERO,
        facing: 90.0,
    };
    assert!(near(place(&l, at(0.0, 5.0), false, &cfg).pan, -1.0));
    let ahead = place(&l, at(5.0, 0.0), false, &cfg);
    assert!(near(ahead.pan, 0.0));
    assert!(
        near(ahead.relative.y, 5.0),
        "east is straight ahead: {:?}",
        ahead.relative
    );
}

#[test]
fn gain_falls_with_distance_and_holds_beyond_the_maximum() {
    let cfg = SpatialConfig::default(); // min 1.5, max 30, rolloff 1
    let l = Listener::default();
    assert!(near(place(&l, at(0.0, 1.5), false, &cfg).gain, 1.0));
    assert!(near(place(&l, at(0.0, 3.0), false, &cfg).gain, 0.5));
    let at_max = place(&l, at(0.0, 30.0), false, &cfg).gain;
    assert!(near(place(&l, at(0.0, 300.0), false, &cfg).gain, at_max));
}

#[test]
fn a_sound_behind_is_duller_and_a_little_quieter_but_one_beside_is_not() {
    let cfg = SpatialConfig::default();
    let l = Listener::default();
    let behind = place(&l, at(0.0, -1.0), false, &cfg);
    assert!(near(behind.gain, cfg.behind_gain));
    assert!(near(behind.lowpass_hz.unwrap(), cfg.behind_lowpass_hz));
    assert_eq!(place(&l, at(1.0, 0.0), false, &cfg).lowpass_hz, None);
}

#[test]
fn occlusion_halves_the_gain_and_muffles_to_the_lower_cutoff() {
    let cfg = SpatialConfig::default();
    let l = Listener::default();
    let p = place(&l, at(0.0, 1.0), true, &cfg);
    assert!(near(p.gain, 0.5));
    assert_eq!(p.lowpass_hz, Some(800.0));
    let behind = place(&l, at(0.0, -1.0), true, &cfg);
    assert_eq!(
        behind.lowpass_hz,
        Some(800.0),
        "the lower of the two cutoffs wins"
    );
}

#[test]
fn nonsense_numbers_from_the_game_never_reach_a_backend() {
    let cfg = SpatialConfig::default();
    for pos in [
        Vec3::new(f32::NAN, 0.0, 0.0),
        Vec3::new(f32::INFINITY, 1.0, 0.0),
    ] {
        let p = place(&Listener::default(), pos, true, &cfg);
        assert_eq!((p.gain, p.pan, p.lowpass_hz), (1.0, 0.0, None), "{pos:?}");
        assert_eq!(p.relative, Vec3::ZERO);
    }
    let lost = Listener {
        pos: Vec3::ZERO,
        facing: f32::NAN,
    };
    assert!(
        near(place(&lost, at(5.0, 0.0), false, &cfg).pan, 1.0),
        "a NaN facing counts as north"
    );
    let odd = SpatialConfig {
        min_distance: 0.0,
        max_distance: -1.0,
        ..cfg
    };
    let p = place(&Listener::default(), at(0.0, 2.0), false, &odd);
    assert!(p.gain.is_finite() && p.pan.is_finite());
}
