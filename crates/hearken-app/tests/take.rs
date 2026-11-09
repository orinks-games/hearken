//! A take: a run's music recorded as the calls that made it, held while a
//! screen covers the run, replayed as the same calls, and saved as a file.

mod common;

use std::rc::Rc;
use std::time::{Duration, Instant};

use common::{Probe, TestGame};
use hearken_app::audio::Start;
use hearken_app::screens::PauseScreen;
use hearken_app::soundtrack::AlbumOptions;
use hearken_app::{
    Bus, Call, Headless, InputEvent, Key, Mods, Performer, Replay, SoundtrackScreen, Take,
    TakeEvent, TakePerformer, TakeState, Track, Vec3,
};

fn game() -> Headless<TestGame> {
    let mut h = Headless::new(Default::default(), Probe::new("run"));
    h.frame(0.0);
    h.clear();
    h
}

/// A call as a replay should repeat it: the channel left out, since a
/// second pool hands out its own.
fn shape(call: &Call) -> String {
    match call {
        Call::Play {
            key, looping, mix, ..
        } => format!(
            "play {key} loop={looping} gain={:.3} pan={:.3} pitch={:.3}",
            mix.gain, mix.pan, mix.pitch
        ),
        Call::SetMix { mix, ramp_ms, .. } => format!(
            "mix gain={:.3} pan={:.3} pitch={:.3} over {ramp_ms}",
            mix.gain, mix.pan, mix.pitch
        ),
        Call::SetMixRamped { mix, .. } => {
            format!(
                "mix gain={:.3} pan={:.3} pitch={:.3} ramped",
                mix.gain, mix.pan, mix.pitch
            )
        }
        Call::Stop { fade_ms, .. } => format!("stop over {fade_ms}"),
        Call::Pause { paused, .. } => format!("pause {paused}"),
        Call::Release { fade_ms, .. } => format!("release over {fade_ms}"),
        Call::SetLoopRegion { region, .. } => format!("region {region:?}"),
    }
}

/// The backend calls that touched the music, in order.
fn music_calls(h: &Headless<TestGame>) -> Vec<String> {
    h.sounds
        .calls()
        .iter()
        .filter(|call| !matches!(call, Call::Play { key, .. } if !key.starts_with("music/")))
        .map(shape)
        .collect()
}

#[test]
fn a_take_follows_its_buses_and_the_state_is_logged() {
    let mut h = game();
    assert_eq!(h.app.ctx.take_state(), TakeState::Idle);
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    assert_eq!(h.app.ctx.take_state(), TakeState::Recording);
    let bed = h.app.ctx.audio.start_loop("music/bed", None, Bus::MUSIC);
    h.app.ctx.audio.play("ui/tick", Bus::EFFECTS);
    h.frame(0.5);
    h.app.ctx.audio.set_volume_over(bed, 0.3, 200);
    h.frame(0.5);
    h.app.ctx.audio.stop(bed, 400);
    h.frame(0.1);
    let take = h.app.ctx.end_take().expect("a take");
    assert_eq!(h.app.ctx.take_state(), TakeState::Idle);
    assert!(h.app.ctx.end_take().is_none());

    assert_eq!(take.buses, vec!["music".to_string()]);
    assert!((take.length_s - 1.1).abs() < 0.01, "{}", take.length_s);
    assert!(!take.cut);
    assert_eq!(take.starts(), 1);
    let kinds: Vec<&str> = take
        .events
        .iter()
        .map(|e| match e {
            TakeEvent::Start { key, .. } => key.as_str(),
            TakeEvent::Volume { .. } => "volume",
            TakeEvent::Stop { .. } => "stop",
            TakeEvent::Listener { .. } => "listener",
            _ => "other",
        })
        .collect();
    // The listener's place first, so the first sounds land where they did.
    assert_eq!(kinds, vec!["listener", "music/bed", "volume", "stop"]);
    assert!(matches!(
        take.events[2],
        TakeEvent::Volume { value, ramp_ms: 200, .. } if (value - 0.3).abs() < 0.001
    ));
    assert!(matches!(
        take.events[3],
        TakeEvent::Stop { fade_ms: 400, .. }
    ));
    assert_eq!(
        h.changes()
            .iter()
            .filter(|l| l.starts_with("[take]"))
            .collect::<Vec<_>>(),
        vec!["[take] idle -> recording", "[take] recording -> ended"]
    );
}

#[test]
fn a_covering_screen_holds_the_take_and_leaves_no_gap() {
    let mut h = game();
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    let bed = h.app.ctx.audio.start_loop("music/bed", None, Bus::MUSIC);
    h.frame(1.0);
    h.app.ctx.push(Probe::new("pause"));
    h.frame(0.0);
    assert_eq!(h.app.ctx.take_state(), TakeState::Held);
    // Time under the pause and anything started there are left out.
    h.frame(5.0);
    let menu = h.app.ctx.audio.play("music/menu", Bus::MUSIC);
    h.frame(0.5);
    h.app.ctx.audio.stop(menu, 0);
    h.app.ctx.pop();
    h.frame(0.0);
    assert_eq!(h.app.ctx.take_state(), TakeState::Recording);
    h.frame(1.0);
    h.app.ctx.audio.stop(bed, 0);
    let take = h.app.ctx.end_take().expect("a take");
    assert!((take.length_s - 2.0).abs() < 0.01, "{}", take.length_s);
    assert_eq!(take.starts(), 1);
    let stop_at = take
        .events
        .iter()
        .find_map(|e| match e {
            TakeEvent::Stop { t, .. } => Some(*t),
            _ => None,
        })
        .expect("the stop");
    assert!((stop_at - 2.0).abs() < 0.01, "{stop_at}");
    assert_eq!(
        h.changes()
            .iter()
            .filter(|l| l.starts_with("[take]"))
            .collect::<Vec<_>>(),
        vec![
            "[take] idle -> recording",
            "[take] recording -> held",
            "[take] held -> recording",
            "[take] recording -> ended"
        ]
    );
}

#[test]
fn a_screen_that_leaves_without_ending_its_take_drops_it() {
    let mut h = game();
    h.app.ctx.push(Probe::new("play"));
    h.frame(0.0);
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    h.app.ctx.audio.play("music/bed", Bus::MUSIC);
    h.frame(0.5);
    h.app.ctx.pop();
    h.frame(0.0);
    assert_eq!(h.app.ctx.take_state(), TakeState::Idle);
    assert!(h.app.ctx.end_take().is_none());
    assert!(h
        .changes()
        .contains(&"[take] recording -> idle".to_string()));
}

#[test]
fn a_take_replays_as_the_same_calls_and_a_positioned_part_lands_where_it_did() {
    let mut h = game();
    h.app.ctx.audio.set_listener(Vec3::new(0.0, 0.0, 0.0), 0.0);
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    let bed = h.app.ctx.audio.start_loop("music/bed", None, Bus::MUSIC);
    h.frame(0.5);
    let bell = h
        .app
        .ctx
        .audio
        .start_loop("music/bell", Some(Vec3::new(4.0, 0.0, 1.0)), Bus::MUSIC);
    h.frame(0.5);
    h.app.ctx.audio.set_volume_over(bed, 0.4, 300);
    h.app.ctx.audio.set_pitch_over(bell, 1.2, 100);
    h.frame(0.5);
    h.app.ctx.audio.set_listener(Vec3::new(2.0, 0.0, 0.0), 0.0);
    h.frame(0.5);
    h.app.ctx.audio.release(bell, 250);
    h.frame(0.2);
    // The bell's tail ends, as it would on a real backend.
    h.sounds.finish(h.sounds.channel_of("music/bell").unwrap());
    h.frame(0.3);
    h.app.ctx.audio.stop(bed, 600);
    h.frame(0.2);
    let take = h.app.ctx.end_take().expect("a take");
    let heard = music_calls(&h);
    assert!(
        heard.iter().any(|c| c.starts_with("play music/bell")),
        "{heard:?}"
    );
    // The bell off to one side came out panned, by the pool's own ears.
    let bell_play = heard
        .iter()
        .find(|c| c.starts_with("play music/bell"))
        .unwrap();
    assert!(
        !bell_play.contains("pan=0.000") && !bell_play.contains("pan=-0.000"),
        "{bell_play}"
    );

    // The same take through a fresh pool, with the listener somewhere else.
    let take = Rc::new(Take::from_json(&take.to_json()).unwrap());
    let mut h2 = game();
    h2.app
        .ctx
        .audio
        .set_listener(Vec3::new(-9.0, 0.0, 0.0), 1.0);
    let mut performer = TakePerformer::new(take.clone());
    assert_eq!(performer.state(), Replay::Waiting);
    performer.start(&mut h2.app.ctx.audio);
    assert_eq!(performer.state(), Replay::Playing);
    let mut clock = 0.0;
    while performer.update(&mut h2.app.ctx.audio, 0.05) {
        h2.frame(0.05);
        clock += 0.05;
        assert!(clock < 30.0, "the replay never ended");
        // Once released, the bell's tail ends here too.
        if let Some(channel) = h2.sounds.channel_of("music/bell") {
            let released = h2
                .sounds
                .calls()
                .iter()
                .any(|c| matches!(c, Call::Release { channel: ch, .. } if *ch == channel));
            if released && h2.sounds.is_playing(channel) {
                h2.sounds.finish(channel);
            }
        }
    }
    assert_eq!(performer.state(), Replay::Done);
    let replayed = music_calls(&h2);
    assert_eq!(replayed, heard);
    // The listener is back where this pool had it.
    let listener = h2.app.ctx.audio.listener();
    assert_eq!(listener.pos, Vec3::new(-9.0, 0.0, 0.0));
    assert_eq!(listener.facing, 1.0);
}

/// The frames and loudest sample of an Ogg Vorbis file.
fn ogg_length_and_peak(path: &std::path::Path) -> (f64, f32) {
    let file = std::fs::File::open(path).unwrap();
    let mut decoder = vorbis_rs::VorbisDecoder::new(file).unwrap();
    assert_eq!(decoder.channels().get(), 2);
    let mut frames = 0;
    let mut loudest = 0.0f32;
    while let Some(block) = decoder.decode_audio_block().unwrap() {
        let samples = block.samples();
        frames += samples[0].len();
        for channel in samples {
            for s in channel.iter() {
                loudest = loudest.max(s.abs());
            }
        }
    }
    (
        frames as f64 / hearken_app::soundtrack::export::RATE as f64,
        loudest,
    )
}

#[test]
fn shift_and_enter_saves_one_take_as_a_file_of_its_length() {
    // Record a short run.
    let mut h = game();
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    let bed = h.app.ctx.audio.start_with(
        "music/bed",
        Bus::MUSIC,
        Start {
            looping: true,
            ..Start::ONCE
        },
    );
    h.frame(1.0);
    h.app.ctx.audio.set_volume_over(bed, 0.5, 200);
    h.frame(1.0);
    h.app.ctx.audio.stop(bed, 300);
    h.frame(0.1);
    let take = h.app.ctx.end_take().expect("a take");
    assert!((take.length_s - 2.1).abs() < 0.01);

    // Keep it as a track of the "Runs" album and save it from the screen.
    let dir = tempfile::tempdir().unwrap();
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app.ctx.soundtrack.set_tracks(vec![
        Track::take("run-1", "Morning run", take).album("Runs"),
        Track::clip("theme", "Theme", "music/theme").album("Runs"),
    ]);
    h.app.ctx.soundtrack.set_album(
        "Runs",
        AlbumOptions {
            export: true,
            vary_export: false,
        },
    );
    h.app
        .ctx
        .soundtrack
        .set_export_dir(Some(dir.path().to_path_buf()));
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new().album("Runs"));
    h.frame(0.0);
    // The take is open from the start; the row tells how to save it.
    h.clear();
    h.press(Key::F1);
    assert!(
        h.spoken()
            .iter()
            .any(|l| l.contains("Shift and Enter saves it as a sound file.")),
        "{:?}",
        h.spoken()
    );
    h.clear();
    h.send(InputEvent::key_mods(Key::Return, Mods::SHIFT));
    assert_eq!(h.spoken(), vec!["Saving Morning run. Escape stops it."]);
    let began = Instant::now();
    while !h.spoken().iter().any(|l| l.starts_with("Saved ")) {
        h.frame(0.016);
        assert!(
            began.elapsed() < Duration::from_secs(120),
            "the save never ended: {:?}",
            h.spoken()
        );
    }
    assert!(
        h.spoken()
            .iter()
            .any(|l| l.starts_with("Saved Morning run to ") && l.contains("Runs")),
        "{:?}",
        h.spoken()
    );
    let folder = dir.path().join("Test Game").join("Runs");
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["01 Morning run.ogg"]);
    let (seconds, loudest) = ogg_length_and_peak(&folder.join("01 Morning run.ogg"));
    // The run, its ring-out and the exporter's tail; not a whole album's
    // worth of silence.
    assert!(seconds > 2.1 && seconds < 8.0, "{seconds}");
    assert!(loudest > 0.01, "loudest sample {loudest}");

    // Saving a locked track, or with no take row, does nothing.
    h.app.ctx.pop();
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new().album("Runs"));
    h.frame(0.0);
    h.press(Key::Down);
    h.clear();
    h.send(InputEvent::key_mods(Key::Return, Mods::SHIFT));
    assert!(h.spoken().is_empty(), "{:?}", h.spoken());
}

#[test]
fn escape_stops_a_save_and_says_so() {
    let mut h = game();
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    let bed = h.app.ctx.audio.start_loop("music/bed", None, Bus::MUSIC);
    h.frame(30.0);
    h.app.ctx.audio.stop(bed, 0);
    let take = h.app.ctx.end_take().expect("a take");

    let dir = tempfile::tempdir().unwrap();
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app
        .ctx
        .soundtrack
        .set_tracks(vec![Track::take("run-1", "Long run", take).album("Runs")]);
    h.app.ctx.soundtrack.set_album(
        "Runs",
        AlbumOptions {
            export: true,
            vary_export: false,
        },
    );
    h.app
        .ctx
        .soundtrack
        .set_export_dir(Some(dir.path().to_path_buf()));
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new().album("Runs"));
    h.frame(0.0);
    h.send(InputEvent::key_mods(Key::Return, Mods::SHIFT));
    h.frame(0.016);
    h.clear();
    h.send(InputEvent::key_mods(Key::Return, Mods::SHIFT));
    assert_eq!(h.spoken(), vec!["Still saving."]);
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), vec!["Saving stopped."]);
    let folder = dir.path().join("Test Game").join("Runs");
    let leftovers = std::fs::read_dir(&folder).map(|d| d.count()).unwrap_or(0);
    assert_eq!(leftovers, 0, "a part file was left behind");
}

#[test]
fn a_pause_menu_over_the_run_leaves_no_pause_in_the_take() {
    let mut h = game();
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    let bed = h.app.ctx.audio.start_loop("music/bed", None, Bus::MUSIC);
    h.frame(1.0);
    // The pause screen pauses every sound on arrival and resumes them on
    // leaving; the take is held before either happens.
    h.app.ctx.push(PauseScreen::new("", |_ctx| {}));
    h.frame(0.0);
    assert_eq!(h.app.ctx.take_state(), TakeState::Held);
    assert!(h.app.ctx.audio.is_paused(bed));
    h.frame(3.0);
    h.app.ctx.pop();
    h.frame(0.0);
    assert!(!h.app.ctx.audio.is_paused(bed));
    h.frame(1.0);
    h.app.ctx.audio.stop(bed, 0);
    let take = h.app.ctx.end_take().expect("a take");
    assert!(
        !take
            .events
            .iter()
            .any(|e| matches!(e, TakeEvent::Pause { .. })),
        "{:?}",
        take.events
    );
    assert!((take.length_s - 2.0).abs() < 0.01, "{}", take.length_s);
    // The game's own pause is still in the take.
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    let bed = h.app.ctx.audio.start_loop("music/bed", None, Bus::MUSIC);
    h.frame(0.5);
    h.app.ctx.audio.pause(bed);
    h.app.ctx.audio.pause(bed);
    h.frame(0.5);
    h.app.ctx.audio.resume(bed);
    h.frame(0.1);
    let take = h.app.ctx.end_take().expect("a take");
    let pauses: Vec<bool> = take
        .events
        .iter()
        .filter_map(|e| match e {
            TakeEvent::Pause { paused, .. } => Some(*paused),
            _ => None,
        })
        .collect();
    assert_eq!(pauses, vec![true, false]);
}

#[test]
fn a_screen_opened_to_save_a_track_starts_saving_on_arrival() {
    let mut h = game();
    h.app.ctx.begin_take(&[Bus::MUSIC]);
    let bed = h.app.ctx.audio.start_loop("music/bed", None, Bus::MUSIC);
    h.frame(2.0);
    h.app.ctx.audio.stop(bed, 0);
    let take = h.app.ctx.end_take().expect("a take");

    let dir = tempfile::tempdir().unwrap();
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app.ctx.soundtrack.set_tracks(vec![
        Track::clip("theme", "Theme", "music/theme").album("Runs"),
        Track::take("run-1", "Morning run", take).album("Runs"),
    ]);
    h.app.ctx.soundtrack.set_album(
        "Runs",
        AlbumOptions {
            export: true,
            vary_export: false,
        },
    );
    h.app
        .ctx
        .soundtrack
        .set_export_dir(Some(dir.path().to_path_buf()));
    h.frame(0.0);
    h.clear();
    h.app
        .ctx
        .push(SoundtrackScreen::new().album("Runs").saving("run-1"));
    h.frame(0.0);
    assert_eq!(h.spoken()[0], "Saving Morning run. Escape stops it.");
    let began = Instant::now();
    while !h.spoken().iter().any(|l| l.starts_with("Saved ")) {
        h.frame(0.016);
        assert!(
            began.elapsed() < Duration::from_secs(120),
            "{:?}",
            h.spoken()
        );
    }
    let folder = dir.path().join("Test Game").join("Runs");
    assert!(folder.join("02 Morning run.ogg").is_file());

    // A track that cannot be saved opens the screen as usual.
    h.app.ctx.pop();
    h.frame(0.0);
    h.clear();
    h.app
        .ctx
        .push(SoundtrackScreen::new().album("Runs").saving("theme"));
    h.frame(0.0);
    assert!(
        h.spoken()[0].starts_with("Runs soundtrack."),
        "{:?}",
        h.spoken()
    );
}
