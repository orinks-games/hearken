use hearken_synth::*;

const EARCONS: [&str; 7] = [
    UI_OPEN, UI_MOVE, UI_SELECT, UI_BACK, UI_TICK, UI_ERROR, UI_NOTIFY,
];

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn a_wav_has_a_riff_header_and_clamps_its_samples() {
    let bytes = wav_bytes(&[0.0, 2.0, -2.0], 44_100);
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(bytes.len(), 44 + 3 * 2);
    assert_eq!(
        u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
        44_100
    );
    assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 32767);
    assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), -32767);
}

#[test]
fn keys_sort_into_kinds_by_their_first_folder() {
    assert_eq!(kind("ui/anything"), SoundKind::Ui);
    assert_eq!(kind("sfx/door"), SoundKind::Sfx);
    assert_eq!(kind("amb/harbour"), SoundKind::Ambience);
    assert_eq!(kind("music/title"), SoundKind::Music);
    assert_eq!(kind("voice/hello"), SoundKind::Other);
    assert_eq!(kind("beep"), SoundKind::Other);
}

#[test]
fn every_earcon_is_short_audible_unclipped_and_its_own() {
    let rendered: Vec<Vec<f32>> = EARCONS.iter().map(|k| builtin(k)).collect();
    for (key, v) in EARCONS.iter().zip(&rendered) {
        assert!(!v.is_empty(), "{key} is empty");
        assert!(
            v.len() < SAMPLE_RATE as usize / 2,
            "{key} is longer than half a second"
        );
        let p = peak(v);
        assert!(p > 0.1 && p <= 1.0, "{key} peaks at {p}");
    }
    for i in 0..rendered.len() {
        for j in i + 1..rendered.len() {
            assert_ne!(
                rendered[i], rendered[j],
                "{} and {} sound the same",
                EARCONS[i], EARCONS[j]
            );
        }
    }
    assert_ne!(
        builtin("ui/something_else"),
        rendered[1],
        "the ui default is not the move earcon"
    );
}

#[test]
fn a_kind_without_a_sound_of_its_own_gets_one_default_rendered_the_same_every_time() {
    assert_eq!(builtin("sfx/door"), builtin("sfx/window"));
    assert_eq!(builtin("amb/a"), builtin("amb/a"));
    assert!(peak(&builtin("sfx/door")) > 0.1);
}

#[test]
fn ambience_and_music_defaults_loop_without_a_click() {
    for (key, seconds) in [("amb/x", 2.0), ("music/x", 4.0)] {
        let v = builtin(key);
        assert_eq!(
            v.len(),
            (seconds * SAMPLE_RATE as f32) as usize,
            "{key} length"
        );
        let seam = (v[0] - v[v.len() - 1]).abs();
        assert!(seam < 0.05, "{key} jumps by {seam} at the loop point");
    }
}

#[test]
fn defaults_are_cached_per_kind_not_per_key() {
    assert!(std::sync::Arc::ptr_eq(
        &builtin_wav("sfx/a"),
        &builtin_wav("sfx/b")
    ));
    assert!(!std::sync::Arc::ptr_eq(
        &builtin_wav(UI_MOVE),
        &builtin_wav("ui/other")
    ));
}
