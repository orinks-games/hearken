use hearken_audio::*;

fn pool() -> (Audio, Recorder) {
    let (backend, rec) = RecordingBackend::new();
    let clips = MemoryClips::default()
        .with("sfx/door", &b"door"[..])
        .with("ui/menu_move", &b"tick"[..]);
    (Audio::new(Box::new(backend), Box::new(clips)), rec)
}

fn no_occlusion(_: Vec3) -> bool {
    false
}

#[test]
fn a_one_shot_plays_its_clip_at_the_bus_volume() {
    let (mut audio, rec) = pool();
    audio.set_bus_volume(Bus::EFFECTS, 0.5);
    let door = audio.play("sfx/door", Bus::EFFECTS);
    assert!(audio.is_playing(door));
    match &rec.calls()[0] {
        Call::Play {
            key,
            looping,
            clip,
            mix,
            ..
        } => {
            assert_eq!((key.as_str(), *looping), ("sfx/door", false));
            assert_eq!(clip.as_deref(), Some(&b"door"[..]));
            assert_eq!((mix.gain, mix.pan, mix.pitch), (0.5, 0.0, 1.0));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_missing_clip_plays_its_default_and_is_named_once_however_often_it_plays() {
    let (mut audio, rec) = pool();
    for _ in 0..100 {
        audio.play("sfx/step", Bus::EFFECTS);
    }
    assert_eq!(audio.missing_clips(), ["sfx/step"]);
    let default = hearken_synth::builtin_wav("sfx/step");
    assert!(rec
        .calls()
        .iter()
        .all(|c| matches!(c, Call::Play { clip: Some(b), .. } if **b == *default)));
    assert_eq!(rec.played().len(), 100);
}

#[test]
fn a_clip_the_backend_cannot_play_falls_back_to_its_default() {
    let (mut audio, rec) = pool();
    rec.fail_next_play("not a sound file");
    audio.play("sfx/door", Bus::EFFECTS);
    assert_eq!(audio.missing_clips(), ["sfx/door"]);
    let default = hearken_synth::builtin_wav("sfx/door");
    assert!(
        matches!(rec.calls().last(), Some(Call::Play { clip: Some(b), .. }) if **b == *default)
    );

    // A key already known bad goes straight to the default: no repeat
    // attempt with the original (still-cached, still-bad) clip bytes.
    rec.clear();
    audio.play("sfx/door", Bus::EFFECTS);
    let calls = rec.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(matches!(&calls[0], Call::Play { clip: Some(b), .. } if **b == *default));
}

#[test]
fn a_refused_voice_returns_none_instead_of_looping_a_fallback_clip() {
    use hearken_synth::fundsp::prelude32::*;
    fn hum(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new(var(&p.get("throttle")))
    }
    let (mut audio, rec) = pool();
    rec.fail_next_play("no voice channels left");
    let v = audio.start_voice(hum, &[("throttle", 0.0)], None, hearken_audio::Bus::EFFECTS);
    assert!(!audio.is_playing(v));
    assert!(audio.missing_clips().is_empty());
    assert!(rec.calls().is_empty(), "{:?}", rec.calls());
}

#[test]
fn a_nan_or_infinite_control_value_leaves_the_previous_one_and_sends_nothing() {
    let (mut audio, rec) = pool();
    let s = audio.play("sfx/door", Bus::EFFECTS);
    rec.clear();
    audio.set_volume(s, f32::NAN);
    audio.set_pitch(s, f32::INFINITY);
    audio.set_bus_volume(Bus::EFFECTS, f32::NAN);
    assert!(rec.calls().is_empty(), "{:?}", rec.calls());
    assert_eq!(audio.bus_volume(Bus::EFFECTS), 1.0);
}

#[test]
fn an_old_handle_does_nothing_to_the_sound_that_took_its_slot() {
    let (mut audio, rec) = pool();
    let old = audio.play("sfx/door", Bus::EFFECTS);
    rec.finish(rec.channel_of("sfx/door").unwrap());
    audio.update(&no_occlusion);
    assert!(!audio.is_playing(old));
    let new = audio.start_loop("ui/menu_move", None, Bus::UI);
    rec.clear();
    audio.set_pitch(old, 2.0);
    audio.stop(old, 0);
    assert!(rec.calls().is_empty(), "{:?}", rec.calls());
    assert!(audio.is_playing(new));
}

#[test]
fn a_burst_of_one_shots_leaves_nothing_behind_once_they_end() {
    let (mut audio, rec) = pool();
    for _ in 0..1000 {
        audio.play("ui/menu_move", Bus::UI);
    }
    for call in rec.calls() {
        if let Call::Play { channel, .. } = call {
            rec.finish(channel);
        }
    }
    audio.update(&no_occlusion);
    assert!(audio.live().is_empty());
}

#[test]
fn a_placed_sound_follows_the_listener_and_is_only_re_mixed_on_a_change() {
    let (mut audio, rec) = pool();
    audio.play_at("sfx/door", Vec3::new(5.0, 0.0, 0.0), Bus::EFFECTS);
    let ch = rec.channel_of("sfx/door").unwrap();
    assert!((rec.last_mix(ch).unwrap().pan - 1.0).abs() < 1e-4);
    audio.set_listener(Vec3::ZERO, 180.0);
    audio.update(&no_occlusion);
    assert!(
        (rec.last_mix(ch).unwrap().pan + 1.0).abs() < 1e-4,
        "facing south, east is on the left"
    );
    rec.clear();
    audio.update(&no_occlusion);
    assert!(rec.calls().is_empty(), "nothing changed, nothing sent");
}

#[test]
fn the_games_occlusion_answer_reaches_the_mix() {
    let (mut audio, rec) = pool();
    audio.play_at("sfx/door", Vec3::new(0.0, 1.0, 0.0), Bus::EFFECTS);
    audio.update(&|_| true);
    let mix = rec.last_mix(rec.channel_of("sfx/door").unwrap()).unwrap();
    assert_eq!((mix.gain, mix.lowpass_hz), (0.5, Some(800.0)));
}

#[test]
fn a_bus_volume_reaches_only_its_own_sounds_and_master_reaches_all() {
    let (mut audio, rec) = pool();
    audio.play("sfx/door", Bus::EFFECTS);
    audio.play("ui/menu_move", Bus::UI);
    let (door, tick) = (
        rec.channel_of("sfx/door").unwrap(),
        rec.channel_of("ui/menu_move").unwrap(),
    );
    audio.set_bus_volume(Bus::EFFECTS, 0.25);
    assert_eq!(rec.last_mix(door).unwrap().gain, 0.25);
    assert_eq!(rec.last_mix(tick).unwrap().gain, 1.0);
    audio.set_bus_volume(Bus::MASTER, 0.5);
    assert_eq!(rec.last_mix(door).unwrap().gain, 0.125);
    assert_eq!(rec.last_mix(tick).unwrap().gain, 0.5);
}

#[test]
fn a_voice_takes_its_parameters_by_name() {
    use hearken_synth::fundsp::prelude32::*;
    fn hum(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new((var(&p.get("throttle")) * 100.0) >> sine())
    }
    let (mut audio, _rec) = pool();
    let v = audio.start_voice(hum, &[("throttle", 0.0)], None, hearken_audio::Bus::EFFECTS);
    assert!(audio.set_voice_param(v, "throttle", 0.8));
    assert!(!audio.set_voice_param(v, "brake", 1.0));
    let live = audio.live();
    assert_eq!(
        live[0].params.as_ref().unwrap().value("throttle"),
        Some(0.8)
    );
    assert!(live[0].looping);
}

#[test]
fn stopping_passes_the_fade_to_the_backend_and_frees_the_handle() {
    let (mut audio, rec) = pool();
    let s = audio.start_loop("sfx/door", None, Bus::EFFECTS);
    audio.stop(s, 300);
    assert!(matches!(
        rec.calls().last(),
        Some(Call::Stop { fade_ms: 300, .. })
    ));
    assert!(!audio.is_playing(s));
}

#[test]
fn a_released_loop_plays_out_and_frees_its_handle_when_it_ends() {
    let (mut audio, rec) = pool();
    let horn = audio.start_loop("sfx/door", None, Bus::EFFECTS);
    let channel = rec.channel_of("sfx/door").unwrap();
    audio.release(horn, 150);
    assert!(matches!(
        rec.calls().last(),
        Some(Call::Release { fade_ms: 150, .. })
    ));
    audio.update(&no_occlusion);
    assert!(audio.is_playing(horn), "the tail is still playing");
    assert!(!audio.live()[0].looping, "and it no longer loops");
    rec.finish(channel);
    audio.update(&no_occlusion);
    assert!(!audio.is_playing(horn), "freed once the tail ends");
}

#[test]
fn releasing_a_one_shot_or_releasing_twice_sends_nothing_more() {
    let (mut audio, rec) = pool();
    let door = audio.play("sfx/door", Bus::EFFECTS);
    audio.release(door, 0);
    let horn = audio.start_loop("sfx/door", None, Bus::EFFECTS);
    audio.release(horn, 0);
    audio.release(horn, 0);
    let releases = rec
        .calls()
        .iter()
        .filter(|c| matches!(c, Call::Release { .. }))
        .count();
    assert_eq!(releases, 1);
}

#[test]
fn a_loop_region_reaches_the_backend_and_a_bad_one_is_ignored() {
    let (mut audio, rec) = pool();
    audio.start_loop_region("sfx/door", LoopRegion::new(0.5, 1.25), None, Bus::EFFECTS);
    assert!(matches!(
        rec.calls().last(),
        Some(Call::SetLoopRegion { region, .. }) if *region == LoopRegion::new(0.5, 1.25)
    ));
    rec.clear();
    let looped = audio.start_loop_region("sfx/door", LoopRegion::new(2.0, 1.0), None, Bus::EFFECTS);
    assert!(audio.is_playing(looped), "it still loops, the whole clip");
    assert!(!rec
        .calls()
        .iter()
        .any(|c| matches!(c, Call::SetLoopRegion { .. })));
}

#[test]
fn has_clip_answers_without_playing_or_noting_a_miss() {
    let (mut audio, rec) = pool();
    assert!(audio.has_clip("sfx/door"));
    assert!(!audio.has_clip("sfx/horn"));
    assert!(rec.calls().is_empty());
    assert!(audio.missing_clips().is_empty());
}
