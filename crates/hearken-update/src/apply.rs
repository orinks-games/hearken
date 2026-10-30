//! Swapping the update in: a small script, started detached, that waits
//! for the game to exit, replaces the install and starts the game again.
//!
//! A failure part-way must never leave the player without a game:
//!
//! - folder installs are copied to a backup first. Without a backup, nothing
//!   is touched. While the update is copied over, a journal in the game's
//!   folder ([`JOURNAL`]) names the backup; it is removed once the copy is
//!   done. If the copy fails, or the script is stopped part-way (a power
//!   cut, the computer shutting down), the journal is still there and the
//!   next start of the game puts the backup back exactly
//!   ([`recover_interrupted`]). The program is copied last, beside itself,
//!   and renamed into place, so until the very end the old program is whole
//!   and can start to do that. The `saves` folder is never copied either
//!   way;
//! - an app bundle is copied in beside the old one first, then the old one
//!   is moved aside (`.old`) and the new one renamed into its place: two
//!   renames, the only moment without an app;
//! - an AppImage is copied in beside the old one and renamed over it, which
//!   is one step that either happens or does not.
//!
//! The script waits up to five minutes for the game to exit, and gives up
//! without touching anything if it never does.
//!
//! The script is written inside the staging folder, which only this user
//! can open (see [`crate::install::make_staging_dir`]), under a random name,
//! as a new file: on Linux the temporary folder is `/tmp`, shared by every
//! user, and a script there under a name anyone could guess could be
//! swapped for theirs, or be a link that makes the game overwrite one of
//! the player's files. Deleting the staging folder deletes the script.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::install::{Host, Install, Os, Staged};

/// The folder in a portable install that holds the player's saves.
pub const SAVES: &str = "saves";

/// The file in a folder install that says an update is being copied in,
/// and where the backup of the version before it is. Also kept in the
/// staging folder, which the script copies it from.
pub const JOURNAL: &str = ".hearken-update-journal";

/// The journal's first line.
const JOURNAL_HEADER: &str = "hearken-update-journal 1";

const WINDOWS_FOLDER: &str = r#"@echo off
rem Waits for the game to close, keeps a copy of its folder, and copies the
rem update over it. saves is never touched. Until the copy is done, a
rem journal in the game's folder names the copy, and the game puts it back
rem when it next starts. The program is copied last, beside itself, and
rem renamed into place.
set tries=0
:wait
tasklist /FI "PID eq {pid}" 2>NUL | find "{pid}" >NUL
if errorlevel 1 goto closed
set /a tries+=1
if %tries% GEQ 300 goto done
ping -n 2 127.0.0.1 >NUL
goto wait
:closed
robocopy "{dst}" "{backup}" /E /XD saves /XF {journal} /R:3 /W:1 >NUL
if errorlevel 8 goto start
copy /y "{staging}\{journal}" "{dst}\{journal}" >NUL || goto start
robocopy "{src}" "{dst}" /E /XD saves /XF "{exe}" /R:10 /W:1 >NUL
if errorlevel 8 goto failed
copy /y "{src}\{exe}" "{dst}\{exe}.update-new" >NUL || goto failed
move /y "{dst}\{exe}.update-new" "{dst}\{exe}" >NUL || goto failed
del "{dst}\{journal}"
:start
start "" /D "{dst}" "{dst}\{exe}"
:done
rem The script is in the staging folder: one line, so cmd never reads on.
rmdir /s /q "{staging}" & exit /b
:failed
rem The journal stays, and so does the backup: the game puts it back.
del "{dst}\{exe}.update-new" 2>NUL
robocopy "{backup}" "{dst}" /E /XD saves /R:10 /W:1 >NUL
start "" /D "{dst}" "{dst}\{exe}"
exit /b
"#;

const POSIX_FOLDER: &str = r#"#!/bin/sh
# Waits for the game to close, keeps a copy of its folder, and copies the
# update over it. saves is never touched. Until the copy is done, a journal
# in the game's folder names the copy, and the game puts it back when it
# next starts, even if this script was stopped part-way. The program is
# copied last, beside itself, and renamed into place.
tries=0
while kill -0 {pid} 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -ge 300 ]; then rm -rf "{staging}"; exit 1; fi
  sleep 1
done
rm -rf "{src}/saves" "{src}/{journal}"
backup="{backup}"
ok=1
keep=0
mkdir -p "$backup" || ok=0
if [ "$ok" = 1 ]; then
  for entry in "{dst}"/* "{dst}"/.[!.]* "{dst}"/..?*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    [ "${entry##*/}" = saves ] && continue
    [ "${entry##*/}" = "{journal}" ] && continue
    cp -a "$entry" "$backup/" || { ok=0; break; }
  done
fi
if [ "$ok" = 1 ]; then
  cp "{staging}/{journal}" "{dst}/{journal}" || ok=0
fi
if [ "$ok" = 1 ]; then
  copied=1
  for entry in "{src}"/* "{src}"/.[!.]* "{src}"/..?*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    [ "${entry##*/}" = "{exe}" ] && continue
    cp -a "$entry" "{dst}/" || { copied=0; break; }
  done
  if [ "$copied" = 1 ] && cp -a "{src}/{exe}" "{dst}/{exe}.update-new" \
    && mv -f "{dst}/{exe}.update-new" "{dst}/{exe}"; then
    rm -f "{dst}/{journal}"
  else
    # The journal stays, and so does the backup: the game puts it back.
    rm -f "{dst}/{exe}.update-new"
    cp -a "$backup/." "{dst}/"
    keep=1
  fi
fi
cd "{dst}" && "./{exe}" >/dev/null 2>&1 &
[ "$keep" = 1 ] || rm -rf "{staging}"
"#;

const MACOS_BUNDLE: &str = r#"#!/bin/sh
# Swaps the whole app. Saves live in Application Support, never inside it.
# The new app is copied in beside the old one first, so a copy that fails
# or is stopped part-way leaves the old app as it was. Then two renames
# swap them.
tries=0
while kill -0 {pid} 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -ge 300 ]; then rm -rf "{staging}"; exit 1; fi
  sleep 1
done
if [ -e "{dst}" ]; then
  rm -rf "{dst}.update-new" "{dst}.old"
  if mv "{src}" "{dst}.update-new" 2>/dev/null || ditto "{src}" "{dst}.update-new"; then
    if mv "{dst}" "{dst}.old"; then
      if mv "{dst}.update-new" "{dst}"; then
        rm -rf "{dst}.old"
      else
        mv "{dst}.old" "{dst}"
      fi
    fi
  fi
  rm -rf "{dst}.update-new"
fi
rm -rf "{staging}"
open "{dst}"
"#;

const APPIMAGE: &str = r#"#!/bin/sh
# Swaps the AppImage file itself; the mount it runs from is read-only and
# never touched. The new file is copied in beside the old one first, so the
# rename that replaces it is one step.
tries=0
while kill -0 {pid} 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -ge 300 ]; then rm -rf "{staging}"; exit 1; fi
  sleep 1
done
if cp "{src}" "{dst}.update-new" && chmod +x "{dst}.update-new"; then
  mv -f "{dst}.update-new" "{dst}" || rm -f "{dst}.update-new"
else
  rm -f "{dst}.update-new"
fi
rm -rf "{staging}"
"{dst}" >/dev/null 2>&1 &
"#;

/// A path made safe to sit between the double quotes of an apply script.
///
/// cmd expands `%NAME%` even inside quotes, so a folder such as
/// `C:\Games\100% Fun` would send the copy somewhere else; a batch file
/// reads `%%` as one percent sign. A POSIX shell still expands `$`,
/// backticks and backslashes inside double quotes, so those are escaped.
pub fn quote_for_script(path: &str, windows: bool) -> String {
    if windows {
        let mut out = path.replace('%', "%%");
        // robocopy reads `"C:\"` as `C:"`: a closing backslash eats the quote.
        if out.ends_with('\\') {
            out.push('.');
        }
        return out;
    }
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        if matches!(ch, '\\' | '"' | '$' | '`') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Fills `{name}` placeholders in one pass, so a value that itself holds
/// `{src}` is never filled again.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 256);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            values
                .iter()
                .find(|(name, _)| *name == &after[..close])
                .map(|(_, value)| (close, value))
        });
        match value {
            Some((close, value)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The script's text, for `staged` over `install`.
pub fn script_text(
    staged: &Staged,
    install: &Install,
    staging: &Path,
    pid: u32,
    os: Os,
) -> io::Result<String> {
    let windows = os == Os::Windows;
    let q = |p: &Path| quote_for_script(&p.display().to_string(), windows);
    let (template, exe) = match (staged, install) {
        (Staged::Folder(_), Install::Folder { exe, .. }) if windows => {
            (WINDOWS_FOLDER, exe.clone())
        }
        (Staged::Folder(_), Install::Folder { exe, .. }) => (POSIX_FOLDER, exe.clone()),
        (Staged::Bundle(_), Install::Bundle { .. }) if os == Os::MacOs => {
            (MACOS_BUNDLE, String::new())
        }
        (Staged::AppImage(_), Install::AppImage { .. }) if !windows => (APPIMAGE, String::new()),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this update cannot be installed over this copy by itself",
            ))
        }
    };
    let backup = backup_dir(staging, windows);
    Ok(fill(
        template,
        &[
            ("pid", pid.to_string()),
            ("src", q(staged.path())),
            ("dst", q(install.target())),
            ("staging", q(staging)),
            ("backup", q(&backup)),
            ("exe", quote_for_script(&exe, windows)),
            ("journal", JOURNAL.to_string()),
        ],
    ))
}

/// Where the script keeps the copy of a folder install. Joined by hand, so
/// a test on one system can write another's script.
fn backup_dir(staging: &Path, windows: bool) -> PathBuf {
    if windows {
        PathBuf::from(format!("{}\\backup", staging.display()))
    } else {
        staging.join("backup")
    }
}

/// The journal's text: its header, then the backup's path.
fn journal_text(staging: &Path, os: Os) -> String {
    let backup = backup_dir(staging, os == Os::Windows);
    format!("{JOURNAL_HEADER}\n{}\n", backup.display())
}

/// Writes `text` to `path` as a new file only this user can read.
fn write_new_file(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(e) = written {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Writes the apply script inside the staging folder (which it deletes).
///
/// The file is made new, never opened over something already there, so a
/// link planted in its place is refused rather than followed. On Unix a
/// staging folder others can write to is refused too: they could swap the
/// script before it runs.
pub fn write_apply_script(
    staged: &Staged,
    install: &Install,
    staging: &Path,
    pid: u32,
    os: Os,
) -> io::Result<PathBuf> {
    let text = script_text(staged, install, staging, pid, os)?;
    let suffix = if os == Os::Windows { "bat" } else { "sh" };
    let text = if os == Os::Windows {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    let meta = fs::symlink_metadata(staging)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the update's staging folder is not a folder",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "other users can write to the update's staging folder",
            ));
        }
    }
    if matches!(install, Install::Folder { .. }) {
        let journal = staging.join(JOURNAL);
        if fs::symlink_metadata(&journal).is_err() {
            write_new_file(&journal, &journal_text(staging, os), 0o600)?;
        }
    }
    let script = staging.join(format!(
        "hearken-apply-{}.{suffix}",
        crate::install::random_suffix()
    ));
    write_new_file(&script, &text, 0o700)?;
    Ok(script)
}

/// What [`recover_interrupted`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// No update was left part-way.
    Nothing,
    /// An update was left part-way, and the version before it is back.
    Restored,
    /// An update was left part-way, and its backup is gone, so the folder
    /// is as the update left it.
    NoBackup,
}

/// Puts a folder install back as it was before an update that did not
/// finish being copied in: the apply script failed, or was stopped before
/// it removed its [`JOURNAL`]. The game calls this as it starts, before it
/// reads anything from its folder. The backup and its staging folder are
/// removed once the folder is back; doing it again after being stopped
/// part-way is safe.
pub fn recover_interrupted(install: &Install) -> io::Result<Recovery> {
    let Install::Folder { dir, .. } = install else {
        return Ok(Recovery::Nothing);
    };
    let journal = dir.join(JOURNAL);
    let meta = match fs::symlink_metadata(&journal) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Recovery::Nothing),
        Err(e) => return Err(e),
    };
    let backup = if meta.is_file() && meta.len() <= 4096 {
        fs::read_to_string(&journal)
            .ok()
            .and_then(|text| journal_backup(&text))
            .filter(|backup| backup_usable(backup, &meta))
    } else {
        None
    };
    let Some(backup) = backup else {
        log::warn!(
            "an update to {} did not finish and its backup is gone",
            dir.display()
        );
        remove_path(&journal)?;
        return Ok(Recovery::NoBackup);
    };
    log::info!(
        "an update to {} did not finish; putting {} back",
        dir.display(),
        backup.display()
    );
    restore_into(&backup, dir)?;
    prune_to(&backup, dir, true)?;
    fs::remove_file(&journal)?;
    if let Some(staging) = backup.parent() {
        let _ = fs::remove_dir_all(staging);
    }
    Ok(Recovery::Restored)
}

/// The backup a journal names.
fn journal_backup(text: &str) -> Option<PathBuf> {
    let mut lines = text.lines();
    if lines.next()? != JOURNAL_HEADER {
        return None;
    }
    let path = PathBuf::from(lines.next()?);
    path.is_absolute().then_some(path)
}

/// Whether `backup` looks like one an apply script made: a real folder
/// named `backup` in a staging folder, which on Unix only its owner, the
/// one who wrote the journal, can open.
fn backup_usable(backup: &Path, journal: &fs::Metadata) -> bool {
    let named = backup.file_name().is_some_and(|n| n == "backup");
    let Some(staging) = backup.parent() else {
        return false;
    };
    let in_staging = staging
        .file_name()
        .is_some_and(|n| n.to_string_lossy().contains("-update-"));
    let real_dir = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
    if !(named && in_staging && real_dir(backup) && real_dir(staging)) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(meta) = fs::symlink_metadata(staging) else {
            return false;
        };
        if meta.mode() & 0o077 != 0 || meta.uid() != journal.uid() {
            return false;
        }
    }
    let _ = journal;
    true
}

fn remove_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The name a restored file is copied to before it is renamed over the one
/// it replaces, so each file is either the old one or the restored one.
const RESTORING: &str = ".hearken-restore";

/// Copies everything in `from` over `to`, file by file.
fn restore_into(from: &Path, to: &Path) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let source = entry.path();
        let target = to.join(&name);
        let source_meta = fs::symlink_metadata(&source)?;
        let target_meta = fs::symlink_metadata(&target).ok();
        if source_meta.is_dir() && target_meta.as_ref().is_some_and(|m| m.is_dir()) {
            restore_into(&source, &target)?;
            continue;
        }
        let mut temp_name = name.clone();
        temp_name.push(RESTORING);
        let temp = to.join(temp_name);
        remove_path(&temp)?;
        crate::install::copy_tree(&source, &temp)?;
        // A rename replaces a file but not a folder.
        if target_meta.is_some_and(|m| m.is_dir() || source_meta.is_dir()) {
            remove_path(&target)?;
        }
        fs::rename(&temp, &target)?;
    }
    Ok(())
}

/// Removes what is in `to` but not in `from`: what the update added. Saves
/// and the journal stay.
fn prune_to(from: &Path, to: &Path, top: bool) -> io::Result<()> {
    for entry in fs::read_dir(to)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == SAVES || (top && name == JOURNAL) {
            continue;
        }
        let source = from.join(&name);
        let target = entry.path();
        match fs::symlink_metadata(&source) {
            Ok(meta) if meta.is_dir() => {
                if fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()) {
                    prune_to(&source, &target, false)?;
                }
            }
            Ok(_) => {}
            Err(_) => remove_path(&target)?,
        }
    }
    Ok(())
}

/// The command that runs `script`.
pub fn apply_command(script: &Path, os: Os) -> Vec<String> {
    let script = script.display().to_string();
    if os == Os::Windows {
        vec!["cmd".into(), "/c".into(), script]
    } else {
        vec!["/bin/sh".into(), script]
    }
}

/// Writes the apply script and hands its command to `spawn`. The caller
/// must then quit the game: the script waits for this process to exit.
pub fn apply_and_restart_with(
    staged: &Staged,
    install: &Install,
    staging: &Path,
    host: &Host,
    spawn: &mut dyn FnMut(Vec<String>) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let script = write_apply_script(staged, install, staging, host.pid, host.os)?;
    if let Err(e) = spawn(apply_command(&script, host.os)) {
        let _ = fs::remove_file(&script);
        return Err(e);
    }
    log::info!("update staged; apply script {} started", script.display());
    Ok(script)
}

/// [`apply_and_restart_with`], starting the script for real.
pub fn apply_and_restart(
    staged: &Staged,
    install: &Install,
    staging: &Path,
    host: &Host,
) -> io::Result<PathBuf> {
    apply_and_restart_with(staged, install, staging, host, &mut spawn_detached)
}

/// Starts `command` with no window, no inherited handles, in its own
/// process group, so it outlives the game.
pub fn spawn_detached(command: Vec<String>) -> io::Result<()> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty command"))?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP
        cmd.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_quoted_for_each_shell() {
        assert_eq!(
            quote_for_script(r"C:\Games\100% Fun", true),
            r"C:\Games\100%% Fun"
        );
        assert_eq!(quote_for_script(r"D:\", true), r"D:\.");
        assert_eq!(
            quote_for_script("/home/a/$HOME/`x`/\"q\"/b\\c", false),
            "/home/a/\\$HOME/\\`x\\`/\\\"q\\\"/b\\\\c"
        );
        assert_eq!(quote_for_script("/plain/path it", false), "/plain/path it");
    }

    #[test]
    fn placeholders_are_filled_once() {
        let text = fill(
            "a {src} b {dst} {unknown} ${entry##*/}",
            &[("src", "{dst}".into()), ("dst", "D".into())],
        );
        assert_eq!(text, "a {dst} b D {unknown} ${entry##*/}");
    }

    fn folder(dir: &str, exe: &str) -> Install {
        Install::Folder {
            dir: PathBuf::from(dir),
            exe: exe.into(),
        }
    }

    #[test]
    fn the_windows_script_backs_up_and_restores() {
        let text = script_text(
            &Staged::Folder(PathBuf::from(r"C:\Temp\s\unpacked\g")),
            &folder(r"C:\Games\100% Fun", "my-game.exe"),
            Path::new(r"C:\Temp\s"),
            77,
            Os::Windows,
        )
        .unwrap();
        assert!(text.contains(r#"tasklist /FI "PID eq 77""#));
        assert!(text.contains(
            r#"robocopy "C:\Games\100%% Fun" "C:\Temp\s\backup" /E /XD saves /XF .hearken-update-journal"#
        ));
        assert!(text.contains(
            r#"copy /y "C:\Temp\s\.hearken-update-journal" "C:\Games\100%% Fun\.hearken-update-journal""#
        ));
        assert!(text.contains(
            r#"robocopy "C:\Temp\s\unpacked\g" "C:\Games\100%% Fun" /E /XD saves /XF "my-game.exe""#
        ));
        // The program goes in last, by a rename.
        assert!(text.contains(
            r#"move /y "C:\Games\100%% Fun\my-game.exe.update-new" "C:\Games\100%% Fun\my-game.exe""#
        ));
        assert!(text.contains(r#"robocopy "C:\Temp\s\backup" "C:\Games\100%% Fun""#));
        assert!(
            text.contains(r#"start "" /D "C:\Games\100%% Fun" "C:\Games\100%% Fun\my-game.exe""#)
        );
        assert!(!text.contains(r"\\?\"));
        // The script is inside the staging folder, so the line that
        // deletes it also ends it.
        assert!(text.contains(r#"rmdir /s /q "C:\Temp\s" & exit /b"#));
        // The backup comes first, and a failed backup skips the copy; the
        // journal is in place before the copy, and gone only after it.
        let backup = text.find("\"C:\\Temp\\s\\backup\" /E").unwrap();
        let journal = text.find("copy /y \"C:\\Temp\\s\\.hearken").unwrap();
        let copy = text.find("robocopy \"C:\\Temp\\s\\unpacked").unwrap();
        let done = text
            .find("del \"C:\\Games\\100%% Fun\\.hearken-update-journal\"")
            .unwrap();
        assert!(backup < journal && journal < copy && copy < done);
        assert!(text[backup..copy].contains("if errorlevel 8 goto start"));
        // A failed copy keeps the staging folder, with the backup in it.
        let failed = text.find(":failed").unwrap();
        assert!(!text[failed..].contains("rmdir"));
    }

    #[test]
    fn the_posix_script_keeps_saves_and_a_backup() {
        let text = script_text(
            &Staged::Folder(PathBuf::from("/tmp/s/unpacked/g")),
            &folder("/home/p/My $Game", "my-game"),
            Path::new("/tmp/s"),
            77,
            Os::Linux,
        )
        .unwrap();
        assert!(text.contains("while kill -0 77"));
        assert!(text.contains("rm -rf \"/tmp/s/unpacked/g/saves\""));
        assert!(text.contains("backup=\"/tmp/s/backup\""));
        assert!(text.contains("[ \"${entry##*/}\" = saves ] && continue"));
        assert!(text.contains(
            "cp \"/tmp/s/.hearken-update-journal\" \"/home/p/My \\$Game/.hearken-update-journal\""
        ));
        assert!(text.contains("[ \"${entry##*/}\" = \"my-game\" ] && continue"));
        assert!(text.contains(
            "mv -f \"/home/p/My \\$Game/my-game.update-new\" \"/home/p/My \\$Game/my-game\""
        ));
        assert!(text.contains("cp -a \"$backup/.\" \"/home/p/My \\$Game/\""));
        assert!(text.contains("cd \"/home/p/My \\$Game\" && \"./my-game\""));
    }

    #[test]
    fn the_mac_script_parks_the_old_app() {
        let text = script_text(
            &Staged::Bundle(PathBuf::from("/tmp/s/unpacked/g/My Game.app")),
            &Install::Bundle {
                app: PathBuf::from("/Applications/My Game.app"),
                exe: PathBuf::from("Contents/MacOS/my-game"),
            },
            Path::new("/tmp/s"),
            77,
            Os::MacOs,
        )
        .unwrap();
        // The new app is copied in beside the old one before the old one
        // moves, so a copy stopped part-way leaves the old app alone.
        let beside = text
            .find(
                "ditto \"/tmp/s/unpacked/g/My Game.app\" \"/Applications/My Game.app.update-new\"",
            )
            .unwrap();
        let aside = text
            .find("mv \"/Applications/My Game.app\" \"/Applications/My Game.app.old\"")
            .unwrap();
        let swap = text
            .find("mv \"/Applications/My Game.app.update-new\" \"/Applications/My Game.app\"")
            .unwrap();
        assert!(beside < aside && aside < swap);
        assert!(text.contains("mv \"/Applications/My Game.app.old\" \"/Applications/My Game.app\""));
        assert!(text.contains("open \"/Applications/My Game.app\""));
    }

    #[test]
    fn the_appimage_script_swaps_one_file() {
        let text = script_text(
            &Staged::AppImage(PathBuf::from("/tmp/s/My.AppImage")),
            &Install::AppImage {
                file: PathBuf::from("/home/p/Apps/My.AppImage"),
            },
            Path::new("/tmp/s"),
            77,
            Os::Linux,
        )
        .unwrap();
        assert!(text.contains("cp \"/tmp/s/My.AppImage\" \"/home/p/Apps/My.AppImage.update-new\""));
        assert!(text.contains(
            "mv -f \"/home/p/Apps/My.AppImage.update-new\" \"/home/p/Apps/My.AppImage\""
        ));
    }

    #[test]
    fn mismatched_updates_get_no_script() {
        let err = script_text(
            &Staged::Folder(PathBuf::from("/tmp/s/g")),
            &Install::AppImage {
                file: PathBuf::from("/a/My.AppImage"),
            },
            Path::new("/tmp/s"),
            1,
            Os::Linux,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn the_script_is_written_and_handed_to_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
        let host = Host::fake(Os::Linux, &dir.path().join("game/my-game"));
        let mut seen = Vec::new();
        let script = apply_and_restart_with(
            &Staged::Folder(staging.join("g")),
            &folder("/games/g", "my-game"),
            &staging,
            &host,
            &mut |command| {
                seen = command;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(script.parent(), Some(staging.as_path()));
        assert_eq!(seen, ["/bin/sh".to_string(), script.display().to_string()]);
        // A spawn that fails leaves no script behind.
        let err = apply_and_restart_with(
            &Staged::Folder(staging.join("g")),
            &folder("/games/g", "my-game"),
            &staging,
            &host,
            &mut |_| Err(io::Error::other("no shell")),
        );
        assert!(err.is_err());
        let mut left: Vec<_> = fs::read_dir(&staging)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        left.sort();
        let mut expected = vec![script, staging.join(JOURNAL)];
        expected.sort();
        assert_eq!(left, expected);
    }

    /// Another user on the machine plants links where the script used to
    /// go (`/tmp/hearken-apply-<pid>.sh`, the pid anyone can see): the
    /// player's file they point at must never be written, and the script
    /// must not be somewhere they can swap it.
    #[cfg(unix)]
    #[test]
    fn a_planted_link_never_redirects_the_script() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        fs::write(&victim, b"IMPORTANT DATA").unwrap();
        let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
        for at in [
            dir.path().join("hearken-apply-4242.sh"),
            staging.join("hearken-apply-4242.sh"),
        ] {
            std::os::unix::fs::symlink(&victim, at).unwrap();
        }
        let script = write_apply_script(
            &Staged::Folder(staging.join("g")),
            &folder("/games/g", "my-game"),
            &staging,
            4242,
            Os::Linux,
        )
        .unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"IMPORTANT DATA");
        let meta = fs::symlink_metadata(&script).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.permissions().mode() & 0o077, 0);
        assert_eq!(script.parent(), Some(staging.as_path()));
        // A second script never lands on the first one's name.
        let again = write_apply_script(
            &Staged::Folder(staging.join("g")),
            &folder("/games/g", "my-game"),
            &staging,
            4242,
            Os::Linux,
        )
        .unwrap();
        assert_ne!(again, script);
    }

    /// A staging folder other users can write to could have its script
    /// swapped before it runs, so none is written there.
    #[cfg(unix)]
    #[test]
    fn no_script_goes_in_a_folder_others_can_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).unwrap();
        let err = write_apply_script(
            &Staged::Folder(shared.join("g")),
            &folder("/games/g", "my-game"),
            &shared,
            1,
            Os::Linux,
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_dir(&shared).unwrap().count(), 0);
        // Nor in a link to a folder.
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path(), &link).unwrap();
        assert!(write_apply_script(
            &Staged::Folder(link.join("g")),
            &folder("/games/g", "my-game"),
            &link,
            1,
            Os::Linux,
        )
        .is_err());
    }

    /// The real POSIX script, run against a real folder: the update lands,
    /// saves stay, and the game is started again.
    #[cfg(unix)]
    #[test]
    fn the_posix_script_really_updates_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("My $Game");
        fs::create_dir_all(game.join("saves")).unwrap();
        fs::write(game.join("saves/progress.json"), b"mine").unwrap();
        fs::write(game.join("data.txt"), b"old").unwrap();
        fs::write(game.join("only-old.txt"), b"kept").unwrap();
        let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
        let new = staging.join("unpacked/g");
        fs::create_dir_all(new.join("saves")).unwrap();
        fs::write(new.join("saves/progress.json"), b"theirs").unwrap();
        fs::write(new.join("data.txt"), b"new").unwrap();
        let started = dir.path().join("started");
        let exe = format!("#!/bin/sh\necho yes > \"{}\"\n", started.display());
        fs::write(new.join("my-game"), exe).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(new.join("my-game"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let install = Install::Folder {
            dir: game.clone(),
            exe: "my-game".into(),
        };
        // A pid that is surely not running.
        let script = write_apply_script(
            &Staged::Folder(new),
            &install,
            &staging,
            999_999_999,
            Os::Linux,
        )
        .unwrap();
        let status = Command::new("/bin/sh").arg(&script).status().unwrap();
        assert!(status.success());
        for _ in 0..100 {
            if started.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(fs::read(game.join("data.txt")).unwrap(), b"new");
        assert_eq!(fs::read(game.join("only-old.txt")).unwrap(), b"kept");
        assert_eq!(fs::read(game.join("saves/progress.json")).unwrap(), b"mine");
        assert!(started.exists(), "the game was not started again");
        assert!(!staging.exists() && !script.exists());
    }

    /// A copy that fails part-way is undone from the backup.
    #[cfg(unix)]
    #[test]
    fn the_posix_script_restores_a_failed_copy() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        // A folder where the update has a file: cp cannot replace it, even
        // as root, and goes on to copy the rest before it fails.
        fs::create_dir_all(game.join("clash/inside")).unwrap();
        fs::write(game.join("a.txt"), b"old").unwrap();
        fs::write(game.join("my-game"), b"#!/bin/sh\n").unwrap();
        let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
        let new = staging.join("unpacked/g");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("a.txt"), b"new").unwrap();
        fs::write(new.join("clash"), b"a file").unwrap();
        fs::write(new.join("my-game"), b"#!/bin/sh\n").unwrap();
        let install = Install::Folder {
            dir: game.clone(),
            exe: "my-game".into(),
        };
        let script = write_apply_script(
            &Staged::Folder(new),
            &install,
            &staging,
            999_999_999,
            Os::Linux,
        )
        .unwrap();
        Command::new("/bin/sh").arg(&script).status().unwrap();
        assert_eq!(fs::read(game.join("a.txt")).unwrap(), b"old");
        assert!(game.join("clash/inside").is_dir());
        // The journal and the backup stay for the next start, which puts
        // the folder back exactly.
        assert!(game.join(JOURNAL).is_file());
        assert!(staging.join("backup").is_dir());
        assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Restored);
        assert!(!game.join(JOURNAL).exists() && !staging.exists());
        assert_eq!(fs::read(game.join("a.txt")).unwrap(), b"old");
        assert!(game.join("clash/inside").is_dir());
        assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Nothing);
    }

    /// Where the real `cp` is, for a stand-in that calls it.
    #[cfg(unix)]
    fn real_cp() -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join("cp"))
            .find(|cp| cp.is_file())
            .unwrap()
    }

    /// The apply script is killed part-way through copying the update in
    /// (the power goes, the computer shuts down): the folder is half old,
    /// half new, and nothing in the script gets to undo it. The next start
    /// of the game must put the old version back from the backup, saves
    /// untouched.
    #[cfg(unix)]
    #[test]
    fn an_apply_killed_part_way_is_put_back_at_the_next_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("saves")).unwrap();
        fs::write(game.join("saves/progress.json"), b"mine").unwrap();
        fs::create_dir_all(game.join("data")).unwrap();
        fs::write(game.join("data/level.txt"), b"old").unwrap();
        fs::write(game.join("data.txt"), b"old").unwrap();
        fs::write(game.join("my-game"), b"old program").unwrap();
        let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
        let new = staging.join("unpacked/g");
        fs::create_dir_all(new.join("data")).unwrap();
        fs::write(new.join("data/level.txt"), b"new").unwrap();
        fs::write(new.join("data/added.txt"), b"new").unwrap();
        fs::write(new.join("data.txt"), b"new").unwrap();
        fs::write(new.join("new-only.txt"), b"new").unwrap();
        fs::write(new.join("my-game"), b"new program").unwrap();
        let install = Install::Folder {
            dir: game.clone(),
            exe: "my-game".into(),
        };
        let script = write_apply_script(
            &Staged::Folder(new),
            &install,
            &staging,
            999_999_999,
            Os::Linux,
        )
        .unwrap();
        // A cp that kills the script once it has copied part of the update.
        let shim = dir.path().join("shim");
        fs::create_dir(&shim).unwrap();
        let cp = format!(
            "#!/bin/sh\n\"{}\" \"$@\"\ncase \"$*\" in *unpacked/g/data.txt*) kill -9 $PPID ;; esac\n",
            real_cp().display()
        );
        fs::write(shim.join("cp"), cp).unwrap();
        fs::set_permissions(shim.join("cp"), fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            shim.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let status = Command::new("/bin/sh")
            .arg(&script)
            .env("PATH", path)
            .status()
            .unwrap();
        assert!(!status.success(), "the script was meant to be killed");
        // Half-way: some of the update is in, the old program is whole.
        assert_eq!(fs::read(game.join("data.txt")).unwrap(), b"new");
        assert_eq!(fs::read(game.join("data/added.txt")).unwrap(), b"new");
        assert_eq!(fs::read(game.join("my-game")).unwrap(), b"old program");
        assert!(game.join(JOURNAL).is_file());

        assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Restored);
        assert_eq!(fs::read(game.join("data.txt")).unwrap(), b"old");
        assert_eq!(fs::read(game.join("data/level.txt")).unwrap(), b"old");
        assert!(!game.join("data/added.txt").exists());
        assert!(!game.join("new-only.txt").exists());
        assert_eq!(fs::read(game.join("my-game")).unwrap(), b"old program");
        assert_eq!(fs::read(game.join("saves/progress.json")).unwrap(), b"mine");
        assert!(!game.join(JOURNAL).exists());
        assert!(!staging.exists());
        let mut names: Vec<_> = fs::read_dir(&game)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["data", "data.txt", "my-game", "saves"]);
    }

    /// A journal whose backup is gone, or that names something that is not
    /// a backup, changes nothing and is removed.
    #[cfg(unix)]
    #[test]
    fn a_journal_without_a_real_backup_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("my-game"), b"program").unwrap();
        let elsewhere = dir.path().join("documents");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("my-game"), b"not the game").unwrap();
        let install = Install::Folder {
            dir: game.clone(),
            exe: "my-game".into(),
        };
        let gone = dir.path().join("my-game-update-1-abc/backup");
        for named in [gone, elsewhere.clone(), PathBuf::from("relative/backup")] {
            fs::write(
                game.join(JOURNAL),
                format!("{JOURNAL_HEADER}\n{}\n", named.display()),
            )
            .unwrap();
            assert_eq!(recover_interrupted(&install).unwrap(), Recovery::NoBackup);
            assert!(!game.join(JOURNAL).exists());
            assert_eq!(fs::read(game.join("my-game")).unwrap(), b"program");
        }
        assert_eq!(
            fs::read(elsewhere.join("my-game")).unwrap(),
            b"not the game"
        );
        assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Nothing);
    }
}
