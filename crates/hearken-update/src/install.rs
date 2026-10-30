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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// Staging folders older than this are left from an update that never
/// finished (the game was killed, or the player installed it by hand), and
/// are removed when the next one is made.
pub const STALE_STAGING: Duration = Duration::from_secs(24 * 60 * 60);

/// A staging folder whose update was being copied in holds the backup an
/// unfinished copy is put back from (see [`crate::apply::recover_interrupted`]),
/// so it is kept this long instead: the player may not start the game again
/// for a while.
pub const STALE_BACKUP: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A new, empty folder under the system's temporary folder, that only this
/// user can open. On Linux that folder is `/tmp`, shared by every user: the
/// folder's name is random, it is made with `create_dir` (which never
/// follows or reuses what is there), and its mode is 0700, so no one else
/// can read the update or plant anything in it. Old staging folders of the
/// same `prefix` are removed on the way (see [`STALE_STAGING`]).
pub fn make_staging_dir(prefix: &str) -> io::Result<PathBuf> {
    let base = std::env::temp_dir();
    let dir = make_staging_dir_in(&base, prefix)?;
    sweep_stale_staging(&base, prefix, &dir, STALE_STAGING);
    Ok(dir)
}

fn staging_name_start(prefix: &str) -> String {
    format!("{prefix}-update-")
}

/// Sixteen random hex digits, for a name no one else can guess.
pub(crate) fn random_suffix() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // Still unique; only harder to guess with the random source.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bytes = (nanos as u64).to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// [`make_staging_dir`] under `base`, without the sweep.
pub fn make_staging_dir_in(base: &Path, prefix: &str) -> io::Result<PathBuf> {
    for _ in 0..100u32 {
        let dir = base.join(format!(
            "{}{}-{}",
            staging_name_start(prefix),
            std::process::id(),
            random_suffix()
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("could not make a folder for the update"))
}

/// Removes staging folders of `prefix` under `base` not changed for
/// `older_than`, except `keep`. Only real folders are removed, never a
/// link, and on Unix only ones owned by the same user as `keep`, since
/// `/tmp` holds other users' files too. Failures are ignored.
pub fn sweep_stale_staging(base: &Path, prefix: &str, keep: &Path, older_than: Duration) {
    let start = staging_name_start(prefix);
    let Ok(mine) = fs::symlink_metadata(keep) else {
        return;
    };
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path == keep || !entry.file_name().to_string_lossy().starts_with(&start) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.uid() != mine.uid() {
                continue;
            }
        }
        let _ = &mine;
        let older_than = if path.join("backup").is_dir() {
            older_than.max(STALE_BACKUP)
        } else {
            older_than
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age >= older_than);
        if old {
            log::info!("removing an old update folder, {}", path.display());
            let _ = fs::remove_dir_all(&path);
        }
    }
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
/// without it could only leave the player with no game. However loose
/// `limits` is, the archive may unpack to no more than
/// [`Limits::for_archive`] of its size.
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
    let limits = limits.at_most(Limits::for_archive(fs::metadata(archive)?.len()));
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

/// Moves an update the player must install by hand to their home folder,
/// where they can find it again, and removes the rest of `staging` (the
/// system's temporary folder is no place to keep it). Returns where the
/// update is.
///
/// It stays in `staging` when there is no home folder, when something of
/// its name is already there (it may be the player's own copy), when it
/// cannot be moved, or when it has no name of its own (an archive without
/// one top folder). [`make_staging_dir`] removes such a folder a day later.
pub fn stash_for_manual_install(staged: &Staged, staging: &Path, home: Option<&Path>) -> PathBuf {
    let from = staged.path();
    let unnamed = from == staging.join("unpacked") || from == staging;
    let dest = match home {
        Some(home) if !unnamed => home.join(file_name(from)),
        _ => return from.to_path_buf(),
    };
    if fs::symlink_metadata(&dest).is_ok() {
        return from.to_path_buf();
    }
    let moved = fs::rename(from, &dest).or_else(|_| {
        // Another disk: copy, and take back a half-made copy.
        let copied = copy_tree(from, &dest);
        if copied.is_err() {
            let _ = remove_any(&dest);
        }
        copied
    });
    match moved {
        Ok(()) => {
            if !dest.starts_with(staging) {
                let _ = fs::remove_dir_all(staging);
            }
            dest
        }
        Err(e) => {
            log::warn!("could not move the update to {}: {e}", dest.display());
            from.to_path_buf()
        }
    }
}

fn remove_any(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Copies a file, or a folder and everything in it, to `to`, which must not
/// exist. Links are copied as links, never followed.
pub(crate) fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(fs::read_link(from)?, to);
        #[cfg(not(unix))]
        return Err(io::Error::other("cannot copy a link"));
    }
    if meta.is_dir() {
        fs::create_dir(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        fs::set_permissions(to, meta.permissions())
    } else {
        fs::copy(from, to).map(|_| ())
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
        let parked = stash_for_manual_install(&staged, dir.path(), Some(&home));
        assert_eq!(parked, home.join("My_Game-1.1.0-x86_64.AppImage"));
        assert!(parked.is_file() && !download.exists());
    }

    #[test]
    fn stashing_never_replaces_the_players_copy() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let staging = dir.path().join("staging");
        let app = staging.join("unpacked/g/My Game.app");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(home.join("My Game.app")).unwrap();
        let staged = Staged::Bundle(app.clone());
        assert_eq!(
            stash_for_manual_install(&staged, &staging, Some(&home)),
            app
        );
        assert!(app.is_dir());
        // An update with no name of its own stays where it is.
        let folder = Staged::Folder(staging.join("unpacked"));
        assert_eq!(
            stash_for_manual_install(&folder, &staging, Some(&home)),
            staging.join("unpacked")
        );
        assert!(staging.is_dir());
    }

    /// A folder update the player installs by hand goes to their home
    /// folder, and the staging folder goes: nothing is left in the
    /// temporary folder.
    #[test]
    fn a_stashed_folder_update_leaves_no_staging_behind() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let staging = dir.path().join("staging");
        let root = staging.join("unpacked/my-game-1.1.0-linux-x64");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(root.join("my-game"), b"ELF").unwrap();
        fs::write(root.join("data/a.txt"), b"a").unwrap();
        let dest = stash_for_manual_install(&Staged::Folder(root.clone()), &staging, Some(&home));
        assert_eq!(dest, home.join("my-game-1.1.0-linux-x64"));
        assert_eq!(fs::read(dest.join("data/a.txt")).unwrap(), b"a");
        assert!(!staging.exists());
    }

    #[test]
    fn a_tree_copies_across_with_its_links() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        fs::create_dir_all(from.join("sub")).unwrap();
        fs::write(from.join("sub/f"), b"f").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("sub/f", from.join("link")).unwrap();
        let to = dir.path().join("to");
        copy_tree(&from, &to).unwrap();
        assert_eq!(fs::read(to.join("sub/f")).unwrap(), b"f");
        #[cfg(unix)]
        assert_eq!(
            fs::read_link(to.join("link")).unwrap(),
            PathBuf::from("sub/f")
        );
    }

    /// On Linux the staging folder is in `/tmp`, which every user shares:
    /// it must be new, unguessable, and closed to everyone else.
    #[test]
    fn staging_folders_are_private_and_unguessable() {
        let dir = tempfile::tempdir().unwrap();
        let a = make_staging_dir_in(dir.path(), "my-game").unwrap();
        let b = make_staging_dir_in(dir.path(), "my-game").unwrap();
        assert_ne!(a, b);
        let name = file_name(&a);
        assert!(name.starts_with("my-game-update-"), "{name}");
        let random = name.rsplit('-').next().unwrap();
        assert_eq!(random.len(), 16, "{name}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&a).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{mode:o}");
        }
    }

    #[test]
    fn old_staging_folders_are_swept_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let keep = make_staging_dir_in(base, "my-game").unwrap();
        let old = make_staging_dir_in(base, "my-game").unwrap();
        fs::write(old.join("archive.zip"), b"zip").unwrap();
        let other_game = make_staging_dir_in(base, "other-game").unwrap();
        let file = base.join("my-game-update-file");
        fs::write(&file, b"not a folder").unwrap();
        let outside = base.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("precious"), b"mine").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, base.join("my-game-update-link")).unwrap();
        // Nothing is old enough yet.
        sweep_stale_staging(base, "my-game", &keep, STALE_STAGING);
        assert!(old.is_dir());
        // One holding a backup an unfinished update may need is kept longer.
        let with_backup = make_staging_dir_in(base, "my-game").unwrap();
        fs::create_dir(with_backup.join("backup")).unwrap();
        sweep_stale_staging(base, "my-game", &keep, Duration::ZERO);
        assert!(!old.exists());
        assert!(with_backup.join("backup").is_dir());
        assert!(keep.is_dir() && other_game.is_dir() && file.is_file());
        assert_eq!(fs::read(outside.join("precious")).unwrap(), b"mine");
    }

    /// A small archive that unpacks huge is stopped at a multiple of its
    /// size, not at the default limit, so it cannot fill the disk.
    #[test]
    fn a_compression_bomb_is_stopped_near_its_own_size() {
        use crate::extract::tests::{file_header, tar_gz};
        use crate::extract::MIN_UNPACK_BYTES;
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("my-game-1.1.0-linux-x64.tar.gz");
        let size = MIN_UNPACK_BYTES + 1024 * 1024;
        tar_gz(&archive, |b| {
            b.append_data(
                &mut file_header(size, 0o644),
                "my-game-1.1.0-linux-x64/zeros",
                io::Read::take(io::repeat(0), size),
            )
            .unwrap();
        });
        assert!(fs::metadata(&archive).unwrap().len() * 20 < MIN_UNPACK_BYTES);
        let install = Install::Folder {
            dir: dir.path().join("game"),
            exe: "my-game".into(),
        };
        let staging = dir.path().join("staging");
        let err = stage(&archive, &staging, &install, Limits::default(), None).unwrap_err();
        assert!(err.to_string().contains("more than allowed"), "{err}");
    }
}
