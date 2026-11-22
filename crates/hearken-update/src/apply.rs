//! Swapping the update in: a small script, started detached, that waits
//! for the game to exit, replaces the install and starts the game again.
//!
//! A failure part-way must never leave the player without a game:
//!
//! - folder installs keep a backup of every file the update replaces,
//!   made before the game quits (see [`back_up_replaced`]): only those
//!   files, so a big file the update leaves alone is never copied. Without
//!   a backup, nothing is touched. While the update is copied over, a journal in the game's
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
//!   renames, the only moment without an app. An app made portable by
//!   hand keeps `saves`, and any other file of the player's, beside its
//!   program, inside the app: those are moved into the new app, and if any
//!   cannot be, they are moved back and the old app is put back. An old
//!   app still holding the player's files is never removed (see
//!   [`crate::recover::old_app_left`]);
//! - an AppImage is copied in beside the old one and renamed over it, which
//!   is one step that either happens or does not.
//!
//! The script waits up to five minutes for the game to exit, and gives up
//! without touching anything if it never does. On Windows it then waits for
//! the program file itself to be free, and retries a file that is still
//! locked (by antivirus, say) for a minute before it gives up and puts the
//! backup back. Each step, and any file that failed, goes to
//! [`APPLY_LOG`] in the game's folder, which the next start copies into
//! the game's log.
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
use crate::recover::remove_path;
pub use crate::recover::{recover_interrupted, Recovery, APPLY_LOG};

/// The folder in a portable install that holds the player's saves.
pub const SAVES: &str = "saves";

/// The file in a folder install that says an update is being copied in,
/// and where the backup of the version before it is. Also kept in the
/// staging folder, which the script copies it from.
pub const JOURNAL: &str = ".hearken-update-journal";

/// The journal's first line.
pub(crate) const JOURNAL_HEADER: &str = "hearken-update-journal 1";

/// The first line of a journal whose backup holds only the files the update
/// replaces; the lines after the backup name what the update adds. A copy
/// that only knows [`JOURNAL_HEADER`] leaves such a journal's folder alone
/// rather than prune it to a partial backup.
pub(crate) const JOURNAL_HEADER_2: &str = "hearken-update-journal 2";

/// The most a journal may hold: its backup's path and every path the
/// update adds.
pub(crate) const MAX_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;

const WINDOWS_FOLDER: &str = r#"@echo off
rem Waits for the game to close and copies the update over its folder. The
rem game already backed up each file the update replaces. saves is never
rem touched. Until the copy is done, a journal in the game's folder names
rem the backup, and the game puts it back when it next starts. The program
rem is copied last, beside itself, and renamed into place. Each step goes
rem to a log in the game's folder, which the game reads when it next starts.
set log="{dst}\{log}"
>>%log% echo %date% %time% waiting for the game to close
set tries=0
:wait
tasklist /FI "PID eq {pid}" 2>NUL | find "{pid}" >NUL
if errorlevel 1 goto closed
set /a tries+=1
if %tries% GEQ 300 goto gaveup
ping -n 2 127.0.0.1 >NUL
goto wait
:gaveup
>>%log% echo %date% %time% the game did not close in five minutes, so nothing was changed
goto done
:closed
rem Windows can hold the program a moment after the game has gone, and
rem antivirus may be reading it: wait until it can be written.
set tries=0
:unlock
2>NUL (>>"{dst}\{exe}" call ) && goto unlocked
set /a tries+=1
if %tries% GEQ 60 goto unlocked
ping -n 2 127.0.0.1 >NUL
goto unlock
:unlocked
>>%log% echo %date% %time% the game closed; waited %tries% seconds for the program to be free
copy /y "{staging}\{journal}" "{dst}\{journal}" >NUL || goto nojournal
robocopy "{src}" "{dst}" /E /XD saves /XF "{exe}" /R:30 /W:2 /NP /NDL /NFL /NJH /LOG+:%log% >NUL
set code=%errorlevel%
if %code% GEQ 8 goto failed
set tries=0
:program
copy /y "{src}\{exe}" "{dst}\{exe}.update-new" >NUL && move /y "{dst}\{exe}.update-new" "{dst}\{exe}" >NUL && goto copied
set /a tries+=1
set code=program
if %tries% GEQ 30 goto failed
ping -n 3 127.0.0.1 >NUL
goto program
:copied
del "{dst}\{journal}"
>>%log% echo %date% %time% the update is in
:start
start "" /D "{dst}" "{dst}\{exe}"
:done
rem The script is in the staging folder: one line, so cmd never reads on.
rmdir /s /q "{staging}" & exit /b
:nojournal
>>%log% echo %date% %time% writing the journal failed, so nothing was changed
goto start
:failed
rem The journal stays, and so does the backup: the game puts it back.
>>%log% echo %date% %time% copying the update failed, code %code%; putting the backup back
del "{dst}\{exe}.update-new" 2>NUL
robocopy "{backup}" "{dst}" /E /XD saves /R:30 /W:2 /NP /NDL /NFL /NJH /LOG+:%log% >NUL
start "" /D "{dst}" "{dst}\{exe}"
exit /b
"#;

const POSIX_FOLDER: &str = r#"#!/bin/sh
# Waits for the game to close and copies the update over its folder. The
# game already backed up each file the update replaces. saves is never touched. Until the copy is done, a journal
# in the game's folder names the copy, and the game puts it back when it
# next starts, even if this script was stopped part-way. The program is
# copied last, beside itself, and renamed into place. Each step goes to a
# log in the game's folder, which the game reads when it next starts.
log="{dst}/{log}"
note() { echo "$(date) $*" >>"$log" 2>/dev/null; }
note waiting for the game to close
tries=0
while kill -0 {pid} 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -ge 300 ]; then
    note the game did not close in five minutes, so nothing was changed
    rm -rf "{staging}"; exit 1
  fi
  sleep 1
done
note the game closed
rm -rf "{src}/saves" "{src}/{journal}"
backup="{backup}"
ok=1
keep=0
[ -d "$backup" ] || { ok=0; note the backup is missing, so nothing was changed; }
if [ "$ok" = 1 ]; then
  cp "{staging}/{journal}" "{dst}/{journal}" || { ok=0; note writing the journal failed, so nothing was changed; }
fi
if [ "$ok" = 1 ]; then
  copied=1
  for entry in "{src}"/* "{src}"/.[!.]* "{src}"/..?*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    [ "${entry##*/}" = "{exe}" ] && continue
    cp -a "$entry" "{dst}/" 2>>"$log" || { copied=0; note copying "${entry##*/}" failed; break; }
  done
  if [ "$copied" = 1 ] && cp -a "{src}/{exe}" "{dst}/{exe}.update-new" 2>>"$log" \
    && mv -f "{dst}/{exe}.update-new" "{dst}/{exe}" 2>>"$log"; then
    rm -f "{dst}/{journal}"
    note the update is in
  else
    # The journal stays, and so does the backup: the game puts it back.
    note copying the update failed, putting the backup back
    rm -f "{dst}/{exe}.update-new"
    cp -a "$backup/." "{dst}/" 2>>"$log"
    keep=1
  fi
fi
cd "{dst}" && "./{exe}" >/dev/null 2>&1 &
[ "$keep" = 1 ] || rm -rf "{staging}"
"#;

const MACOS_BUNDLE: &str = r#"#!/bin/sh
# Swaps the whole app. Saves live in Application Support, except in an app
# made portable by hand, which keeps them beside its program, inside the
# app, with anything else of the player's there (exported music, say).
# Those are moved into the new app; if any cannot be, they are moved back
# and the old app is put back as it was. An old app that still holds the
# player's files is never removed: the update waits until the player has
# dealt with it. The new app is copied in beside the old one first, so a
# copy that fails or is stopped part-way leaves the old app as it was.
# Then two renames swap them. What goes wrong is written to a log beside
# the app, which the game reads when it next starts.
log="{dst}{log}"
note() { echo "$(date) $*" >>"$log" 2>/dev/null; }
old="{dst}.old/{programs}"
new="{dst}/{programs}"
carried="{staging}/carried"
there() { [ -e "$1" ] || [ -L "$1" ]; }
# Whether the old app holds a file of the player's that the app at {dst}
# lacks: saves, or, in a portable app, anything beside its program.
holds_mine() {
  [ -d "$old" ] || return 1
  there "$old/saves" && return 0
  [ -f "$new/{marker}" ] || return 1
  for entry in "$old"/* "$old"/.[!.]* "$old"/..?*; do
    there "$entry" || continue
    there "$new/${entry##*/}" || return 0
  done
  return 1
}
# Moves the player's files from the old app into the new one. Each name is
# written down before it moves, so uncarry can move it back.
carry() {
  : >"$carried" || return 1
  if there "$old/saves"; then rm -rf "$new/saves" || return 1; fi
  for entry in "$old"/* "$old"/.[!.]* "$old"/..?*; do
    there "$entry" || continue
    name="${entry##*/}"
    [ "$name" = saves ] || [ -f "$new/{marker}" ] || continue
    there "$new/$name" && continue
    case "$name" in *"
"*) note a file name has a line break in it; return 1;; esac
    printf '%s\n' "$name" >>"$carried" || return 1
    mv "$entry" "$new/$name" 2>>"$log" || { note moving "$name" into the new app failed; return 1; }
  done
}
uncarry() {
  [ -f "$carried" ] || return 1
  while IFS= read -r name; do
    if there "$new/$name" && ! there "$old/$name"; then
      mv "$new/$name" "$old/$name" 2>>"$log" || return 1
    fi
  done <"$carried"
}
tries=0
while kill -0 {pid} 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -ge 300 ]; then rm -rf "{staging}"; exit 1; fi
  sleep 1
done
if [ -e "{dst}" ]; then
  if holds_mine; then
    note "{dst}.old" still holds files of the player from an earlier update, so {not_installed}
    rm -rf "{staging}"
    open "{dst}"
    exit 1
  fi
  rm -rf "{dst}.update-new" "{dst}.old"
  if mv "{src}" "{dst}.update-new" 2>/dev/null || ditto "{src}" "{dst}.update-new"; then
    if mv "{dst}" "{dst}.old"; then
      if mv "{dst}.update-new" "{dst}"; then
        if carry; then
          rm -rf "{dst}.old"
        elif uncarry && mv "{dst}" "{dst}.update-new" 2>>"$log"; then
          if mv "{dst}.old" "{dst}" 2>>"$log"; then
            note files of the player could not be moved into the new app, so {not_installed}
          else
            mv "{dst}.update-new" "{dst}"
            note the old app could not be put back, so it is kept at "{dst}.old"
          fi
        else
          note the old app could not be put back, so it is kept at "{dst}.old"
        fi
      else
        mv "{dst}.old" "{dst}"
      fi
    fi
  fi
  [ -e "{dst}" ] && rm -rf "{dst}.update-new"
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
    // The program's folder inside an app: where a portable app keeps its
    // saves.
    let mut programs = String::new();
    let (template, exe) = match (staged, install) {
        (Staged::Folder(_), Install::Folder { exe, .. }) if windows => {
            (WINDOWS_FOLDER, exe.clone())
        }
        (Staged::Folder(_), Install::Folder { exe, .. }) => (POSIX_FOLDER, exe.clone()),
        (Staged::Bundle(_), Install::Bundle { exe, .. }) if os == Os::MacOs => {
            programs = q(exe.parent().unwrap_or(Path::new("")));
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
            ("programs", programs),
            ("journal", JOURNAL.to_string()),
            ("log", APPLY_LOG.to_string()),
            ("marker", crate::install::PORTABLE_MARKER.to_string()),
            (
                "not_installed",
                crate::recover::NOT_INSTALLED_LINE.to_string(),
            ),
        ],
    ))
}

/// Where the script keeps the copy of a folder install. Joined by hand, so
/// a test on one system can write another's script.
fn backup_dir(staging: &Path, windows: bool) -> PathBuf {
    if windows {
        PathBuf::from(format!("{}\\backup", staging.display()))
    } else {
        PathBuf::from(format!("{}/backup", staging.display()))
    }
}

/// The journal's text: its header, the backup's path, then each path the
/// update adds, relative to the game's folder with `/` between parts.
fn journal_text(staging: &Path, os: Os, added: &[String]) -> String {
    let backup = backup_dir(staging, os == Os::Windows);
    let mut text = format!("{JOURNAL_HEADER_2}\n{}\n", backup.display());
    for path in added {
        text.push_str(path);
        text.push('\n');
    }
    text
}

/// Copies into `backup` each file of `dir` that the update in `new` will
/// replace, and returns what the update adds that `dir` does not have,
/// as `/`-separated paths from the top. The game's `saves` and the journal
/// are left out. The game is still running: nothing in `dir` changes.
///
/// Only what is replaced is copied, so an unchanged big file left in place
/// by a partial download costs nothing here.
pub fn back_up_replaced(new: &Path, dir: &Path, backup: &Path) -> io::Result<Vec<String>> {
    fs::create_dir_all(backup)?;
    let mut added = Vec::new();
    back_up_inner(new, dir, backup, "", &mut added)?;
    Ok(added)
}

fn back_up_inner(
    new: &Path,
    dir: &Path,
    backup: &Path,
    rel: &str,
    added: &mut Vec<String>,
) -> io::Result<()> {
    for entry in fs::read_dir(new)? {
        let entry = entry?;
        let name = entry.file_name();
        if rel.is_empty() && (name == SAVES || name == JOURNAL) {
            continue;
        }
        let name_text = name
            .to_str()
            .ok_or_else(|| io::Error::other("an update file's name is not text"))?;
        let path = if rel.is_empty() {
            name_text.to_string()
        } else {
            format!("{rel}/{name_text}")
        };
        let target = dir.join(&name);
        let new_meta = fs::symlink_metadata(entry.path())?;
        match fs::symlink_metadata(&target) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => added.push(path),
            Err(e) => return Err(e),
            Ok(old) if old.is_dir() && new_meta.is_dir() => {
                let kept = backup.join(&name);
                fs::create_dir_all(&kept)?;
                fs::set_permissions(&kept, old.permissions())?;
                back_up_inner(&entry.path(), &target, &kept, &path, added)?;
            }
            Ok(_) => crate::install::copy_tree(&target, &backup.join(&name))?,
        }
    }
    Ok(())
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
    if let (Install::Folder { dir, .. }, Staged::Folder(new)) = (install, staged) {
        let journal = staging.join(JOURNAL);
        if fs::symlink_metadata(&journal).is_err() {
            let backup = backup_dir(staging, os == Os::Windows);
            if fs::symlink_metadata(&backup).is_ok() {
                remove_path(&backup)?;
            }
            let added = back_up_replaced(new, dir, &backup)?;
            write_new_file(&journal, &journal_text(staging, os, &added), 0o600)?;
        }
    }
    let script = staging.join(format!(
        "hearken-apply-{}.{suffix}",
        crate::install::random_suffix()
    ));
    write_new_file(&script, &text, 0o700)?;
    Ok(script)
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
        const FLAGS: u32 = 0x0800_0000 | 0x0000_0200;
        // CREATE_BREAKAWAY_FROM_JOB: a launcher or terminal that runs the
        // game in a job closes every process in it when the game exits,
        // which would stop the script part-way. A job that forbids leaving
        // refuses the spawn, so it is tried again inside the job.
        cmd.creation_flags(FLAGS | 0x0100_0000);
        if cmd.spawn().is_ok() {
            return Ok(());
        }
        cmd.creation_flags(FLAGS);
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
    #[cfg(unix)]
    use crate::recover::journal_backup;

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
        // The game made the backup before it quit, so the script never
        // copies the whole folder; the journal is in place before the
        // copy, and gone only after it.
        assert!(!text.contains(r#"robocopy "C:\Games\100%% Fun" "C:\Temp\s\backup""#));
        let journal = text.find("copy /y \"C:\\Temp\\s\\.hearken").unwrap();
        let copy = text.find("robocopy \"C:\\Temp\\s\\unpacked").unwrap();
        let done = text
            .find("del \"C:\\Games\\100%% Fun\\.hearken-update-journal\"")
            .unwrap();
        assert!(journal < copy && copy < done);
        // Each step is logged in the game's folder; the program must be
        // free before anything is copied, and a locked file is retried.
        assert!(text.contains(r#"set log="C:\Games\100%% Fun\.hearken-update.log""#));
        assert!(
            text.contains(r#"2>NUL (>>"C:\Games\100%% Fun\my-game.exe" call ) && goto unlocked"#)
        );
        assert!(text.contains("/R:30 /W:2 /NP /NDL /NFL /NJH /LOG+:%log%"));
        let unlock = text.find(":unlock").unwrap();
        assert!(unlock < journal);
        assert!(text.contains("if %tries% GEQ 30 goto failed"));
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
        // No backup without the one the game made, and no copy of the
        // whole folder.
        assert!(text.contains("[ -d \"$backup\" ] || { ok=0;"));
        assert!(text.contains("log=\"/home/p/My \\$Game/.hearken-update.log\""));
        assert!(!text.contains("\"$backup/\" ||"));
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
        // A portable app's own files move into the new app before the old
        // one goes; if they cannot, the old app is put back.
        let carry = text
            .find("if carry; then\n          rm -rf \"/Applications/My Game.app.old\"")
            .unwrap();
        assert!(swap < carry);
        assert!(text.contains("old=\"/Applications/My Game.app.old/Contents/MacOS\""));
        assert!(text.contains("[ -f \"$new/portable.txt\" ]"));
        assert!(text.contains(&format!("so {}", crate::recover::NOT_INSTALLED_LINE)));
        assert!(text.contains("log=\"/Applications/My Game.app.hearken-update.log\""));
        // An old app still holding the player's files is never removed.
        let guard = text.find("if holds_mine; then").unwrap();
        let clear = text
            .find(
                "rm -rf \"/Applications/My Game.app.update-new\" \"/Applications/My Game.app.old\"",
            )
            .unwrap();
        assert!(guard < clear);
    }

    /// A Mac app made portable by hand, in a temporary folder, and an
    /// update for it staged as the update screen stages one.
    #[cfg(unix)]
    struct MacCase {
        dir: tempfile::TempDir,
        programs: PathBuf,
        staging: PathBuf,
        new: PathBuf,
        install: Install,
    }

    #[cfg(unix)]
    impl MacCase {
        fn new() -> MacCase {
            let dir = tempfile::tempdir().unwrap();
            let app = dir.path().join("Apps/My Game.app");
            let programs = app.join("Contents/MacOS");
            fs::create_dir_all(programs.join("saves")).unwrap();
            fs::write(programs.join("saves/progress.json"), b"mine").unwrap();
            fs::write(programs.join("my-game"), b"old").unwrap();
            fs::write(programs.join(crate::install::PORTABLE_MARKER), b"by hand").unwrap();
            let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
            let new = staging.join("unpacked/g/My Game.app");
            fs::create_dir_all(new.join("Contents/MacOS/saves")).unwrap();
            fs::write(new.join("Contents/MacOS/my-game"), b"new").unwrap();
            fs::write(new.join("Contents/MacOS/saves/progress.json"), b"theirs").unwrap();
            fs::write(
                new.join("Contents/MacOS")
                    .join(crate::install::PORTABLE_MARKER),
                b"by hand",
            )
            .unwrap();
            let install = Install::Bundle {
                app: app.clone(),
                exe: PathBuf::from("Contents/MacOS/my-game"),
            };
            MacCase {
                dir,
                programs,
                staging,
                new,
                install,
            }
        }

        fn old_app(&self) -> PathBuf {
            self.dir.path().join("Apps/My Game.app.old")
        }

        /// Runs the real script with a stand-in `open`, and a `mv` that
        /// fails to move anything named `fail_mv`, when given. Returns
        /// whether it succeeded.
        fn run(&self, fail_mv: Option<&str>) -> bool {
            use std::os::unix::fs::PermissionsExt;
            let script = write_apply_script(
                &Staged::Bundle(self.new.clone()),
                &self.install,
                &self.staging,
                999_999_999,
                Os::MacOs,
            )
            .unwrap();
            let shim = self.dir.path().join("shim");
            fs::create_dir_all(&shim).unwrap();
            fs::write(shim.join("open"), "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(shim.join("open"), fs::Permissions::from_mode(0o755)).unwrap();
            let _ = fs::remove_file(shim.join("mv"));
            if let Some(name) = fail_mv {
                let real = ["/bin/mv", "/usr/bin/mv"]
                    .into_iter()
                    .find(|p| Path::new(p).exists())
                    .unwrap();
                fs::write(
                    shim.join("mv"),
                    format!(
                        "#!/bin/sh\ncase \"$1\" in */{name}) exit 1;; esac\nexec {real} \"$@\"\n"
                    ),
                )
                .unwrap();
                fs::set_permissions(shim.join("mv"), fs::Permissions::from_mode(0o755)).unwrap();
            }
            let path = format!(
                "{}:{}",
                shim.display(),
                std::env::var("PATH").unwrap_or_default()
            );
            Command::new("/bin/sh")
                .arg(&script)
                .env("PATH", path)
                .status()
                .unwrap()
                .success()
        }
    }

    /// The real Mac script, run by a POSIX shell against a real app made
    /// portable by hand: the new app is in place, with the saves and the
    /// marker, and the old one is gone. `open` is a stand-in.
    #[cfg(unix)]
    #[test]
    fn the_mac_script_carries_a_portable_apps_saves() {
        let case = MacCase::new();
        assert!(case.run(None));
        let programs = &case.programs;
        assert_eq!(fs::read(programs.join("my-game")).unwrap(), b"new");
        assert_eq!(
            fs::read(programs.join("saves/progress.json")).unwrap(),
            b"mine"
        );
        assert!(programs.join(crate::install::PORTABLE_MARKER).is_file());
        assert!(!case.old_app().exists());
        assert!(!case.dir.path().join("Apps/My Game.app.update-new").exists());
        assert!(!case.staging.exists());
        assert_eq!(
            recover_interrupted(&case.install).unwrap(),
            Recovery::Nothing
        );
    }

    /// Everything else of the player's beside a portable app's program
    /// (music exported there, the seal witness, a hidden file) moves into
    /// the new app too, and the update's own files win over none of them
    /// but stay.
    #[cfg(unix)]
    #[test]
    fn the_mac_script_carries_a_portable_apps_own_files() {
        let case = MacCase::new();
        let programs = &case.programs;
        fs::create_dir_all(programs.join("Music")).unwrap();
        fs::write(programs.join("Music/Lake.ogg"), b"song").unwrap();
        fs::write(programs.join("MyGame.hearken-seal"), b"witness").unwrap();
        fs::write(programs.join(".hidden"), b"dot").unwrap();
        fs::write(case.new.join("Contents/MacOS/data.pack"), b"shipped").unwrap();
        assert!(case.run(None));
        assert_eq!(fs::read(programs.join("my-game")).unwrap(), b"new");
        assert_eq!(fs::read(programs.join("Music/Lake.ogg")).unwrap(), b"song");
        assert_eq!(
            fs::read(programs.join("MyGame.hearken-seal")).unwrap(),
            b"witness"
        );
        assert_eq!(fs::read(programs.join(".hidden")).unwrap(), b"dot");
        assert_eq!(fs::read(programs.join("data.pack")).unwrap(), b"shipped");
        assert_eq!(
            fs::read(programs.join("saves/progress.json")).unwrap(),
            b"mine"
        );
        assert!(!case.old_app().exists());
    }

    /// An installed app (no marker) carries only a stray saves folder, as
    /// before: nothing else of the old app's is moved into the new one.
    #[cfg(unix)]
    #[test]
    fn the_mac_script_carries_nothing_else_for_an_installed_app() {
        let case = MacCase::new();
        fs::remove_file(case.programs.join(crate::install::PORTABLE_MARKER)).unwrap();
        fs::remove_file(
            case.new
                .join("Contents/MacOS")
                .join(crate::install::PORTABLE_MARKER),
        )
        .unwrap();
        fs::write(case.programs.join("dropped.dylib"), b"old lib").unwrap();
        assert!(case.run(None));
        assert_eq!(fs::read(case.programs.join("my-game")).unwrap(), b"new");
        assert!(!case.programs.join("dropped.dylib").exists());
        assert!(!case.old_app().exists());
    }

    /// The saves cannot be moved into the new app: the old app is put back
    /// whole, with its saves and every other file of the player's, the new
    /// one is gone, and the next start tells the player the update was not
    /// installed. A later update then works as usual.
    #[cfg(unix)]
    #[test]
    fn the_mac_script_puts_the_old_app_back_when_the_saves_cannot_move() {
        let case = MacCase::new();
        let programs = &case.programs;
        fs::create_dir_all(programs.join("Music")).unwrap();
        fs::write(programs.join("Music/Lake.ogg"), b"song").unwrap();
        // "Music" sorts before "saves", so it has moved when saves fails.
        // The script still opens the app, so it ends as a success.
        case.run(Some("saves"));
        assert_eq!(fs::read(programs.join("my-game")).unwrap(), b"old");
        assert_eq!(
            fs::read(programs.join("saves/progress.json")).unwrap(),
            b"mine"
        );
        assert_eq!(fs::read(programs.join("Music/Lake.ogg")).unwrap(), b"song");
        assert!(programs.join(crate::install::PORTABLE_MARKER).is_file());
        assert!(!case.old_app().exists());
        assert!(!case.dir.path().join("Apps/My Game.app.update-new").exists());
        assert!(!case.staging.exists());
        assert_eq!(
            recover_interrupted(&case.install).unwrap(),
            Recovery::NotInstalled
        );
        // Told once.
        assert_eq!(
            recover_interrupted(&case.install).unwrap(),
            Recovery::Nothing
        );

        // The player plays on, then the next update goes in as usual.
        fs::write(programs.join("saves/progress.json"), b"played since").unwrap();
        let again = MacCase::new();
        let staging = crate::install::make_staging_dir_in(case.dir.path(), "my-game").unwrap();
        let new = staging.join("unpacked/g/My Game.app");
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        crate::install::copy_tree(&again.new, &new).unwrap();
        let next = MacCase {
            staging,
            new,
            ..case
        };
        assert!(next.run(None));
        assert_eq!(fs::read(next.programs.join("my-game")).unwrap(), b"new");
        assert_eq!(
            fs::read(next.programs.join("saves/progress.json")).unwrap(),
            b"played since"
        );
        assert_eq!(
            fs::read(next.programs.join("Music/Lake.ogg")).unwrap(),
            b"song"
        );
    }

    /// An old app an earlier swap left holding the player's saves is never
    /// removed by a later update: the update is not installed, and each
    /// start tells the player where that app is until it is gone.
    #[cfg(unix)]
    #[test]
    fn the_mac_script_never_removes_an_old_app_holding_saves() {
        let case = MacCase::new();
        let old = case.old_app().join("Contents/MacOS");
        fs::create_dir_all(old.join("saves")).unwrap();
        fs::write(old.join("saves/progress.json"), b"first saves").unwrap();
        fs::write(old.join("my-game"), b"older").unwrap();
        assert_eq!(
            recover_interrupted(&case.install).unwrap(),
            Recovery::OldAppLeft(case.old_app())
        );
        assert!(!case.run(None));
        assert_eq!(
            fs::read(old.join("saves/progress.json")).unwrap(),
            b"first saves"
        );
        assert_eq!(fs::read(case.programs.join("my-game")).unwrap(), b"old");
        assert!(!case.staging.exists());
        assert_eq!(
            recover_interrupted(&case.install).unwrap(),
            Recovery::OldAppLeft(case.old_app())
        );
        // A portable app's other files count too; files the app in use
        // also has do not.
        fs::remove_dir_all(old.join("saves")).unwrap();
        assert_eq!(
            recover_interrupted(&case.install).unwrap(),
            Recovery::Nothing
        );
        fs::write(old.join("Lake.ogg"), b"song").unwrap();
        assert_eq!(
            recover_interrupted(&case.install).unwrap(),
            Recovery::OldAppLeft(case.old_app())
        );
        // Once it holds nothing of the player's, an update clears it.
        fs::remove_file(old.join("Lake.ogg")).unwrap();
        let staging = crate::install::make_staging_dir_in(case.dir.path(), "my-game").unwrap();
        let fresh = MacCase::new();
        let new = staging.join("unpacked/g/My Game.app");
        fs::create_dir_all(new.parent().unwrap()).unwrap();
        crate::install::copy_tree(&fresh.new, &new).unwrap();
        let next = MacCase {
            staging,
            new,
            ..case
        };
        assert!(next.run(None));
        assert_eq!(fs::read(next.programs.join("my-game")).unwrap(), b"new");
        assert!(!next.old_app().exists());
    }

    /// Both kinds of folder install, updated by the real POSIX script from
    /// one archive that is portable: each keeps its kind, and its saves
    /// stay where it reads them.
    #[cfg(unix)]
    #[test]
    fn one_archive_updates_each_kind_to_its_own_kind() {
        use crate::extract::tests::zip_file;
        use crate::install::PORTABLE_MARKER;
        for portable in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let game = dir.path().join("game");
            fs::create_dir_all(game.join("saves")).unwrap();
            fs::write(game.join("saves/progress.json"), b"mine").unwrap();
            fs::write(game.join("my-game"), b"#!/bin/sh\n").unwrap();
            if portable {
                fs::write(game.join(PORTABLE_MARKER), b"mine").unwrap();
            }
            let archive = dir.path().join("my-game-1.1.0-linux-x64.zip");
            zip_file(
                &archive,
                &[
                    ("my-game-1.1.0-linux-x64/my-game", b"#!/bin/sh\n# new\n"),
                    ("my-game-1.1.0-linux-x64/portable.txt", b"shipped"),
                ],
            );
            let install = Install::Folder {
                dir: game.clone(),
                exe: "my-game".into(),
            };
            let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
            let staged =
                crate::install::stage(&archive, &staging, &install, crate::Limits::default(), None)
                    .unwrap();
            let script =
                write_apply_script(&staged, &install, &staging, 999_999_999, Os::Linux).unwrap();
            assert!(Command::new("/bin/sh")
                .arg(&script)
                .status()
                .unwrap()
                .success());
            assert_eq!(
                fs::read(game.join("my-game")).unwrap(),
                b"#!/bin/sh\n# new\n"
            );
            assert_eq!(game.join(PORTABLE_MARKER).exists(), portable, "{portable}");
            assert_eq!(fs::read(game.join("saves/progress.json")).unwrap(), b"mine");
        }
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
        fs::create_dir(staging.join("g")).unwrap();
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
        let mut expected = vec![
            script,
            staging.join(JOURNAL),
            staging.join("backup"),
            staging.join("g"),
        ];
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
        fs::create_dir(staging.join("g")).unwrap();
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
        let log = fs::read_to_string(game.join(APPLY_LOG)).unwrap();
        assert!(log.contains("the game closed") && log.contains("the update is in"));
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
        let log = fs::read_to_string(game.join(APPLY_LOG)).unwrap();
        assert!(log.contains("copying clash failed"), "{log}");
        assert_eq!(fs::read(game.join("a.txt")).unwrap(), b"old");
        assert!(game.join("clash/inside").is_dir());
        // The journal and the backup stay for the next start, which puts
        // the folder back exactly.
        assert!(game.join(JOURNAL).is_file());
        assert!(staging.join("backup").is_dir());
        assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Restored);
        assert!(!game.join(JOURNAL).exists() && !staging.exists());
        assert!(!game.join(APPLY_LOG).exists());
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

    /// The backup holds only what the update replaces: a big file the
    /// update leaves in place is never copied, and the next start after a
    /// stopped apply removes only what the update added.
    #[cfg(unix)]
    #[test]
    fn only_replaced_files_are_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("saves")).unwrap();
        fs::create_dir_all(game.join("data")).unwrap();
        fs::write(game.join("sounds.pak"), b"big and unchanged").unwrap();
        fs::write(game.join("data/old.txt"), b"old").unwrap();
        fs::write(game.join("data/kept.txt"), b"kept").unwrap();
        fs::write(game.join("my-game"), b"old program").unwrap();
        let new = dir.path().join("new");
        fs::create_dir_all(new.join("data/fresh")).unwrap();
        fs::create_dir_all(new.join("saves")).unwrap();
        fs::write(new.join("data/old.txt"), b"new").unwrap();
        fs::write(new.join("data/fresh/a.txt"), b"new").unwrap();
        fs::write(new.join("added.txt"), b"new").unwrap();
        fs::write(new.join("my-game"), b"new program").unwrap();
        let backup = dir.path().join("backup");
        let mut added = back_up_replaced(&new, &game, &backup).unwrap();
        added.sort();
        assert_eq!(added, ["added.txt", "data/fresh"]);
        let mut backed: Vec<_> = walk(&backup);
        backed.sort();
        assert_eq!(backed, ["data", "data/old.txt", "my-game"]);
        assert_eq!(fs::read(backup.join("data/old.txt")).unwrap(), b"old");

        // A journal naming paths outside the folder, or its saves, is not
        // followed.
        for bad in ["../outside.txt", "saves/progress.json", "/etc/passwd"] {
            let text = format!("{JOURNAL_HEADER_2}\n/tmp/x-update-1/backup\n{bad}\n");
            assert!(journal_backup(&text).is_none(), "{bad}");
        }
    }

    #[cfg(unix)]
    fn walk(root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(at) = stack.pop() {
            for entry in fs::read_dir(&at).unwrap() {
                let path = entry.unwrap().path();
                out.push(path.strip_prefix(root).unwrap().display().to_string());
                if path.is_dir() {
                    stack.push(path);
                }
            }
        }
        out
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
