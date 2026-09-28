use hearken_audio::*;

fn pool() -> (Audio, Recorder) {
    let (backend, rec) = RecordingBackend::new();
    (
        Audio::new(Box::new(backend), Box::new(MemoryClips::default())),
        rec,
    )
}

#[test]
fn a_new_bed_fades_in_over_the_old_one_fading_out_and_the_same_bed_does_nothing() {
    let (mut audio, rec) = pool();
    audio.set_bed(Some("amb/harbour"));
    let first = rec.channel_of("amb/harbour").unwrap();
    rec.clear();
    audio.set_bed(Some("amb/harbour"));
    assert!(rec.calls().is_empty());
    audio.set_bed(Some("amb/market"));
    let calls = rec.calls();
    assert!(calls.contains(&Call::Stop {
        channel: first,
        fade_ms: 800
    }));
    let second = rec.channel_of("amb/market").unwrap();
    assert!(
        matches!(calls.iter().find(|c| matches!(c, Call::Play { channel, .. } if *channel == second)),
        Some(Call::Play { looping: true, mix, .. }) if mix.gain == 0.0)
    );
    assert!(calls.iter().any(|c| matches!(c, Call::SetMix { channel, ramp_ms: 800, mix } if *channel == second && mix.gain == 1.0)));
    assert_eq!(audio.bed(), Some("amb/market"));
    audio.set_bed(None);
    assert_eq!(audio.bed(), None);
}

#[test]
fn music_asked_for_again_carries_on_and_new_music_replaces_it() {
    let (mut audio, rec) = pool();
    audio.play_music(Some("music/title"), true);
    let title = rec.channel_of("music/title").unwrap();
    rec.clear();
    audio.play_music(Some("music/title"), true);
    assert!(rec.calls().is_empty());
    audio.play_music(Some("music/game"), false);
    assert!(rec.calls().contains(&Call::Stop {
        channel: title,
        fade_ms: 500
    }));
    assert_eq!(audio.music(), Some("music/game"));
}

#[test]
fn pause_all_pauses_what_was_playing_and_resume_all_resumes_only_that() {
    let (mut audio, rec) = pool();
    let a = audio.start_loop("sfx/a", None, Bus::EFFECTS);
    let b = audio.start_loop("sfx/b", None, Bus::EFFECTS);
    audio.pause(b);
    audio.pause_all();
    let menu = audio.play("ui/menu_move", Bus::UI);
    audio.resume_all();
    let resumed: Vec<_> = rec
        .calls()
        .into_iter()
        .filter(|c| matches!(c, Call::Pause { paused: false, .. }))
        .collect();
    assert_eq!(
        resumed.len(),
        1,
        "only a: b was paused by the game, the menu sound never was"
    );
    assert!(audio.is_playing(a) && audio.is_playing(b) && audio.is_playing(menu));
}

#[test]
fn the_duck_lowers_only_its_buses() {
    let (mut audio, rec) = pool();
    audio.set_duck_buses(&[Bus::MUSIC]);
    audio.play_music(Some("music/x"), true);
    audio.play("ui/menu_move", Bus::UI);
    audio.set_duck(0.5);
    assert_eq!(
        rec.last_mix(rec.channel_of("music/x").unwrap())
            .unwrap()
            .gain,
        0.5
    );
    assert_eq!(
        rec.last_mix(rec.channel_of("ui/menu_move").unwrap())
            .unwrap()
            .gain,
        1.0
    );
    audio.set_duck(1.0);
    assert_eq!(
        rec.last_mix(rec.channel_of("music/x").unwrap())
            .unwrap()
            .gain,
        1.0
    );
}

/// Task 3's review carry-over: gain used to be computed in two places (the
/// `start`/`resend` path through `Audio::bus_gain`, and `update`'s own
/// `bus_gain_of` call), so a positioned sound on a ducked bus could flip
/// between ducked and unducked depending on which path last touched it.
/// Both now go through the same `Mixing`-based `bus_gain_of`; this pins that
/// an `update` triggered by the listener moving keeps the duck applied.
#[test]
fn a_positioned_sound_on_a_ducked_bus_keeps_its_ducked_gain_across_an_update_after_the_listener_moves(
) {
    let (mut audio, rec) = pool();
    audio.set_duck_buses(&[Bus::EFFECTS]);
    audio.set_duck(0.5);
    let pos = Vec3::new(5.0, 0.0, 0.0);
    audio.play_at("sfx/loop", pos, Bus::EFFECTS);
    let ch = rec.channel_of("sfx/loop").unwrap();
    let before = rec.last_mix(ch).unwrap().gain;
    assert!(before < 1.0, "the very first mix should already be ducked");

    audio.set_listener(Vec3::new(1.0, 0.0, 0.0), 90.0);
    audio.update(&|_| false);

    let placement = place(&audio.listener(), pos, false, &audio.spatial);
    let expected = 0.5 * placement.gain;
    let after = rec.last_mix(ch).unwrap().gain;
    assert!(
        (after - expected).abs() < 1e-5,
        "still ducked after update re-mixed for the moved listener: got {after}, expected {expected}"
    );
}
