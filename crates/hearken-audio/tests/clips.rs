use hearken_audio::{ClipSource, LooseClips};

#[test]
fn a_key_finds_its_file_by_folder_and_any_known_extension() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("ui")).unwrap();
    std::fs::write(dir.path().join("ui/menu_move.wav"), b"RIFF-ish").unwrap();
    std::fs::write(dir.path().join("door.ogg"), b"OggS").unwrap();
    let mut clips = LooseClips::at(dir.path());
    assert_eq!(
        clips.load("ui/menu_move").as_deref(),
        Some(&b"RIFF-ish"[..])
    );
    assert_eq!(clips.load("door").as_deref(), Some(&b"OggS"[..]));
    assert!(clips.load("ui/nothing").is_none());
}

#[test]
fn a_key_that_climbs_out_of_the_sounds_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.wav"), b"x").unwrap();
    let inner = dir.path().join("sounds");
    std::fs::create_dir_all(&inner).unwrap();
    let mut clips = LooseClips::at(&inner);
    for key in ["../secret", "/secret", "C:/secret", "a/../../secret"] {
        assert!(clips.load(key).is_none(), "{key}");
    }
}

#[test]
fn a_miss_is_remembered_so_the_disk_is_not_asked_every_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut clips = LooseClips::at(dir.path());
    assert!(clips.load("sfx/step").is_none());
    std::fs::create_dir_all(dir.path().join("sfx")).unwrap();
    std::fs::write(dir.path().join("sfx/step.wav"), b"x").unwrap();
    assert!(
        clips.load("sfx/step").is_none(),
        "cached as missing for the run"
    );
}
