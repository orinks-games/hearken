use hearken_audio::{ClipSource, LooseClips};

#[test]
fn a_key_finds_its_file_by_folder_and_any_known_extension() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("ui")).unwrap();
    std::fs::write(dir.path().join("ui/menu_move.wav"), b"RIFF-ish").unwrap();
    std::fs::write(dir.path().join("door.ogg"), b"OggS").unwrap();
    std::fs::write(dir.path().join("theme.opus"), b"OggS-opus").unwrap();
    let mut clips = LooseClips::at(dir.path());
    assert_eq!(
        clips.load("ui/menu_move").as_deref(),
        Some(&b"RIFF-ish"[..])
    );
    assert_eq!(clips.load("door").as_deref(), Some(&b"OggS"[..]));
    assert_eq!(clips.load("theme").as_deref(), Some(&b"OggS-opus"[..]));
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

#[test]
fn a_layered_source_takes_the_first_layer_that_has_the_key() {
    use hearken_audio::{LayeredClips, MemoryClips};
    let loose = MemoryClips::default().with("horn", b"recording".to_vec());
    let builtin = MemoryClips::default()
        .with("horn", b"placeholder".to_vec())
        .with("bell", b"placeholder bell".to_vec());
    let mut clips = LayeredClips::new(vec![Box::new(loose), Box::new(builtin)]);
    assert_eq!(&*clips.load("horn").unwrap(), b"recording");
    assert_eq!(&*clips.load("bell").unwrap(), b"placeholder bell");
    assert!(clips.load("whistle").is_none());
}

#[test]
fn made_clips_are_made_once_and_cached_misses_included() {
    use hearken_audio::MadeClips;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let mut clips = MadeClips::new(move |key| {
        counter.fetch_add(1, Ordering::SeqCst);
        (key == "tone").then(|| vec![1, 2, 3])
    });
    assert_eq!(&*clips.load("tone").unwrap(), &[1, 2, 3]);
    assert_eq!(&*clips.load("tone").unwrap(), &[1, 2, 3]);
    assert!(clips.load("none").is_none());
    assert!(clips.load("none").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_sound_pack_gives_the_same_sounds_as_the_folder_it_was_made_from() {
    use hearken_audio::PackedClips;
    let dir = tempfile::tempdir().unwrap();
    let sounds = dir.path().join("sounds");
    std::fs::create_dir_all(sounds.join("ui")).unwrap();
    std::fs::write(sounds.join("ui/menu_move.wav"), b"RIFF-ish").unwrap();
    std::fs::write(sounds.join("door.ogg"), b"OggS").unwrap();
    let pak = dir.path().join(hearken_pack::FILE_NAME);
    let key = hearken_pack::new_key().unwrap();
    hearken_pack::write(&sounds, &pak, &key).unwrap();

    let mut loose = LooseClips::at(&sounds);
    let mut packed = PackedClips::open(&pak, &key).unwrap();
    for name in ["ui/menu_move", "door", "ui/nothing"] {
        assert_eq!(packed.load(name), loose.load(name), "{name}");
    }
    // A Windows-style key finds the sound on every platform.
    assert_eq!(
        packed.load("ui\\menu_move").as_deref(),
        Some(&b"RIFF-ish"[..])
    );
    assert!(PackedClips::open(&pak, &hearken_pack::new_key().unwrap()).is_err());
}

/// A game folder with a loose `sounds` folder, an override folder named by
/// `<prefix>_SOUNDS`, and a `sounds.pak` made from other sounds.
fn packed_game(prefix: &str) -> (tempfile::TempDir, [u8; 32]) {
    let dir = tempfile::tempdir().unwrap();
    let put = |rel: &str, bytes: &[u8]| {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    put("sounds/door.ogg", b"loose door");
    put("override/door.ogg", b"override door");
    put("source/door.ogg", b"packed door");
    let key = hearken_pack::new_key().unwrap();
    hearken_pack::write(
        &dir.path().join("source"),
        &dir.path().join(hearken_pack::FILE_NAME),
        &key,
    )
    .unwrap();
    std::env::set_var(format!("{prefix}_SOUNDS"), dir.path().join("override"));
    (dir, key)
}

#[test]
fn a_packed_build_reads_its_pack_and_nothing_else() {
    let prefix = "HEARKEN_TEST_PACKED_ONLY";
    let (dir, key) = packed_game(prefix);
    let mut found = hearken_audio::discover_clips_from(prefix, Some(key), Some(dir.path()));
    assert!(found.notice.is_none());
    assert_eq!(
        found.clips.load("door").as_deref(),
        Some(&b"packed door"[..])
    );
    std::env::remove_var(format!("{prefix}_SOUNDS"));
}

#[test]
fn a_packed_mac_app_reads_the_pack_in_its_resources() {
    let prefix = "HEARKEN_TEST_PACKED_MAC_APP";
    let (dir, key) = packed_game(prefix);
    let contents = dir.path().join("Game.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    std::fs::create_dir_all(contents.join("Resources")).unwrap();
    std::fs::rename(
        dir.path().join(hearken_pack::FILE_NAME),
        contents.join("Resources").join(hearken_pack::FILE_NAME),
    )
    .unwrap();
    let mut found =
        hearken_audio::discover_clips_from(prefix, Some(key), Some(&contents.join("MacOS")));
    assert!(found.notice.is_none());
    assert_eq!(
        found.clips.load("door").as_deref(),
        Some(&b"packed door"[..])
    );
    std::env::remove_var(format!("{prefix}_SOUNDS"));
}

#[test]
fn a_packed_build_with_a_missing_or_damaged_pack_plays_nothing_loose() {
    let prefix = "HEARKEN_TEST_PACK_DAMAGED";
    let (dir, key) = packed_game(prefix);
    let pak = dir.path().join(hearken_pack::FILE_NAME);
    let mut bytes = std::fs::read(&pak).unwrap();
    bytes[14] ^= 1;
    std::fs::write(&pak, &bytes).unwrap();
    let mut found = hearken_audio::discover_clips_from(prefix, Some(key), Some(dir.path()));
    assert_eq!(
        found.notice.as_deref(),
        Some(hearken_audio::DAMAGED_PACK_NOTICE)
    );
    assert_eq!(found.clips.load("door"), None);

    std::fs::remove_file(&pak).unwrap();
    let mut found = hearken_audio::discover_clips_from(prefix, Some(key), Some(dir.path()));
    assert!(found.notice.is_some());
    assert_eq!(found.clips.load("door"), None);
    std::env::remove_var(format!("{prefix}_SOUNDS"));
}

#[test]
fn a_changed_sound_in_the_pack_plays_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("s")).unwrap();
    std::fs::write(dir.path().join("s/door.ogg"), b"OggS door").unwrap();
    let pak = dir.path().join(hearken_pack::FILE_NAME);
    let key = hearken_pack::new_key().unwrap();
    hearken_pack::write(&dir.path().join("s"), &pak, &key).unwrap();
    let mut bytes = std::fs::read(&pak).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&pak, &bytes).unwrap();
    let mut packed = hearken_audio::PackedClips::open(&pak, &key).unwrap();
    assert_eq!(packed.load("door"), None);
    assert_eq!(packed.load("door"), None);
}

#[test]
fn a_build_without_a_pack_key_reads_loose_sounds_as_before() {
    let prefix = "HEARKEN_TEST_UNPACKED";
    let (dir, _key) = packed_game(prefix);
    let mut found = hearken_audio::discover_clips_from(prefix, None, Some(dir.path()));
    assert!(found.notice.is_none());
    assert_eq!(
        found.clips.load("door").as_deref(),
        Some(&b"override door"[..])
    );
    std::env::remove_var(format!("{prefix}_SOUNDS"));
    let mut found = hearken_audio::discover_clips_from(prefix, None, Some(dir.path()));
    assert_eq!(
        found.clips.load("door").as_deref(),
        Some(&b"loose door"[..])
    );
}
