//! Swapping the update in: a small script, started detached, that waits
//! for the game to exit, replaces the install and starts the game again.
//!
//! A failure part-way must never leave the player without a game:
//!
//! - folder installs are copied to a backup first, and the backup is copied
//!   back if copying the update over fails. Without a backup, nothing is
//!   touched. The `saves` folder is never copied either way;
//! - an app bundle is moved aside (`.old`) until the new one is in place,
//!   and moved back if it is not;
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

const WINDOWS_FOLDER: &str = r#"@echo off
rem Waits for the game to close, keeps a copy of its folder, copies the
rem update over it, and puts the copy back if that fails. saves is never
rem touched.
set tries=0
:wait
tasklist /FI "PID eq {pid}" 2>NUL | find "{pid}" >NUL
if errorlevel 1 goto closed
set /a tries+=1
if %tries% GEQ 300 goto done
ping -n 2 127.0.0.1 >NUL
goto wait
:closed
robocopy "{dst}" "{backup}" /E /XD saves /R:3 /W:1 >NUL
if errorlevel 8 goto start
robocopy "{src}" "{dst}" /E /XD saves /R:10 /W:1 >NUL
if errorlevel 8 robocopy "{backup}" "{dst}" /E /XD saves /R:10 /W:1 >NUL
:start
start "" /D "{dst}" "{dst}\{exe}"
:done
rem The script is in the staging folder: one line, so cmd never reads on.
rmdir /s /q "{staging}" & exit /b
"#;

const POSIX_FOLDER: &str = r#"#!/bin/sh
# Waits for the game to close, keeps a copy of its folder, copies the
# update over it, and puts the copy back if that fails. saves is never
# touched.
tries=0
while kill -0 {pid} 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -ge 300 ]; then rm -rf "{staging}"; exit 1; fi
  sleep 1
done
rm -rf "{src}/saves"
backup="{backup}"
ok=1
mkdir -p "$backup" || ok=0
if [ "$ok" = 1 ]; then
  for entry in "{dst}"/* "{dst}"/.[!.]* "{dst}"/..?*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    [ "${entry##*/}" = saves ] && continue
    cp -a "$entry" "$backup/" || { ok=0; break; }
  done
fi
if [ "$ok" = 1 ]; then
  cp -a "{src}/." "{dst}/" || cp -a "$backup/." "{dst}/"
fi
cd "{dst}" && "./{exe}" >/dev/null 2>&1 &
rm -rf "{staging}"
"#;

const MACOS_BUNDLE: &str = r#"#!/bin/sh
# Swaps the whole app. Saves live in Application Support, never inside it.
# The old app is moved aside until the new one is in place, so a failed
# copy cannot leave the player with no game.
tries=0
while kill -0 {pid} 2>/dev/null; do
  tries=$((tries + 1))
  if [ "$tries" -ge 300 ]; then rm -rf "{staging}"; exit 1; fi
  sleep 1
done
if [ -e "{dst}" ]; then
  rm -rf "{dst}.old"
  if mv "{dst}" "{dst}.old"; then
    if mv "{src}" "{dst}" 2>/dev/null || ditto "{src}" "{dst}"; then
      rm -rf "{dst}.old"
    else
      rm -rf "{dst}"
      mv "{dst}.old" "{dst}"
    fi
  fi
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
    // Joined by hand, so a test on one system can write another's script.
    let backup = if windows {
        PathBuf::from(format!("{}\\backup", staging.display()))
    } else {
        staging.join("backup")
    };
    Ok(fill(
        template,
        &[
            ("pid", pid.to_string()),
            ("src", q(staged.path())),
            ("dst", q(install.target())),
            ("staging", q(staging)),
            ("backup", q(&backup)),
            ("exe", quote_for_script(&exe, windows)),
        ],
    ))
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
    let script = staging.join(format!(
        "hearken-apply-{}.{suffix}",
        crate::install::random_suffix()
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700);
    }
    let mut file = options.open(&script)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(e) = written {
        drop(file);
        let _ = fs::remove_file(&script);
        return Err(e);
    }
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
        assert!(text.contains(r#"robocopy "C:\Games\100%% Fun" "C:\Temp\s\backup" /E /XD saves"#));
        assert!(
            text.contains(r#"robocopy "C:\Temp\s\unpacked\g" "C:\Games\100%% Fun" /E /XD saves"#)
        );
        assert!(
            text.contains(r#"if errorlevel 8 robocopy "C:\Temp\s\backup" "C:\Games\100%% Fun""#)
        );
        assert!(
            text.contains(r#"start "" /D "C:\Games\100%% Fun" "C:\Games\100%% Fun\my-game.exe""#)
        );
        assert!(!text.contains(r"\\?\"));
        // The script is inside the staging folder, so the line that
        // deletes it also ends it.
        assert!(text.contains(r#"rmdir /s /q "C:\Temp\s" & exit /b"#));
        // The backup comes first, and a failed backup skips the copy.
        let backup = text.find("\"C:\\Temp\\s\\backup\" /E").unwrap();
        let copy = text.find("robocopy \"C:\\Temp\\s\\unpacked").unwrap();
        assert!(backup < copy);
        assert!(text[backup..copy].contains("if errorlevel 8 goto start"));
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
            "cp -a \"/tmp/s/unpacked/g/.\" \"/home/p/My \\$Game/\" || cp -a \"$backup/.\" \"/home/p/My \\$Game/\""
        ));
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
        assert!(text.contains("mv \"/Applications/My Game.app\" \"/Applications/My Game.app.old\""));
        // A half-made copy is removed before the old app goes back.
        let undo = text.find("rm -rf \"/Applications/My Game.app\"\n").unwrap();
        let back = text
            .find("mv \"/Applications/My Game.app.old\" \"/Applications/My Game.app\"")
            .unwrap();
        assert!(undo < back);
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
        assert_eq!(fs::read_dir(&staging).unwrap().count(), 1);
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
    }
}
