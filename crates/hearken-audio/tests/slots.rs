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

/// Task 4 review, round 1, finding 2: the same-key check used to compare
/// against the raw stored key, not a liveness-filtered one, so a bed whose
/// handle already died could never be restarted with the same key.
#[test]
fn a_bed_can_be_restarted_with_the_same_key_once_its_handle_died() {
    let (mut audio, rec) = pool();
    audio.set_bed(Some("amb/harbour"));
    let ch = rec.channel_of("amb/harbour").unwrap();
    rec.finish(ch);
    audio.update(&|_| false);
    assert_eq!(audio.bed(), None, "the bed's handle died and was reaped");
    rec.clear();
    audio.set_bed(Some("amb/harbour"));
    assert!(
        rec.played().contains(&"amb/harbour".to_string()),
        "restarted, not treated as already playing: {:?}",
        rec.played()
    );
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

/// Same finding as the bed's: the same-key-and-loop-flag check must use the
/// liveness-filtered `music()`, not the raw stored key, so a track that
/// already ended can be restarted with the same key.
#[test]
fn music_can_be_restarted_with_the_same_key_and_loop_flag_once_its_handle_died() {
    let (mut audio, rec) = pool();
    audio.play_music(Some("music/title"), false);
    let ch = rec.channel_of("music/title").unwrap();
    rec.finish(ch);
    audio.update(&|_| false);
    assert_eq!(audio.music(), None, "the track ended and was reaped");
    rec.clear();
    audio.play_music(Some("music/title"), false);
    assert!(
        rec.played().contains(&"music/title".to_string()),
        "restarted, not treated as already playing: {:?}",
        rec.played()
    );
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

/// An explicit `pause`/`resume` after `pause_all` must clear `paused_by_all`,
/// or a sound the game re-paused itself would be swept up by the next
/// `resume_all` even though `pause_all` never touched it that time.
#[test]
fn a_sound_the_game_re_pauses_after_pause_all_is_not_swept_up_by_the_next_resume_all() {
    let (mut audio, rec) = pool();
    let a = audio.start_loop("sfx/a", None, Bus::EFFECTS);
    audio.pause_all();
    audio.resume(a); // the game's own doing, not resume_all's
    audio.pause(a); // the game re-pauses it itself
    rec.clear();
    audio.resume_all();
    assert!(
        rec.calls().is_empty(),
        "resume_all must not touch a sound the game paused itself: {:?}",
        rec.calls()
    );
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

/// `set_duck_buses` must resend sounds on a bus that enters or leaves the
/// set while a duck is already active, or they're left at a stale gain
/// until something else happens to touch them.
#[test]
fn changing_the_ducked_buses_resends_the_sounds_that_enter_or_leave_the_set() {
    let (mut audio, rec) = pool();
    audio.set_duck_buses(&[Bus::MUSIC]);
    audio.play_music(Some("music/x"), true);
    audio.play("ui/menu_move", Bus::UI);
    audio.set_duck(0.5);
    let (music_ch, ui_ch) = (
        rec.channel_of("music/x").unwrap(),
        rec.channel_of("ui/menu_move").unwrap(),
    );

    // ui/menu_move's bus enters the set: it should duck immediately, not
    // wait for the next `set_duck` or a positional re-mix.
    audio.set_duck_buses(&[Bus::MUSIC, Bus::UI]);
    assert_eq!(rec.last_mix(ui_ch).unwrap().gain, 0.5);

    // music's bus leaves the set: it should return to full volume
    // immediately, not stay ducked.
    audio.set_duck_buses(&[Bus::UI]);
    assert_eq!(rec.last_mix(music_ch).unwrap().gain, 1.0);
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
