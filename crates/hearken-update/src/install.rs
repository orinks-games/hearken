//! Where the running game is installed, and the unpacked update that
//! replaces it.
//!
//! Three kinds of install, each replaced its own way:
//!
//! - a folder holding the program (Windows, Linux): the update's files are
//!   copied over it, leaving its `saves` folder alone;
//! - a macOS app bundle: the whole bundle is swapped;
//! - an AppImage: the one file is swapped. The folder it runs from is a
//!   read-only mount, never touched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::extract::{self, Limits};
use crate::manifest::{archive_kind, ArchiveKind};

/// The operating system, as the updater branches on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

/// The process facts the updater reads, gathered once so tests can fake
/// any of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub os: Os,
    /// The running program.
    pub exe: PathBuf,
    /// The AppImage file this process runs from (`APPIMAGE`), when it is one.
    pub appimage: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub pid: u32,
}

impl Host {
    pub fn current() -> Host {
        Host {
            os: Os::current(),
            exe: std::env::current_exe().unwrap_or_default(),
            appimage: std::env::var_os("APPIMAGE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            home: if cfg!(windows) {
                std::env::var_os("USERPROFILE").map(PathBuf::from)
            } else {
                std::env::var_os("HOME").map(PathBuf::from)
            },
            pid: std::process::id(),
        }
    }

    /// A host for tests: `os` and `exe`, no AppImage, no home, pid 4242.
    pub fn fake(os: Os, exe: &Path) -> Host {
        Host {
            os,
            exe: exe.to_path_buf(),
            appimage: None,
            home: None,
            pid: 4242,
        }
    }

    /// The program's path, links resolved, in a form the apply script's
    /// tools accept.
    fn real_exe(&self) -> PathBuf {
        strip_verbatim(fs::canonicalize(&self.exe).unwrap_or_else(|_| self.exe.clone()))
    }

    /// What an update replaces.
    pub fn install(&self) -> Install {
        if let Some(file) = self.appimage.as_ref().filter(|f| f.is_file()) {
            return Install::AppImage { file: file.clone() };
        }
        let exe = self.real_exe();
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if self.os == Os::MacOs {
            for app in dir.ancestors() {
                if app.extension().is_some_and(|e| e == "app") {
                    let inside = exe.strip_prefix(app).unwrap_or(&exe).to_path_buf();
                    return Install::Bundle {
                        app: app.to_path_buf(),
                        exe: inside,
                    };
                }
            }
        }
        Install::Folder { dir, exe: name }
    }
}

/// Undoes `fs::canonicalize`'s Windows verbatim prefix (`\\?\C:\...`).
///
/// robocopy refuses the `\\?\` form outright: with it, every copy in the
/// apply script fails, the old build starts again, and the same update is
/// offered forever. The drive form maps back to `C:\...`, a share to
/// `\\server\share`.
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    path
}

/// The running game's install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// The folder holding the program, and the program's file name.
    Folder {
        dir: PathBuf,
        exe: String,
    },
    /// A macOS app, and the program's path inside it
    /// (`Contents/MacOS/my-game`).
    Bundle {
        app: PathBuf,
        exe: PathBuf,
    },
    AppImage {
        file: PathBuf,
    },
}

impl Install {
    /// The folder, bundle or file the update replaces.
    pub fn target(&self) -> &Path {
        match self {
            Install::Folder { dir, .. } => dir,
            Install::Bundle { app, .. } => app,
            Install::AppImage { file } => file,
        }
    }
}

/// An update ready to swap in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Staged {
    /// The new install folder's files.
    Folder(PathBuf),
    /// The new app bundle.
    Bundle(PathBuf),
    /// The new AppImage file.
    AppImage(PathBuf),
}

impl Staged {
    pub fn path(&self) -> &Path {
        match self {
            Staged::Folder(p) | Staged::Bundle(p) | Staged::AppImage(p) => p,
        }
    }
}

/// Whether the program at `exe` is a Cargo build in a `target` folder, not
/// an install: such a program never replaces itself, even one built by
/// `hearken release`, or an update would land in the build folder.
pub fn in_cargo_target(exe: &Path) -> bool {
    exe.ancestors()
        .skip(1)
        .take(4)
        .any(|dir| dir.join("CACHEDIR.TAG").is_file() && dir.join(".rustc_info.json").is_file())
}

/// A new, empty folder under the system's temporary folder.
pub fn make_staging_dir(prefix: &str) -> io::Result<PathBuf> {
    let base = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..100u32 {
        let dir = base.join(format!(
            "{prefix}-update-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("could not make a folder for the update"))
}

fn missing(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, what)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Unpacks the downloaded `archive` into `staging` and finds what replaces
/// `install`. The archive is deleted once unpacked. The update must hold the
/// running program under the same name, or it is refused: an archive
/// without it could only leave the player with no game.
pub fn stage(
    archive: &Path,
    staging: &Path,
    install: &Install,
    limits: Limits,
    cancelled: Option<&AtomicBool>,
) -> io::Result<Staged> {
    if archive_kind(&file_name(archive)) == Some(ArchiveKind::AppImage) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(archive, fs::Permissions::from_mode(0o755))?;
        }
        return Ok(Staged::AppImage(archive.to_path_buf()));
    }
    let unpacked = staging.join("unpacked");
    extract::extract(archive, &unpacked, limits, cancelled)?;
    match fs::remove_file(archive) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let root = extract::unpacked_root(&unpacked)?;
    match install {
        Install::Bundle { app, exe } => {
            let bundle = find_bundle(&root, &file_name(app))?;
            if !bundle.join(exe).is_file() {
                return Err(missing(format!(
                    "the update's app has no {}",
                    exe.display()
                )));
            }
            Ok(Staged::Bundle(bundle))
        }
        Install::Folder { exe, .. } => {
            if !root.join(exe).is_file() {
                return Err(missing(format!("the update has no {exe}")));
            }
            Ok(Staged::Folder(root))
        }
        // A folder update for an AppImage: kept for a manual install.
        Install::AppImage { .. } => Ok(Staged::Folder(root)),
    }
}

/// The app bundle in `root`: the one named `name`, else the only one.
fn find_bundle(root: &Path, name: &str) -> io::Result<PathBuf> {
    if root.extension().is_some_and(|e| e == "app") {
        return Ok(root.to_path_buf());
    }
    let same = root.join(name);
    if same.is_dir() {
        return Ok(same);
    }
    let apps: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.extension().is_some_and(|e| e == "app"))
        .collect();
    match apps.as_slice() {
        [one] => Ok(one.clone()),
        _ => Err(missing("the update holds no single app".to_string())),
    }
}

fn dir_writable(path: &Path) -> bool {
    let probe = path.join(format!(".hearken-update-probe-{}", std::process::id()));
    if fs::write(&probe, b"").is_err() {
        return false;
    }
    fs::remove_file(&probe).is_ok()
}

/// True for a bundle macOS runs from an App Translocation mount: an app
/// opened straight out of a quarantined download runs from a read-only copy
/// that no swap can touch, and that is gone once the game quits.
pub fn is_translocated(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new("AppTranslocation"))
}

/// Whether the apply script can install `staged` over `install` by itself.
/// False means the player installs it by hand: a folder no one can write
/// to, an app running translocated, or an update of another kind than the
/// install (a folder for an AppImage).
pub fn can_auto_apply(staged: &Staged, install: &Install) -> bool {
    match (staged, install) {
        (Staged::Folder(_), Install::Folder { dir, .. }) => dir_writable(dir),
        (Staged::Bundle(_), Install::Bundle { app, .. }) => {
            !is_translocated(app) && app.parent().is_some_and(dir_writable)
        }
        (Staged::AppImage(new), Install::AppImage { file }) => {
            new.is_file() && file.parent().is_some_and(dir_writable)
        }
        _ => false,
    }
}

/// Moves an update the player must install by hand somewhere they can find
/// it again: an AppImage or an app bundle to the home folder, since the
/// staging folder is under the system's temporary folder. A folder update
/// stays where it was unpacked. Returns where it is.
pub fn stash_for_manual_install(staged: &Staged, home: Option<&Path>) -> PathBuf {
    let from = staged.path();
    let Some(home) = home.filter(|_| !matches!(staged, Staged::Folder(_))) else {
        return from.to_path_buf();
    };
    let dest = home.join(file_name(from));
    // Never replace what is already there: it may be the player's own copy.
    if fs::symlink_metadata(&dest).is_ok() {
        return from.to_path_buf();
    }
    let moved = fs::rename(from, &dest).or_else(|_| match staged {
        Staged::AppImage(_) => fs::copy(from, &dest).and_then(|_| fs::remove_file(from)),
        _ => Err(io::Error::other("cannot move a bundle across disks")),
    });
    match moved {
        Ok(()) => dest,
        Err(_) => from.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::tests::zip_file;

    #[test]
    fn verbatim_prefixes_are_stripped_for_the_apply_script() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\Games\MyGame\my-game.exe")),
            PathBuf::from(r"C:\Games\MyGame\my-game.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\server\share\my-game.exe")),
            PathBuf::from(r"\\server\share\my-game.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"C:\Games\my-game.exe")),
            PathBuf::from(r"C:\Games\my-game.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from("/home/user/my-game")),
            PathBuf::from("/home/user/my-game")
        );
    }

    #[test]
    fn installs_are_found_from_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let exe = root.join("my-game");
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            Host::fake(Os::Linux, &exe).install(),
            Install::Folder {
                dir: root.clone(),
                exe: "my-game".into()
            }
        );
        let app = root.join("My Game.app");
        let inner = app.join("Contents/MacOS/my-game");
        fs::create_dir_all(inner.parent().unwrap()).unwrap();
        fs::write(&inner, b"").unwrap();
        assert_eq!(
            Host::fake(Os::MacOs, &inner).install(),
            Install::Bundle {
                app: app.clone(),
                exe: PathBuf::from("Contents/MacOS/my-game")
            }
        );
        // Not on a Mac, an .app folder is just a folder.
        assert!(matches!(
            Host::fake(Os::Linux, &inner).install(),
            Install::Folder { .. }
        ));
        let image = root.join("My_Game-x86_64.AppImage");
        fs::write(&image, b"").unwrap();
        let mut host = Host::fake(Os::Linux, Path::new("/tmp/.mount_x/usr/bin/my-game"));
        host.appimage = Some(image.clone());
        assert_eq!(host.install(), Install::AppImage { file: image });
        // An APPIMAGE that names no file is not an AppImage run.
        host.appimage = Some(root.join("gone.AppImage"));
        assert!(matches!(host.install(), Install::Folder { .. }));
    }

    #[test]
    fn cargo_builds_are_not_installs() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir_all(target.join("release")).unwrap();
        fs::write(target.join("CACHEDIR.TAG"), b"").unwrap();
        fs::write(target.join(".rustc_info.json"), b"{}").unwrap();
        assert!(in_cargo_target(&target.join("release/my-game")));
        assert!(in_cargo_target(&target.join("debug/deps/my_game-abc")));
        assert!(!in_cargo_target(&dir.path().join("MyGame/my-game")));
    }

    #[test]
    fn an_update_without_the_program_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("my-game-1.1.0-windows-x64.zip");
        zip_file(&archive, &[("my-game-1.1.0-windows-x64/readme.txt", b"hi")]);
        let install = Install::Folder {
            dir: dir.path().join("game"),
            exe: "my-game.exe".into(),
        };
        let staging = dir.path().join("staging");
        let err = stage(&archive, &staging, &install, Limits::default(), None).unwrap_err();
        assert!(err.to_string().contains("my-game.exe"), "{err}");
    }

    #[test]
    fn a_folder_update_stages_its_root() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("my-game-1.1.0-windows-x64.zip");
        zip_file(
            &archive,
            &[
                ("my-game-1.1.0-windows-x64/my-game.exe", b"MZ"),
                ("my-game-1.1.0-windows-x64/SDL2.dll", b"MZ"),
            ],
        );
        let game = dir.path().join("game");
        fs::create_dir_all(&game).unwrap();
        let install = Install::Folder {
            dir: game.clone(),
            exe: "my-game.exe".into(),
        };
        let staging = dir.path().join("staging");
        let staged = stage(&archive, &staging, &install, Limits::default(), None).unwrap();
        assert_eq!(
            staged,
            Staged::Folder(staging.join("unpacked/my-game-1.1.0-windows-x64"))
        );
        assert!(!archive.exists());
        assert!(can_auto_apply(&staged, &install));
        // A folder update never goes over an AppImage.
        let image = Install::AppImage {
            file: game.join("x.AppImage"),
        };
        assert!(!can_auto_apply(&staged, &image));
    }

    #[test]
    fn a_bundle_update_finds_its_app() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("my-game-1.1.0-macos-arm64.zip");
        zip_file(
            &archive,
            &[
                (
                    "my-game-1.1.0-macos-arm64/My Game.app/Contents/MacOS/my-game",
                    b"bin",
                ),
                ("my-game-1.1.0-macos-arm64/README.md", b"hi"),
            ],
        );
        let apps = dir.path().join("Applications");
        fs::create_dir_all(&apps).unwrap();
        let install = Install::Bundle {
            app: apps.join("My Game.app"),
            exe: PathBuf::from("Contents/MacOS/my-game"),
        };
        let staging = dir.path().join("staging");
        let staged = stage(&archive, &staging, &install, Limits::default(), None).unwrap();
        assert_eq!(
            staged,
            Staged::Bundle(staging.join("unpacked/my-game-1.1.0-macos-arm64/My Game.app"))
        );
        assert!(can_auto_apply(&staged, &install));
        let translocated = Install::Bundle {
            app: PathBuf::from("/private/var/folders/x/AppTranslocation/ABC/d/My Game.app"),
            exe: PathBuf::from("Contents/MacOS/my-game"),
        };
        assert!(!can_auto_apply(&staged, &translocated));
    }

    #[test]
    fn an_appimage_is_its_own_update() {
        let dir = tempfile::tempdir().unwrap();
        let download = dir.path().join("My_Game-1.1.0-x86_64.AppImage");
        fs::write(&download, b"ELF").unwrap();
        let running = dir.path().join("My_Game.AppImage");
        fs::write(&running, b"ELF").unwrap();
        let install = Install::AppImage {
            file: running.clone(),
        };
        let staged = stage(&download, dir.path(), &install, Limits::default(), None).unwrap();
        assert_eq!(staged, Staged::AppImage(download.clone()));
        assert!(can_auto_apply(&staged, &install));
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let parked = stash_for_manual_install(&staged, Some(&home));
        assert_eq!(parked, home.join("My_Game-1.1.0-x86_64.AppImage"));
        assert!(parked.is_file() && !download.exists());
    }

    #[test]
    fn stashing_never_replaces_the_players_copy() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let app = dir.path().join("staging/My Game.app");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(home.join("My Game.app")).unwrap();
        let staged = Staged::Bundle(app.clone());
        assert_eq!(stash_for_manual_install(&staged, Some(&home)), app);
        let folder = Staged::Folder(dir.path().join("staging"));
        assert_eq!(
            stash_for_manual_install(&folder, Some(&home)),
            dir.path().join("staging")
        );
    }
}
