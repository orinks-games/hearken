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
    let key = if project.pack {
        Some(hearken_pack::new_key().map_err(|e| format!("Could not make a pack key: {e}"))?)
    } else {
        None
    };
    let hex = key.as_ref().map(hearken_pack::key_hex).unwrap_or_default();
    project.build_with(false, &[(PACK_KEY_VAR, &hex)])?;
    let stem = format!(
        "{}-{}-{}",
        project.package,
        project.version,
        platform(std::env::consts::OS, std::env::consts::ARCH)
    );
    let dist = project.dir.join("dist");
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
    if project.pack && !packed {
        println!("Nothing to pack: the release includes no {SOUNDS} folder.");
    }
    if cfg!(windows) {
        windows_runtime(&project.dir.join("target").join("release"), &stage)?;
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

/// The newest x64 `Microsoft.VC*.CRT` folder of the installed Visual Studio.
fn vc_runtime() -> Option<PathBuf> {
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    let out = Command::new(vswhere)
        .args(["-latest", "-products", "*", "-requires"])
        .arg("Microsoft.VisualStudio.Component.VC.Tools.x86.x64")
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
    std::fs::read_dir(versions.pop()?.join("x64"))
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
    use super::platform;

    #[test]
    fn names_match_what_play_downloads() {
        assert_eq!(platform("windows", "x86_64"), "windows-x64");
        assert_eq!(platform("linux", "x86_64"), "linux-x64");
        assert_eq!(platform("linux", "aarch64"), "linux-arm64");
        assert_eq!(platform("macos", "aarch64"), "macos-arm64");
        assert_eq!(platform("macos", "x86_64"), "macos");
    }
}
