//! `hearken release`: a player's zip of the game, without the agent server.
//! On a Mac the zip holds an app (see `macos_app`).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::macos_app::Layout;
use crate::project::Project;

/// The file that makes a copy portable: `hearken_app::paths::PORTABLE_MARKER`.
/// (This crate stays free of the engine, so the name is repeated here.)
const PORTABLE_MARKER: &str = "portable.txt";

/// The folder the engine reads sounds from, which `pack = true` ships as
/// `sounds.pak`.
const SOUNDS: &str = "sounds";

/// The variable `hearken-pack`'s build script reads the release's key from.
const PACK_KEY_VAR: &str = "HEARKEN_PACK_KEY";

/// What a portable copy's marker file says to whoever opens it.
const PORTABLE_NOTE: &str = "This copy of the game is portable: its settings, saves and log \
are kept in the saves folder beside it. Delete this file to keep them in your user folder \
instead.\n";

/// What `hearken release` was asked for beyond the game itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// The update channel this release is published on.
    pub channel: String,
    /// A Markdown file of release notes for the update manifest.
    pub notes: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            channel: hearken_update::DEFAULT_CHANNEL.to_string(),
            notes: None,
        }
    }
}

/// Builds the release, stages it under `dist/` and archives it. Returns the
/// archive's path.
pub fn release(project: &Project, options: &Options) -> Result<PathBuf, String> {
    if !hearken_update::manifest::is_channel(&options.channel) {
        return Err(format!(
            "The channel {:?} may hold only lowercase letters, digits and hyphens, like beta.",
            options.channel
        ));
    }
    let notes = match &options.notes {
        Some(path) => read_notes(path)?,
        None => String::new(),
    };
    if cfg!(target_os = "macos") {
        crate::macos_sign::check()?;
    }
    // A packed release gets a key of its own, built into its program. Any
    // other build is made with none, so it reads loose sounds.
    if project.pack {
        engine_reads_packs(&project.dir)?;
    }
    let key = if project.pack {
        Some(hearken_pack::new_key().map_err(|e| format!("Could not make a pack key: {e}"))?)
    } else {
        None
    };
    let hex = key.as_ref().map(hearken_pack::key_hex).unwrap_or_default();
    // Always passed, empty when the game does not seal, so a key in the
    // developer's shell never reaches a release by accident.
    let seal = crate::seal_key::release_key(project)?.map(|k| k.to_hex());
    // The public half goes into the game; the secret signs the manifest.
    let update = crate::update_key::release_key(project)?;
    let identity = match &update {
        Some(key) => UpdateIdentity {
            key: key.public().to_string(),
            game: project.package.clone(),
            version: project.version.clone(),
            channel: options.channel.clone(),
        },
        None => UpdateIdentity::default(),
    };
    // The rules module first: its ID goes into the game.
    let rules = match &project.rules {
        Some(rules) => Some(crate::rules_module::build(project, rules)?),
        None => None,
    };
    let rules_id = rules.as_ref().map(|m| m.id.clone()).unwrap_or_default();
    project.build_with(
        false,
        &key_env(&hex, &seal.unwrap_or_default(), &rules_id, &identity),
    )?;
    // The platform part of the name, the one `hearken play --download`
    // looks for and the update manifest names.
    let platform = hearken_update::platform::current();
    let stem = format!("{}-{}-{platform}", project.package, project.version);
    let dist = project.dir.join("dist");
    // An earlier release's module is not this one's.
    let stale = format!("{}-{}-rules-", project.package, project.version);
    for entry in std::fs::read_dir(&dist).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&stale) && name.ends_with(".wasm") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    if let Some(module) = &rules {
        // Beside the archive, not in it: players never need it, and the
        // server is given it to publish the build.
        std::fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
        let name = rules_file_name(&project.package, &project.version, &module.id);
        copy(&module.path, &dist.join(&name))?;
        println!(
            "Rules module: dist/{name}\nRules ID: {}\nPublish this module on the game's server, or \
             runs from this build are refused.",
            module.id
        );
    }
    let stage = dist.join(&stem);
    if stage.exists() {
        std::fs::remove_dir_all(&stage)
            .map_err(|e| format!("Could not clear {}: {e}", stage.display()))?;
    }
    // On a Mac the release is an app, so a player opens it from Finder
    // rather than a Terminal window.
    let layout = Layout::new(&stage, project, cfg!(target_os = "macos"));
    layout.create()?;

    let exe = project.exe();
    copy(
        &exe,
        &layout.programs.join(exe.file_name().unwrap_or_default()),
    )?;
    let mut packed = false;
    for item in &project.include {
        let from = project.dir.join(item);
        if let (Some(key), SOUNDS) = (&key, item.as_str()) {
            if from.is_dir() {
                pack(&from, &layout.resources, key)?;
                packed = true;
                continue;
            }
        }
        if from.exists() {
            copy(&from, &layout.place(item).join(item))?;
        } else {
            println!("Skipping {item}: it is not in the workspace.");
        }
    }
    if let (Some(key), false) = (&key, packed) {
        // A packed build reads only its pack, so it still gets one.
        println!("Nothing to pack: the release includes no {SOUNDS} folder.");
        pack(&stage.join(".nothing-to-pack"), &layout.resources, key)?;
    }
    if cfg!(windows) {
        windows_runtime(&project.release_dir(), &stage)?;
    }
    if project.portable {
        if layout.app.is_some() {
            // An app is replaced whole on update, and macOS may run it from
            // a read-only copy, so its saves go in Application Support.
            println!("The Mac app keeps its saves in Application Support: `portable` is for the other platforms.");
        } else {
            std::fs::write(stage.join(PORTABLE_MARKER), PORTABLE_NOTE)
                .map_err(|e| e.to_string())?;
        }
    }
    if let Some(app) = &layout.app {
        crate::macos_app::finish(project, app)?;
    }
    let signed = match &layout.app {
        Some(app) => crate::macos_sign::sign(&stage, app)?,
        None => false,
    };

    let archive = make_archive(&dist, &stem)?;
    if let (true, Some(app)) = (signed, &layout.app) {
        if crate::macos_sign::notarize(&archive)? {
            // Apple's ticket goes into the app, so it opens offline too;
            // the zip is made again to carry it.
            crate::macos_sign::staple(app)?;
            make_archive(&dist, &stem)?;
        }
    }
    if let Some(key) = &update {
        let manifest = write_manifest(
            &archive,
            &ManifestFor {
                game: &project.package,
                version: &project.version,
                channel: &options.channel,
                platform: &platform,
                notes: &notes,
            },
            key,
        )?;
        let name = manifest
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        println!(
            "Update manifest: dist/{name}\n\
             Publish it beside the archive, under this name: as assets of the same GitHub \
             release, or in the folder the game's manifest URL points at."
        );
    }
    Ok(archive)
}

/// The update identity a release is built with. Empty, every variable is
/// still set, so none in the developer's shell reaches a release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UpdateIdentity {
    key: String,
    game: String,
    version: String,
    channel: String,
}

/// What a release's update manifest says, besides its archive.
struct ManifestFor<'a> {
    game: &'a str,
    version: &'a str,
    channel: &'a str,
    platform: &'a str,
    notes: &'a str,
}

/// Signs the update manifest for `archive` and writes it beside it, as
/// `update-<channel>-<platform>.json`.
fn write_manifest(
    archive: &Path,
    what: &ManifestFor,
    key: &hearken_update::SecretKey,
) -> Result<PathBuf, String> {
    let name = archive
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let size = std::fs::metadata(archive)
        .map_err(|e| format!("Could not read {name}: {e}"))?
        .len();
    let sha256 = hearken_update::download::sha256_file(archive)
        .map_err(|e| format!("Could not read {name}: {e}"))?;
    let manifest = hearken_update::Manifest {
        format: hearken_update::manifest::FORMAT,
        game: what.game.to_string(),
        version: what.version.to_string(),
        channel: what.channel.to_string(),
        platform: what.platform.to_string(),
        archive: name,
        url: None,
        size,
        sha256,
        notes: what.notes.to_string(),
        published: Some(utc_now()),
    };
    let text = hearken_update::manifest::sign(&manifest, key)
        .map_err(|e| format!("Could not sign the update manifest: {e}"))?;
    let path = archive.with_file_name(hearken_update::source::manifest_name(
        what.channel,
        what.platform,
    ));
    std::fs::write(&path, text).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// Release notes for the manifest, from a Markdown file.
fn read_notes(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Could not read the notes {}: {e}", path.display()))?;
    if text.len() > hearken_update::manifest::MAX_NOTES_BYTES {
        return Err(format!(
            "The notes in {} are longer than {} KB.",
            path.display(),
            hearken_update::manifest::MAX_NOTES_BYTES / 1024
        ));
    }
    Ok(text)
}

/// Now, in UTC, as RFC 3339: `2026-10-30T02:12:20Z`.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339(secs)
}

/// Seconds since 1970 as an RFC 3339 UTC time (Hinnant's civil-from-days).
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// Archives `dist/stem` as `dist/stem.zip`, or `.tar.gz` on Linux.
fn make_archive(dist: &Path, stem: &str) -> Result<PathBuf, String> {
    let archive = if cfg!(target_os = "linux") {
        format!("{stem}.tar.gz")
    } else {
        format!("{stem}.zip")
    };
    let _ = std::fs::remove_file(dist.join(&archive));
    let mut cmd = if cfg!(target_os = "macos") {
        // ditto keeps an app whole: its symlinks, permissions and
        // signature.
        let mut ditto = Command::new("ditto");
        ditto
            .args(["-c", "-k", "--keepParent"])
            .arg(stem)
            .arg(&archive);
        ditto
    } else {
        let mut tar = Command::new("tar");
        if cfg!(target_os = "linux") {
            tar.arg("-czf");
        } else {
            // bsdtar, on Windows, writes a zip when the name ends .zip.
            tar.args(["-a", "-cf"]);
        }
        tar.arg(&archive).arg(stem);
        tar
    };
    let packed = cmd.current_dir(dist).status().is_ok_and(|s| s.success());
    if !packed {
        return Err(format!("Could not pack {archive}."));
    }
    Ok(dist.join(archive))
}

/// The keys, rules ID and update identity a release's build gets: all
/// always set, empty when unused, so none in the developer's shell reaches
/// a release.
fn key_env<'a>(
    pack: &'a str,
    seal: &'a str,
    rules: &'a str,
    update: &'a UpdateIdentity,
) -> [(&'static str, &'a str); 8] {
    [
        (PACK_KEY_VAR, pack),
        (crate::seal_key::BUILD_VAR, seal),
        (crate::rules_module::RULES_VAR, rules),
        // A release, with or without rules: no developer override is read.
        (crate::rules_module::RELEASE_VAR, "1"),
        (crate::update_key::KEY_VAR, &update.key),
        (crate::update_key::GAME_VAR, &update.game),
        (crate::update_key::VERSION_VAR, &update.version),
        (crate::update_key::CHANNEL_VAR, &update.channel),
    ]
}

/// The rules module's name in `dist`: the game, its version, and the
/// start of its ID, since each platform's release may build its own.
fn rules_file_name(package: &str, version: &str, id: &str) -> String {
    format!("{package}-{version}-rules-{}.wasm", &id[..id.len().min(12)])
}

/// Refuses a packed release of a game whose Hearken predates sound packs:
/// its program could not read `sounds.pak` and would play no sounds.
fn engine_reads_packs(dir: &Path) -> Result<(), String> {
    let lock = std::fs::read_to_string(dir.join("Cargo.lock")).unwrap_or_default();
    if lock.contains("name = \"hearken-pack\"") {
        Ok(())
    } else {
        Err(
            "This game's Hearken is too old to read a sound pack. Update it with \
             `cargo update -p hearken`, or turn `pack` off."
                .to_string(),
        )
    }
}

/// Writes `sounds` into the stage as `sounds.pak`, checks it opens with
/// `key`, and copies any file in it that is not a sound (credits, say) loose
/// into the stage's `sounds` folder.
fn pack(sounds: &Path, stage: &Path, key: &[u8; 32]) -> Result<(), String> {
    let out = stage.join(hearken_pack::FILE_NAME);
    let written = hearken_pack::write(sounds, &out, key)
        .map_err(|e| format!("Could not pack {}: {e}", sounds.display()))?;
    let pack = hearken_pack::Pack::open(&out, key)
        .map_err(|e| format!("The sound pack does not open: {e}"))?;
    if pack.len() != written.sounds {
        return Err("The sound pack lost sounds while it was written.".to_string());
    }
    for other in &written.others {
        copy(&sounds.join(other), &stage.join(SOUNDS).join(other))?;
    }
    println!(
        "Packed {} sounds into {}.",
        written.sounds,
        hearken_pack::FILE_NAME
    );
    Ok(())
}

/// SDL2 and any other library the build put beside the exe, and the Visual
/// C++ runtime, which a player's PC may not have.
fn windows_runtime(target: &Path, stage: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(target)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        {
            copy(&path, &stage.join(entry.file_name()))?;
        }
    }
    directml_notices(stage)?;
    match vc_runtime() {
        Some(crt) => {
            for entry in std::fs::read_dir(&crt)
                .map_err(|e| e.to_string())?
                .flatten()
            {
                copy(&entry.path(), &stage.join(entry.file_name()))?;
            }
        }
        None => println!(
            "Could not find the Visual C++ runtime to ship; players without it installed \
             will need it."
        ),
    }
    Ok(())
}

/// Microsoft's DirectML licence (1.15.4, the redistributable ONNX Runtime
/// 1.22 is built against) and its third-party notices.
const DIRECTML_LICENSE: &str = include_str!("../licenses/directml/LICENSE.txt");
const DIRECTML_NOTICES: &str = include_str!("../licenses/directml/ThirdPartyNotices.txt");

/// A game with AI voices ships `DirectML.dll`. Its licence allows that
/// inside a game for Windows, and its third-party notices (Zstandard's BSD
/// licence among them) must travel with it, so both go beside the DLL.
fn directml_notices(stage: &Path) -> Result<(), String> {
    if !stage.join("DirectML.dll").is_file() {
        return Ok(());
    }
    for (name, text) in [
        ("DirectML-LICENSE.txt", DIRECTML_LICENSE),
        ("DirectML-ThirdPartyNotices.txt", DIRECTML_NOTICES),
    ] {
        std::fs::write(stage.join(name), text)
            .map_err(|e| format!("Could not write {name}: {e}"))?;
    }
    Ok(())
}

/// The newest `Microsoft.VC*.CRT` folder of the installed Visual Studio,
/// for the processor the game was built for.
fn vc_runtime() -> Option<PathBuf> {
    let (arch, tools) = vc_arch(std::env::consts::ARCH)?;
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    let out = Command::new(vswhere)
        .args(["-latest", "-products", "*", "-requires", tools])
        .args(["-property", "installationPath"])
        .output()
        .ok()?;
    let vs = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let mut versions: Vec<PathBuf> = std::fs::read_dir(vs.join(r"VC\Redist\MSVC"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    versions.sort_by_key(|p| {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        name.split('.')
            .map(|n| n.parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    });
    std::fs::read_dir(versions.pop()?.join(arch))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            name.starts_with("Microsoft.VC") && name.ends_with(".CRT")
        })
}

/// Visual Studio's redist folder for `arch`, and the C++ tools component
/// that builds for it.
fn vc_arch(arch: &str) -> Option<(&'static str, &'static str)> {
    match arch {
        "x86_64" => Some(("x64", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64")),
        "x86" => Some(("x86", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64")),
        "aarch64" => Some(("arm64", "Microsoft.VisualStudio.Component.VC.Tools.ARM64")),
        _ => None,
    }
}

/// Copies a file, or a folder and everything in it.
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("Could not copy {}: {e}", from.display());
    if from.is_dir() {
        std::fs::create_dir_all(to).map_err(fail)?;
        for entry in std::fs::read_dir(from).map_err(fail)?.flatten() {
            copy(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(fail)?;
        }
        std::fs::copy(from, to).map(|_| ()).map_err(fail)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        directml_notices, engine_reads_packs, key_env, read_notes, rfc3339, rules_file_name,
        vc_arch, write_manifest, ManifestFor, UpdateIdentity,
    };

    #[test]
    fn a_build_without_keys_still_clears_them() {
        assert_eq!(
            key_env("", "", "", &UpdateIdentity::default()),
            [
                ("HEARKEN_PACK_KEY", ""),
                ("HEARKEN_SEAL_KEY", ""),
                ("HEARKEN_RULES_ID", ""),
                ("HEARKEN_RELEASE", "1"),
                ("HEARKEN_UPDATE_KEY", ""),
                ("HEARKEN_UPDATE_GAME", ""),
                ("HEARKEN_UPDATE_VERSION", ""),
                ("HEARKEN_UPDATE_CHANNEL", ""),
            ]
        );
    }

    #[test]
    fn the_update_manifest_is_signed_beside_its_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("my-game-1.2.0-linux-x64.tar.gz");
        std::fs::write(&archive, b"archive bytes").unwrap();
        let key = hearken_update::SecretKey::generate().unwrap();
        let path = write_manifest(
            &archive,
            &ManifestFor {
                game: "my-game",
                version: "1.2.0",
                channel: "beta",
                platform: "linux-x64",
                notes: "- Faster.",
            },
            &key,
        )
        .unwrap();
        assert_eq!(path, dir.path().join("update-beta-linux-x64.json"));
        let text = std::fs::read(&path).unwrap();
        let manifest = hearken_update::manifest::open(&text, &key.public()).unwrap();
        assert_eq!(manifest.archive, "my-game-1.2.0-linux-x64.tar.gz");
        assert_eq!(manifest.size, 13);
        assert_eq!(
            manifest.sha256,
            hearken_update::download::sha256_file(&archive).unwrap()
        );
        assert_eq!(
            (manifest.channel.as_str(), manifest.version.as_str()),
            ("beta", "1.2.0")
        );
        assert_eq!(manifest.notes, "- Faster.");
        assert!(manifest.published.is_some());
        // The secret never lands in the file.
        assert!(!String::from_utf8(text).unwrap().contains(&key.to_hex()));
    }

    #[test]
    fn notes_are_read_whole_or_refused() {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("NOTES.md");
        std::fs::write(&notes, "## New\n- A thing.").unwrap();
        assert_eq!(read_notes(&notes).unwrap(), "## New\n- A thing.");
        std::fs::write(&notes, "x".repeat(70_000)).unwrap();
        assert!(read_notes(&notes).is_err());
        assert!(read_notes(&dir.path().join("missing.md")).is_err());
    }

    #[test]
    fn times_are_written_as_rfc3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_793_326_340), "2026-10-30T02:12:20Z");
    }

    #[test]
    fn a_rules_module_is_named_for_its_build() {
        assert_eq!(
            rules_file_name("wickets", "1.2.0", &"ab".repeat(32)),
            "wickets-1.2.0-rules-abababababab.wasm"
        );
    }

    #[test]
    fn ships_the_runtime_for_the_build_processor() {
        assert_eq!(vc_arch("x86_64").map(|a| a.0), Some("x64"));
        assert_eq!(vc_arch("aarch64").map(|a| a.0), Some("arm64"));
        assert_eq!(vc_arch("riscv64"), None);
    }

    #[test]
    fn directml_ships_with_its_licence() {
        let dir = tempfile::tempdir().unwrap();
        directml_notices(dir.path()).unwrap();
        assert!(!dir.path().join("DirectML-LICENSE.txt").exists());
        std::fs::write(dir.path().join("DirectML.dll"), "").unwrap();
        directml_notices(dir.path()).unwrap();
        let licence = std::fs::read_to_string(dir.path().join("DirectML-LICENSE.txt")).unwrap();
        assert!(licence.contains("games that run on Windows and Xbox"));
        let notices =
            std::fs::read_to_string(dir.path().join("DirectML-ThirdPartyNotices.txt")).unwrap();
        assert!(notices.contains("Zstandard"));
    }

    #[test]
    fn packing_needs_an_engine_that_reads_packs() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("Cargo.lock");
        std::fs::write(&lock, "[[package]]\nname = \"hearken\"\n").unwrap();
        assert!(engine_reads_packs(dir.path()).is_err());
        std::fs::write(&lock, "[[package]]\nname = \"hearken-pack\"\n").unwrap();
        assert!(engine_reads_packs(dir.path()).is_ok());
    }

    #[test]
    fn names_match_what_play_downloads() {
        let platform = hearken_update::platform::name;
        assert_eq!(platform("windows", "x86_64"), "windows-x64");
        assert_eq!(platform("linux", "x86_64"), "linux-x64");
        assert_eq!(platform("linux", "aarch64"), "linux-arm64");
        assert_eq!(platform("macos", "aarch64"), "macos-arm64");
        assert_eq!(platform("macos", "x86_64"), "macos");
    }
}
