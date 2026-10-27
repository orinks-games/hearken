//! `hearken release`: a player's zip of the game, without the agent server.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Builds the release, stages it under `dist/` and archives it. Returns the
/// archive's path.
pub fn release(project: &Project) -> Result<PathBuf, String> {
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
    // The rules module first: its ID goes into the game.
    let rules = match &project.rules {
        Some(rules) => Some(crate::rules_module::build(project, rules)?),
        None => None,
    };
    let rules_id = rules.as_ref().map(|m| m.id.clone()).unwrap_or_default();
    project.build_with(false, &key_env(&hex, &seal.unwrap_or_default(), &rules_id))?;
    let stem = format!(
        "{}-{}-{}",
        project.package,
        project.version,
        platform(std::env::consts::OS, std::env::consts::ARCH)
    );
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
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    let exe = project.exe();
    copy(&exe, &stage.join(exe.file_name().unwrap_or_default()))?;
    let mut packed = false;
    for item in &project.include {
        let from = project.dir.join(item);
        if let (Some(key), SOUNDS) = (&key, item.as_str()) {
            if from.is_dir() {
                pack(&from, &stage, key)?;
                packed = true;
                continue;
            }
        }
        if from.exists() {
            copy(&from, &stage.join(item))?;
        } else {
            println!("Skipping {item}: it is not in the workspace.");
        }
    }
    if let (Some(key), false) = (&key, packed) {
        // A packed build reads only its pack, so it still gets one.
        println!("Nothing to pack: the release includes no {SOUNDS} folder.");
        pack(&stage.join(".nothing-to-pack"), &stage, key)?;
    }
    if cfg!(windows) {
        windows_runtime(&project.release_dir(), &stage)?;
    }
    if project.portable {
        std::fs::write(stage.join(PORTABLE_MARKER), PORTABLE_NOTE).map_err(|e| e.to_string())?;
    }

    let archive = if cfg!(target_os = "linux") {
        format!("{stem}.tar.gz")
    } else {
        format!("{stem}.zip")
    };
    let _ = std::fs::remove_file(dist.join(&archive));
    // bsdtar, on Windows and macOS, writes a zip when the name ends .zip.
    let mut tar = Command::new("tar");
    if cfg!(target_os = "linux") {
        tar.arg("-czf");
    } else {
        tar.args(["-a", "-cf"]);
    }
    let packed = tar
        .arg(&archive)
        .arg(&stem)
        .current_dir(&dist)
        .status()
        .is_ok_and(|s| s.success());
    if !packed {
        return Err(format!("Could not pack {archive}."));
    }
    Ok(dist.join(archive))
}

/// The keys and rules ID a release's build gets: all always set, empty
/// when unused, so none in the developer's shell reaches a release.
fn key_env<'a>(pack: &'a str, seal: &'a str, rules: &'a str) -> [(&'static str, &'a str); 4] {
    [
        (PACK_KEY_VAR, pack),
        (crate::seal_key::BUILD_VAR, seal),
        (crate::rules_module::RULES_VAR, rules),
        // A release, with or without rules: no developer override is read.
        (crate::rules_module::RELEASE_VAR, "1"),
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

/// The platform part of a release's name, the one `hearken play --download`
/// looks for.
pub fn platform(os: &str, arch: &str) -> String {
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    match (os, arch) {
        ("macos", "x64") => "macos".to_string(),
        _ => format!("{os}-{arch}"),
    }
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
        directml_notices, engine_reads_packs, key_env, platform, rules_file_name, vc_arch,
    };

    #[test]
    fn a_build_without_keys_still_clears_them() {
        assert_eq!(
            key_env("", "", ""),
            [
                ("HEARKEN_PACK_KEY", ""),
                ("HEARKEN_SEAL_KEY", ""),
                ("HEARKEN_RULES_ID", ""),
                ("HEARKEN_RELEASE", "1")
            ]
        );
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
        assert_eq!(platform("windows", "x86_64"), "windows-x64");
        assert_eq!(platform("linux", "x86_64"), "linux-x64");
        assert_eq!(platform("linux", "aarch64"), "linux-arm64");
        assert_eq!(platform("macos", "aarch64"), "macos-arm64");
        assert_eq!(platform("macos", "x86_64"), "macos");
    }
}
