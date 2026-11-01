//! The built-in voice in the registry: chosen when nothing else can speak,
//! or when Settings asks for it, and never over a working screen reader.
#![cfg(feature = "genny-voice")]

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use hearken_speech::builtin::{BuiltinEngine, WithBuiltin};
use hearken_speech::fakes::{FakeRegistry, FakeVoice};
use hearken_speech::{
    apply_speech_settings, Speech, SpeechSettings, SpeechSink, VoiceFeatures, BUILTIN_BACKEND,
};

/// A renderer with no sound device, so the tests run anywhere.
fn offline() -> Arc<BuiltinEngine> {
    BuiltinEngine::offline()
}

fn registry(voices: Vec<FakeVoice>) -> WithBuiltin {
    WithBuiltin::with_engine(Some(Box::new(FakeRegistry::new(voices))), offline)
}

#[test]
fn nothing_else_can_speak_so_the_built_in_voice_does() {
    let nvda = FakeVoice::new("NVDA", 103, VoiceFeatures::SPEAKING);
    nvda.set_runtime_supported(false);
    let s = Speech::with_registry(Box::new(registry(vec![nvda])), None);
    assert_eq!(s.backend_name(), BUILTIN_BACKEND);
    assert!(s.supports_rate());
    assert!(s.supports_volume());
}

#[test]
fn prism_missing_entirely_still_speaks() {
    let reg = WithBuiltin::with_engine(None, offline);
    let mut s = Speech::with_registry(Box::new(reg), None);
    assert_eq!(s.backend_name(), BUILTIN_BACKEND);
    s.say("Play, 1 of 6.", true);
}

#[test]
fn a_running_screen_reader_wins_until_settings_choose_the_built_in_voice() {
    let nvda = FakeVoice::new("NVDA", 103, VoiceFeatures::SPEAKING);
    let mut s = Speech::with_registry(Box::new(registry(vec![nvda.clone()])), None);
    assert_eq!(s.backend_name(), "NVDA");

    let mut settings = SpeechSettings {
        self_voicing: true,
        builtin_voice: "British male".to_string(),
        ..SpeechSettings::default()
    };
    apply_speech_settings(&mut s, &mut settings);
    assert_eq!(s.backend_name(), BUILTIN_BACKEND);
    // A health check keeps the choice.
    s.refresh();
    assert_eq!(s.backend_name(), BUILTIN_BACKEND);
    s.say("Settings.", true);
    assert!(nvda.spoken().is_empty());

    settings.self_voicing = false;
    apply_speech_settings(&mut s, &mut settings);
    assert_eq!(s.backend_name(), "NVDA");
}

#[test]
fn every_voice_is_offered_and_renders() {
    static ENGINE: OnceLock<Arc<BuiltinEngine>> = OnceLock::new();
    fn shared() -> Arc<BuiltinEngine> {
        Arc::clone(ENGINE.get_or_init(BuiltinEngine::offline))
    }
    let reg = WithBuiltin::with_engine(None, shared);
    let mut s = Speech::with_registry(Box::new(reg), None);
    let names: Vec<String> = hearken_speech::builtin_voices()
        .iter()
        .map(|v| v.to_string())
        .collect();
    assert_eq!(s.voice_names(), names);
    for name in &names {
        s.configure(None, None, None, Some(name));
        s.say("Rooms cleared: 4.", false);
    }
    let engine = shared();
    assert!(engine.wait_rendered(Duration::from_secs(20)));
    assert!(engine.queued() > 6 * 16_000, "{} samples", engine.queued());
    // An interrupting line drops everything queued before it.
    s.say("Go.", true);
    assert!(engine.wait_rendered(Duration::from_secs(5)));
    assert!(engine.queued() < 16_000, "{} samples", engine.queued());
    s.stop();
    assert_eq!(engine.queued(), 0);
    assert!(!s.is_speaking());
}
