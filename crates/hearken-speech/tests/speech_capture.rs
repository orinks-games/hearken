//! The `CaptureSpeech` / `NullSpeech` contract, `SpeechSettings`, and the
//! `SpeechEnv` / `SharedCapture` surface a game builds its own test harness
//! on. Backend selection and the live `Speech` object are covered in
//! `speech_backends.rs`; the checks against the real Prism on this machine
//! are in `speech_live.rs`.

use hearken_speech::{
    apply_speech_settings, CaptureSpeech, NullSpeech, SpeechChannel, SpeechSettings, SpeechSink,
    SpokenMessage, REFRESH_INTERVAL_S,
};

fn spoken(lines: &[(&str, bool)]) -> Vec<(String, bool)> {
    lines
        .iter()
        .map(|(text, interrupt)| (text.to_string(), *interrupt))
        .collect()
}

// -- CaptureSpeech: the query/record contract ---------------------------------------

#[test]
fn capture_records_both_channels_in_order_with_interrupt() {
    let mut capture = CaptureSpeech::new();
    capture.say("Main menu.", true);
    capture.say_event("Brake now.", true);
    capture.say("Settings.", false);
    assert_eq!(
        capture.lines(),
        vec!["Main menu.", "Brake now.", "Settings."]
    );
    assert_eq!(capture.main_lines(), vec!["Main menu.", "Settings."]);
    assert_eq!(capture.event_lines(), vec!["Brake now."]);
    assert_eq!(
        capture.calls(SpeechChannel::Main),
        spoken(&[("Main menu.", true), ("Settings.", false)])
    );
    assert_eq!(
        capture.tagged(),
        vec![
            ("main", "Main menu.".to_string()),
            ("event", "Brake now.".to_string()),
            ("main", "Settings.".to_string())
        ]
    );
    let sequences: Vec<u64> = capture.entries().iter().map(|e| e.sequence).collect();
    assert_eq!(sequences, vec![0, 1, 2]);
    assert_eq!(
        capture.transcript(),
        "Main menu.\n[event] Brake now.\nSettings."
    );
    capture.clear();
    assert!(capture.lines().is_empty());
    assert!(capture.transcript().is_empty());
}

#[test]
fn capture_resolves_spoken_messages_by_its_terse_flag() {
    let pair = SpokenMessage::with_terse("Arriving at the station.", "Arrived.");
    let silent_when_terse = SpokenMessage::with_terse("Nice weather today.", "");

    let mut normal = CaptureSpeech::new();
    normal.say_message(&pair, true);
    normal.say_event_message(&silent_when_terse, false);
    assert_eq!(
        normal.lines(),
        vec!["Arriving at the station.", "Nice weather today."]
    );

    let mut terse = CaptureSpeech::new().terse();
    terse.say_message(&pair, true);
    terse.say_event_message(&silent_when_terse, false); // dropped, as the real method drops it
    assert_eq!(terse.lines(), vec!["Arrived."]);
    assert!(terse.is_terse());
}

#[test]
fn capture_prefix_marks_every_line() {
    let mut capture = CaptureSpeech::new().with_prefix("[menu] ");
    capture.say("Main menu.", true);
    capture.say_event("Brake now.", true);
    assert_eq!(
        capture.lines(),
        vec!["[menu] Main menu.", "[menu] Brake now."]
    );
    assert!(capture.contains("Brake"));
}

#[test]
fn capture_drops_empty_text_like_the_real_channel() {
    let mut capture = CaptureSpeech::new();
    capture.say("", true);
    capture.say_event("", false);
    assert!(capture.entries().is_empty());
}

#[test]
fn capture_default_answers_as_if_speech_were_disabled() {
    // With speech disabled a real sink has no context: no voice, no
    // separate event voice, nothing adjustable. Most transcript-preserving
    // tests are recorded against those answers.
    let mut capture = CaptureSpeech::new();
    assert!(!capture.available());
    assert_eq!(capture.backend_name(), "none");
    assert!(!capture.has_separate_event_voice());
    assert_eq!(capture.event_backend_name(), "none");
    assert!(!capture.supports_rate() && !capture.supports_pitch() && !capture.supports_volume());
    assert!(!capture.event_supports_rate());
    assert!(capture.event_backend_options().is_empty());
    assert!(capture.voice_names().is_empty());
    assert!(!capture.refresh(true));
    capture.select_event_backend(Some("SAPI"));
    assert!(!capture.has_separate_event_voice()); // nothing to bind headless
    assert_eq!(capture.event_backend_name(), "none");
    capture.poll(REFRESH_INTERVAL_S);
    capture.request_refresh();
    assert_eq!(capture.refresh_requests(), 1);
    capture.shutdown();
    assert_eq!(capture.shutdown_calls(), 1);
}

#[test]
fn capture_full_voice_answers_like_nvda_plus_sapi() {
    let mut capture = CaptureSpeech::full_voice();
    assert!(capture.available());
    assert_eq!(capture.backend_name(), "NVDA");
    assert!(capture.has_separate_event_voice());
    assert_eq!(capture.event_backend_name(), "SAPI");
    assert!(capture.supports_rate() && capture.supports_pitch() && capture.supports_volume());
    assert!(capture.event_supports_rate());
    assert_eq!(capture.event_backend_options(), vec!["OneCore", "SAPI"]);
    assert_eq!(capture.voice_names(), vec!["David", "Zira"]);
    capture.select_event_backend(None); // events on the main voice
    assert!(!capture.has_separate_event_voice());
    assert_eq!(capture.event_backend_name(), "none");
    assert!(!capture.event_supports_rate());
    capture.select_event_backend(Some("OneCore"));
    assert!(capture.has_separate_event_voice());
    assert_eq!(capture.event_preference(), Some("OneCore"));
}

#[test]
fn capture_counts_stops_previews_and_configure_calls() {
    let mut capture = CaptureSpeech::new();
    capture.stop_main();
    capture.stop_event();
    capture.stop_event();
    capture.stop();
    assert_eq!(
        (
            capture.stop_main_calls(),
            capture.stop_event_calls(),
            capture.stop_calls()
        ),
        (1, 2, 1)
    );
    assert!(capture.say_adjustment_preview("speech_rate", "Speech rate: 60 percent.", true));
    assert!(!capture.say_adjustment_preview("speech_verbosity", "Speech verbosity: normal.", true));
    assert!(!capture.say_adjustment_preview("speech_rate", "", true));
    assert_eq!(
        capture.previews(),
        &[(
            "speech_rate".to_string(),
            "Speech rate: 60 percent.".to_string(),
            true
        )]
    );
    capture.configure(Some(0.8), None, Some(1.0), Some("Zira"));
    assert_eq!(
        capture.configure_calls(),
        &[(Some(0.8), None, Some(1.0), Some("Zira".to_string()))]
    );
    assert!(capture.lines().is_empty()); // previews and config are not speech
}

#[test]
fn capture_apply_speech_settings_keeps_the_saved_voice_name() {
    // The headless capture binds nothing, so the "record the voice actually
    // used" rule never rewrites the setting.
    let mut capture = CaptureSpeech::new();
    let mut settings = SpeechSettings {
        event_backend: "OneCore".to_string(),
        ..Default::default()
    };
    apply_speech_settings(&mut capture, &mut settings);
    assert_eq!(settings.event_backend, "OneCore");
    assert_eq!(capture.configure_calls().len(), 1);
    // And the full-voice capture offers OneCore as an option, so the choice
    // stands even though the capture still reports SAPI as bound.
    let mut full = CaptureSpeech::full_voice();
    apply_speech_settings(&mut full, &mut settings);
    assert_eq!(settings.event_backend, "OneCore");
}

#[test]
fn apply_speech_settings_keeps_a_chosen_voice_the_sink_has_not_bound_yet() {
    // A threaded sink queues a backend switch and answers
    // event_backend_name() from a snapshot that still names the voice bound
    // before the switch, so reading it back must never overwrite a fresh
    // choice: the full-voice capture is exactly that lagging sink -- OneCore
    // is on offer, SAPI is what it reports.
    let mut lagging = CaptureSpeech::full_voice();
    let mut settings = SpeechSettings {
        separate_event_voice: true,
        event_backend: "OneCore".to_string(),
        ..Default::default()
    };
    apply_speech_settings(&mut lagging, &mut settings);
    assert_eq!(lagging.event_backend_name(), "SAPI"); // the stale answer
    assert_eq!(settings.event_backend, "OneCore"); // the choice survives it

    // A voice that is not on this machine at all still resolves to what the
    // sink will bind in its place, so the menu and later sessions read true.
    settings.event_backend = "AVSpeech".to_string();
    apply_speech_settings(&mut lagging, &mut settings);
    assert_eq!(settings.event_backend, "OneCore");
}

#[test]
fn null_speech_swallows_everything() {
    let mut null = NullSpeech;
    null.say("hello", true);
    null.say_event("hello", false);
    null.stop();
    null.poll(REFRESH_INTERVAL_S);
    assert!(!null.available());
    assert_eq!(null.backend_name(), "none");
    assert!(!null.has_separate_event_voice());
    assert!(!null.say_adjustment_preview("speech_rate", "x", true));
    assert!(!null.refresh(true));
    null.shutdown();
}

// -- SpeechEnv / SpeechSettings / SharedCapture --------------------------------

#[test]
fn env_names_follow_the_game_prefix() {
    let env = hearken_speech::SpeechEnv::with_prefix("HELLO_GAME");
    assert_eq!(env.no_speech, "HELLO_GAME_NO_SPEECH");
    assert_eq!(env.backend, "HELLO_GAME_SPEECH_BACKEND");
    assert_eq!(env.stream_transcript, "HELLO_GAME_STREAM_TRANSCRIPT");
}

#[test]
fn a_disabled_live_sink_starts_and_takes_every_call_quietly() {
    let env = hearken_speech::SpeechEnv::with_prefix("HEARKEN_TEST_DISABLED");
    std::env::set_var(&env.no_speech, "1");
    let mut speech = hearken_speech::Speech::new(&env);
    assert!(!speech.available());
    speech.say("hello", true);
    speech.say_event("event", false);
    speech.poll(0.016);
    speech.stop();
    speech.shutdown();
    speech.shutdown();
    std::env::remove_var(&env.no_speech);
}

#[test]
fn speech_settings_default_and_partial_json() {
    let s: hearken_speech::SpeechSettings = serde_json::from_str(r#"{"rate": 0.7}"#).unwrap();
    assert_eq!(s.rate, 0.7);
    assert_eq!(s.pitch, 0.5);
    assert!(s.separate_event_voice);
    assert_eq!(s.event_backend, "SAPI");
}

#[test]
fn a_shared_capture_is_read_through_its_handle() {
    use hearken_speech::{CaptureSpeech, SharedCapture, SpeechSink};
    let handle = std::rc::Rc::new(std::cell::RefCell::new(CaptureSpeech::new()));
    let mut sink: Box<dyn SpeechSink> = Box::new(SharedCapture(handle.clone()));
    sink.say("one", true);
    sink.say_event("two", false);
    assert_eq!(handle.borrow().main_lines(), vec!["one".to_string()]);
    assert_eq!(handle.borrow().event_lines(), vec!["two".to_string()]);
}
