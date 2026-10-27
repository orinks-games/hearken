use super::*;

const NAME: &str = "records.json";

fn sealing() -> (tempfile::TempDir, Vault, Key) {
    let dir = tempfile::tempdir().unwrap();
    let key = Key::generate().unwrap();
    let vault = Vault::with_key(dir.path(), Some(key.clone()));
    (dir, vault, key)
}

/// A new vault on the same folder, as the next run of the game sees it.
fn reopen(dir: &tempfile::TempDir, key: &Key) -> Vault {
    Vault::with_key(dir.path(), Some(key.clone()))
}

fn tampered(opened: Opened) -> Tamper {
    match opened {
        Opened::Tampered(how) => how,
        other => panic!("expected Tampered, got {other:?}"),
    }
}

fn sealed(opened: Opened) -> Vec<u8> {
    match opened {
        Opened::Sealed(bytes) => bytes,
        other => panic!("expected Sealed, got {other:?}"),
    }
}

#[test]
fn a_sealed_file_round_trips_and_cannot_be_read() {
    let (dir, mut vault, key) = sealing();
    assert!(vault.is_sealing());
    assert!(matches!(vault.read(NAME), Opened::Missing));
    vault.write(NAME, b"{\"best\":12}").unwrap();
    assert_eq!(sealed(vault.read(NAME)), b"{\"best\":12}");
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"{\"best\":12}");
    let raw = std::fs::read(dir.path().join(NAME)).unwrap();
    assert!(raw.starts_with(b"HKSL"));
    assert!(!raw.windows(4).any(|w| w == b"best"));
    assert!(dir.path().join(LEDGER).is_file());
    assert!(!dir.path().join(name::temp(NAME)).exists());
}

#[test]
fn a_build_without_a_key_keeps_plain_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = Vault::with_key(dir.path(), None);
    assert!(!vault.is_sealing());
    assert!(matches!(vault.read(NAME), Opened::Missing));
    vault.write(NAME, b"{\"best\":3}").unwrap();
    assert_eq!(
        std::fs::read(dir.path().join(NAME)).unwrap(),
        b"{\"best\":3}"
    );
    assert_eq!(vault.read(NAME).trusted().unwrap(), b"{\"best\":3}");
    assert!(!dir.path().join(LEDGER).exists());
    vault.remove(NAME).unwrap();
    assert!(matches!(vault.read(NAME), Opened::Missing));
}

#[test]
fn every_changed_byte_is_forged() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"{\"best\":12}").unwrap();
    let path = dir.path().join(NAME);
    let good = std::fs::read(&path).unwrap();
    for at in 0..good.len() {
        let mut bad = good.clone();
        bad[at] ^= 0x20;
        std::fs::write(&path, &bad).unwrap();
        let how = tampered(reopen(&dir, &key).read(NAME));
        // A changed magic makes it a plain file over a sealed one.
        let expected = if at < 4 {
            Tamper::Unsealed
        } else {
            Tamper::Forged
        };
        assert_eq!(how, expected, "byte {at}");
    }
    std::fs::write(&path, &good[..good.len() - 3]).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Forged);
    let mut longer = good.clone();
    longer.extend_from_slice(b"99");
    std::fs::write(&path, &longer).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Forged);
    std::fs::write(&path, &good).unwrap();
    assert!(reopen(&dir, &key).read(NAME).trusted().is_some());
}

#[test]
fn a_file_copied_over_another_is_forged() {
    let (dir, mut vault, key) = sealing();
    vault.write("a.json", b"a's").unwrap();
    vault.write("b.json", b"b's").unwrap();
    std::fs::copy(dir.path().join("a.json"), dir.path().join("b.json")).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read("b.json")), Tamper::Forged);
    assert_eq!(sealed(reopen(&dir, &key).read("a.json")), b"a's");
}

#[test]
fn another_keys_file_is_forged() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"mine").unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut theirs = Vault::with_key(other.path(), Some(Key::generate().unwrap()));
    theirs.write(NAME, b"theirs").unwrap();
    std::fs::copy(other.path().join(NAME), dir.path().join(NAME)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Forged);
}

#[test]
fn someone_elses_whole_file_and_ledger_are_forged() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"mine").unwrap();
    let other = tempfile::tempdir().unwrap();
    let mut theirs = Vault::with_key(other.path(), Some(Key::generate().unwrap()));
    theirs.write(NAME, b"theirs").unwrap();
    std::fs::copy(other.path().join(LEDGER), dir.path().join(LEDGER)).unwrap();
    assert_eq!(
        tampered(reopen(&dir, &key).read(NAME)),
        Tamper::LedgerForged
    );
}

#[test]
fn an_older_copy_is_rolled_back() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"first").unwrap();
    let old = std::fs::read(dir.path().join(NAME)).unwrap();
    vault.write(NAME, b"second").unwrap();
    vault.write(NAME, b"third").unwrap();
    std::fs::write(dir.path().join(NAME), &old).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::RolledBack);
}

#[test]
fn a_deleted_file_is_removed() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"x").unwrap();
    std::fs::remove_file(dir.path().join(NAME)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Removed);
}

#[test]
fn a_plain_file_over_a_sealed_one_is_unsealed() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"x").unwrap();
    std::fs::write(dir.path().join(NAME), b"{\"best\":9999}").unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unsealed);
    // So is an empty one.
    std::fs::write(dir.path().join(NAME), b"").unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unsealed);
}

#[test]
fn deleting_the_ledger_leaves_later_files_unexpected() {
    let (dir, mut vault, key) = sealing();
    for n in 0..3 {
        vault.write(NAME, format!("{n}").as_bytes()).unwrap();
    }
    std::fs::remove_file(dir.path().join(LEDGER)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unexpected);
}

#[test]
fn a_file_from_another_folder_is_forged() {
    let (dir, mut vault, key) = sealing();
    vault.write("other.json", b"x").unwrap();
    // The same game on another computer, played longer.
    let elsewhere = tempfile::tempdir().unwrap();
    let mut theirs = Vault::with_key(elsewhere.path(), Some(key.clone()));
    for n in 0..5 {
        theirs.write(NAME, format!("{n}").as_bytes()).unwrap();
    }
    std::fs::copy(elsewhere.path().join(NAME), dir.path().join(NAME)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Forged);
}

#[test]
fn a_file_from_another_folder_is_forged_even_one_generation_ahead() {
    // The attack: this folder's ledger is at 3, another install's file is
    // at 4, which the crash repair would otherwise take as this folder's.
    let (dir, mut vault, key) = sealing();
    for n in 0..3 {
        vault.write(NAME, format!("mine {n}").as_bytes()).unwrap();
    }
    let elsewhere = tempfile::tempdir().unwrap();
    let mut theirs = Vault::with_key(elsewhere.path(), Some(key.clone()));
    for n in 0..4 {
        theirs
            .write(NAME, format!("theirs {n}").as_bytes())
            .unwrap();
    }
    std::fs::copy(elsewhere.path().join(NAME), dir.path().join(NAME)).unwrap();
    let mut vault = reopen(&dir, &key);
    assert_eq!(tampered(vault.read(NAME)), Tamper::Forged);
    // And the ledger did not move on: the attempt changes nothing.
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Forged);
}

#[test]
fn another_folders_ledger_does_not_vouch_for_this_folders_files() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"mine").unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let mut theirs = Vault::with_key(elsewhere.path(), Some(key.clone()));
    theirs.write(NAME, b"theirs").unwrap();
    std::fs::copy(elsewhere.path().join(LEDGER), dir.path().join(LEDGER)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Forged);
}

#[test]
fn an_edited_or_garbled_ledger_is_forged() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"x").unwrap();
    let ledger = dir.path().join(LEDGER);
    let mut bytes = std::fs::read(&ledger).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&ledger, &bytes).unwrap();
    assert_eq!(
        tampered(reopen(&dir, &key).read(NAME)),
        Tamper::LedgerForged
    );
    std::fs::write(&ledger, b"records.json\t1\n").unwrap();
    assert_eq!(
        tampered(reopen(&dir, &key).read(NAME)),
        Tamper::LedgerForged
    );
    std::fs::write(&ledger, b"").unwrap();
    assert_eq!(
        tampered(reopen(&dir, &key).read(NAME)),
        Tamper::LedgerForged
    );
    std::fs::remove_file(&ledger).unwrap();
    std::fs::create_dir(&ledger).unwrap();
    assert_eq!(
        tampered(reopen(&dir, &key).read(NAME)),
        Tamper::LedgerForged
    );
}

#[test]
fn a_forged_ledger_is_set_aside_with_the_file_and_the_folder_starts_over() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"x").unwrap();
    std::fs::write(dir.path().join(LEDGER), b"garbage").unwrap();
    let mut vault = reopen(&dir, &key);
    assert_eq!(tampered(vault.read(NAME)), Tamper::LedgerForged);
    assert_eq!(
        vault.write(NAME, b"y").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let to = vault.set_aside(NAME).unwrap().unwrap();
    assert!(to.ends_with("records.json.tampered"));
    assert!(dir.path().join("hearken-seal.ledger.tampered").is_file());
    assert!(matches!(vault.read(NAME), Opened::Missing));
    vault.write(NAME, b"fresh").unwrap();
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"fresh");
}

#[test]
fn a_crash_between_the_file_and_the_ledger_is_repaired() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"one").unwrap();
    let ledger_at_one = std::fs::read(dir.path().join(LEDGER)).unwrap();
    vault.write(NAME, b"two").unwrap();
    // As if the game stopped after writing the file, before the ledger.
    std::fs::write(dir.path().join(LEDGER), &ledger_at_one).unwrap();
    let mut vault = reopen(&dir, &key);
    assert_eq!(sealed(vault.read(NAME)), b"two");
    // The ledger has caught up: the copy at one is now a rollback.
    let mut vault2 = reopen(&dir, &key);
    assert_eq!(sealed(vault2.read(NAME)), b"two");
    vault.write(NAME, b"three").unwrap();
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"three");
}

/// The folder's ledger as it would be with `name`'s entry left out: what
/// a crash between a first write and its ledger update leaves.
fn ledger_without(dir: &tempfile::TempDir, key: &Key, name: &str) -> Vec<u8> {
    let path = dir.path().join(LEDGER);
    let (generation, text) = format::open(
        &key.file_key(),
        &NO_FOLDER,
        LEDGER,
        &std::fs::read(path).unwrap(),
    )
    .unwrap();
    let (folder, mut entries) = ledger::from_text(&text).unwrap();
    entries.remove(name);
    let text = ledger::to_text(&folder, &entries);
    format::seal(
        &key.file_key(),
        &NO_FOLDER,
        LEDGER,
        generation - 1,
        text.as_bytes(),
    )
    .unwrap()
}

#[test]
fn a_crash_after_the_first_write_is_repaired() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"one").unwrap();
    let before = ledger_without(&dir, &key, NAME);
    std::fs::write(dir.path().join(LEDGER), before).unwrap();
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"one");
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"one");
}

#[test]
fn the_ledger_is_kept_before_the_first_file_is_sealed() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"one").unwrap();
    // No ledger at all with a sealed file can only be a deleted ledger.
    std::fs::remove_file(dir.path().join(LEDGER)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unexpected);
}

#[test]
fn a_leftover_temporary_file_changes_nothing() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"good").unwrap();
    std::fs::write(dir.path().join(name::temp(NAME)), b"half a file").unwrap();
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"good");
    vault.write(NAME, b"better").unwrap();
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"better");
}

#[test]
fn a_legacy_plain_file_can_be_imported_once() {
    let (dir, mut vault, key) = sealing();
    std::fs::write(dir.path().join(NAME), b"{\"best\":7}").unwrap();
    match vault.read(NAME) {
        Opened::Legacy(bytes) => assert_eq!(bytes, b"{\"best\":7}"),
        other => panic!("expected Legacy, got {other:?}"),
    }
    vault.write(NAME, b"{\"best\":7}").unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("records.json.plain-backup")).unwrap(),
        b"{\"best\":7}"
    );
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"{\"best\":7}");
    std::fs::write(dir.path().join(NAME), b"{\"best\":9999}").unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unsealed);
}

#[test]
fn a_build_without_the_key_cannot_read_or_overwrite_sealed_files() {
    let (dir, mut vault, _key) = sealing();
    vault.write(NAME, b"release save").unwrap();
    let before = std::fs::read(dir.path().join(NAME)).unwrap();
    let mut dev = Vault::with_key(dir.path(), None);
    assert!(matches!(dev.read(NAME), Opened::Locked));
    assert_eq!(
        dev.write(NAME, b"dev").unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        dev.remove(NAME).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        dev.set_aside(NAME).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(std::fs::read(dir.path().join(NAME)).unwrap(), before);
    // A build with another key cannot read them either.
    let mut other = Vault::with_key(dir.path(), Some(Key::generate().unwrap()));
    assert_eq!(tampered(other.read(NAME)), Tamper::LedgerForged);
}

#[test]
fn remove_and_set_aside_forget_the_history() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"x").unwrap();
    vault.write(NAME, b"y").unwrap();
    vault.remove(NAME).unwrap();
    assert!(!dir.path().join(NAME).exists());
    assert!(matches!(reopen(&dir, &key).read(NAME), Opened::Missing));

    vault.write(NAME, b"z").unwrap();
    std::fs::write(dir.path().join(NAME), b"edited").unwrap();
    let mut vault = reopen(&dir, &key);
    assert_eq!(tampered(vault.read(NAME)), Tamper::Unsealed);
    let first = vault.set_aside(NAME).unwrap().unwrap();
    assert_eq!(std::fs::read(&first).unwrap(), b"edited");
    assert!(matches!(vault.read(NAME), Opened::Missing));
    assert!(matches!(reopen(&dir, &key).read(NAME), Opened::Missing));
    // A second one goes beside the first.
    std::fs::write(dir.path().join(NAME), b"again").unwrap();
    let second = vault.set_aside(NAME).unwrap().unwrap();
    assert!(second.ends_with("records.json.tampered-2"));
    // Nothing to set aside: the history is still forgotten.
    vault.write(NAME, b"w").unwrap();
    std::fs::remove_file(dir.path().join(NAME)).unwrap();
    assert_eq!(tampered(vault.read(NAME)), Tamper::Removed);
    assert_eq!(vault.set_aside(NAME).unwrap(), None);
    assert!(matches!(vault.read(NAME), Opened::Missing));
}

#[test]
fn bad_names_are_refused() {
    let (_dir, mut vault, _key) = sealing();
    for bad in ["../x", "a/b", ".hidden", LEDGER, &"x".repeat(65)] {
        assert!(matches!(
            vault.read(bad),
            Opened::Failed(e) if e.kind() == io::ErrorKind::InvalidInput
        ));
        assert_eq!(
            vault.write(bad, b"x").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            vault.remove(bad).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            vault.set_aside(bad).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}

#[test]
fn a_folder_in_place_of_the_file_fails_and_is_never_trusted() {
    let (dir, mut vault, _key) = sealing();
    std::fs::create_dir(dir.path().join(NAME)).unwrap();
    assert!(matches!(vault.read(NAME), Opened::Failed(_)));
    assert!(vault.write(NAME, b"x").is_err());
}

#[cfg(unix)]
#[test]
fn a_link_to_another_file_is_judged_by_what_it_points_at() {
    let (dir, mut vault, key) = sealing();
    vault.write("a.json", b"a").unwrap();
    vault.write(NAME, b"mine").unwrap();
    std::fs::remove_file(dir.path().join(NAME)).unwrap();
    std::os::unix::fs::symlink(dir.path().join("a.json"), dir.path().join(NAME)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Forged);
    // A dangling link is a removed file.
    std::fs::remove_file(dir.path().join(NAME)).unwrap();
    std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join(NAME)).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Removed);
    // Writing replaces the link, not what it points at.
    let mut vault = reopen(&dir, &key);
    vault.set_aside(NAME).unwrap();
    vault.write(NAME, b"fresh").unwrap();
    assert!(!std::fs::symlink_metadata(dir.path().join(NAME))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn only_sealed_and_plain_are_trusted() {
    assert_eq!(Opened::Sealed(vec![1]).trusted(), Some(vec![1]));
    assert_eq!(Opened::Plain(vec![2]).trusted(), Some(vec![2]));
    assert_eq!(Opened::Legacy(vec![3]).trusted(), None);
    assert_eq!(Opened::Locked.trusted(), None);
    assert_eq!(Opened::Missing.trusted(), None);
    assert_eq!(Opened::Tampered(Tamper::Forged).trusted(), None);
}

#[test]
fn a_plain_file_after_a_set_aside_is_unsealed_not_legacy() {
    // The attack: get a file set aside as tampered, then drop in a plain
    // one, hoping the forgotten history lets it import.
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"{\"best\":20}").unwrap();
    std::fs::write(dir.path().join(NAME), b"garbled").unwrap();
    let mut vault = reopen(&dir, &key);
    assert_eq!(tampered(vault.read(NAME)), Tamper::Unsealed);
    vault.set_aside(NAME).unwrap();
    assert!(matches!(reopen(&dir, &key).read(NAME), Opened::Missing));
    std::fs::write(dir.path().join(NAME), b"{\"best\":999999}").unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unsealed);
}

#[test]
fn a_plain_file_after_a_remove_is_unsealed_and_an_old_copy_rolled_back() {
    let (dir, mut vault, key) = sealing();
    vault.write(NAME, b"one").unwrap();
    let old = std::fs::read(dir.path().join(NAME)).unwrap();
    vault.remove(NAME).unwrap();
    assert!(matches!(reopen(&dir, &key).read(NAME), Opened::Missing));
    std::fs::write(dir.path().join(NAME), b"plain").unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unsealed);
    std::fs::write(dir.path().join(NAME), &old).unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::RolledBack);
    // The next write starts past the old history, and reads back.
    let mut vault = reopen(&dir, &key);
    vault.set_aside(NAME).unwrap();
    vault.write(NAME, b"two").unwrap();
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"two");
}

#[test]
fn a_folder_that_has_sealed_never_reads_a_plain_file_as_legacy() {
    let (dir, mut vault, key) = sealing();
    vault.write("other.json", b"x").unwrap();
    std::fs::write(dir.path().join(NAME), b"plain").unwrap();
    assert_eq!(tampered(reopen(&dir, &key).read(NAME)), Tamper::Unsealed);
}

/// A sealing vault on a folder of its own, with one witness in `elsewhere`.
fn witnessed(key: &Key, elsewhere: &tempfile::TempDir) -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::with_key(dir.path(), Some(key.clone()))
        .with_witnesses(vec![elsewhere.path().join("game").join("witness")]);
    (dir, vault)
}

#[test]
fn a_witness_stops_a_wiped_folder_from_importing_a_plain_file() {
    // The attack: delete the sealed file and the ledger, then drop in a
    // hand-written plain file, as if from before sealing.
    let key = Key::generate().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let (dir, mut vault) = witnessed(&key, &elsewhere);
    std::fs::write(dir.path().join(NAME), b"{\"best\":20}").unwrap();
    // The genuine first import still works: nothing has sealed yet.
    assert!(matches!(vault.read(NAME), Opened::Legacy(_)));
    vault.write(NAME, b"{\"best\":20}").unwrap();
    assert!(elsewhere.path().join("game").join("witness").is_file());
    std::fs::remove_file(dir.path().join(NAME)).unwrap();
    std::fs::remove_file(dir.path().join(LEDGER)).unwrap();
    std::fs::write(dir.path().join(NAME), b"{\"best\":999999}").unwrap();
    let mut again = Vault::with_key(dir.path(), Some(key.clone()))
        .with_witnesses(vec![elsewhere.path().join("game").join("witness")]);
    assert_eq!(tampered(again.read(NAME)), Tamper::Unsealed);
}

#[test]
fn a_witness_stops_a_fresh_folder_from_importing_a_plain_file() {
    // The attack: point the game at a new folder (portable.txt, XDG_DATA_HOME)
    // holding a hand-written plain file.
    let key = Key::generate().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let (_dir, mut vault) = witnessed(&key, &elsewhere);
    vault.write(NAME, b"{\"best\":20}").unwrap();
    let (fresh, mut other) = witnessed(&key, &elsewhere);
    std::fs::write(fresh.path().join(NAME), b"{\"best\":999999}").unwrap();
    assert_eq!(tampered(other.read(NAME)), Tamper::Unsealed);
    // An empty fresh folder is simply empty.
    let (_empty, mut empty) = witnessed(&key, &elsewhere);
    assert!(matches!(empty.read(NAME), Opened::Missing));
}

#[test]
fn a_witness_that_cannot_be_written_does_not_stop_saving() {
    let key = Key::generate().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("game"), b"a file, not a folder").unwrap();
    let (dir, mut vault) = witnessed(&key, &elsewhere);
    vault.write(NAME, b"x").unwrap();
    assert_eq!(sealed(reopen(&dir, &key).read(NAME)), b"x");
}
