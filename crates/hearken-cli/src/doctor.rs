//! `hearken doctor`: checks that this computer has what building a Hearken
//! game from source needs, and says how to get whatever is missing.
//!
//! Also the environment `hearken play` and `hearken release` build with, so
//! a build works on a computer set up the way the doctor asks for without
//! any variables set by hand.

#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::process::Command;

/// The oldest CMake that builds Prism, SDL2 and libopus.
const CMAKE_MIN: (u32, u32) = (3, 24);
/// The oldest GCC that builds Prism on Linux.
#[cfg(target_os = "linux")]
const GCC_MIN: u32 = 13;

/// One thing a build needs.
struct Check {
    /// What it is, as the player reads it: "CMake 3.24 or newer".
    what: &'static str,
    /// What was found, or `Err` with how to get it.
    found: Result<String, String>,
}

/// Runs every check, prints one line for each, and fails when anything a
/// build needs is missing.
pub fn run() -> Result<(), String> {
    println!("Checking what building a Hearken game from source needs.");
    let checks = checks();
    let mut missing = 0;
    for check in &checks {
        match &check.found {
            Ok(found) => println!("ok: {}: {found}", check.what),
            Err(fix) => {
                missing += 1;
                println!("missing: {}. {fix}", check.what);
            }
        }
    }
    #[cfg(target_os = "linux")]
    if missing > 0 {
        println!(
            "On Debian and Ubuntu, this installs every library a build uses, the window and \
             sound backends among them: {LINUX_PACKAGES}"
        );
    }
    match missing {
        0 => {
            println!("Everything is here. hearken play builds a game from its source.");
            Ok(())
        }
        1 => Err("1 thing is missing. Install it, then run hearken doctor again.".into()),
        n => Err(format!(
            "{n} things are missing. Install them, then run hearken doctor again."
        )),
    }
}

fn checks() -> Vec<Check> {
    let mut checks = vec![
        Check {
            what: "Git",
            found: version_line("git", &["--version"])
                .ok_or_else(|| format!("Install it: {}", git_fix())),
        },
        rust(),
        cmake(),
    ];
    checks.extend(platform_checks());
    checks
}

/// Rust through rustup, which reads each game's `rust-toolchain.toml` and
/// fetches the compiler it pins.
fn rust() -> Check {
    let found = match version_line("rustup", &["--version"]) {
        Some(v) => Ok(v),
        None if version_line("cargo", &["--version"]).is_some() => Err(
            "Rust is installed, but not through rustup, so a game's pinned compiler cannot be \
             fetched. Remove that Rust and install it from https://rustup.rs"
                .into(),
        ),
        None => Err("Install it from https://rustup.rs".into()),
    };
    Check {
        what: "Rust, installed through rustup",
        found,
    }
}

fn cmake() -> Check {
    let fix = cmake_fix();
    let found = match version_line("cmake", &["--version"]) {
        None => Err(format!("Install it: {fix}")),
        Some(line) => match parse_version(&line) {
            Some((major, minor)) if (major, minor) >= CMAKE_MIN => Ok(line),
            _ => Err(format!(
                "This computer has {line}. Install a newer one: {fix}"
            )),
        },
    };
    Check {
        what: "CMake 3.24 or newer",
        found,
    }
}

fn git_fix() -> &'static str {
    match std::env::consts::OS {
        "macos" => "xcode-select --install puts it in with the Xcode Command Line Tools.",
        "windows" => "winget install Git.Git",
        _ => "sudo apt install git on Debian and Ubuntu, or sudo dnf install git on Fedora.",
    }
}

fn cmake_fix() -> &'static str {
    match std::env::consts::OS {
        "macos" => {
            "brew install cmake with Homebrew (https://brew.sh), or the macOS installer from \
             https://cmake.org/download/"
        }
        "windows" => {
            "winget install Kitware.CMake, then open a new terminal. The CMake inside Visual \
             Studio is too old for Prism."
        }
        _ => "sudo apt install cmake on Debian and Ubuntu, or sudo dnf install cmake on Fedora.",
    }
}

#[cfg(target_os = "macos")]
fn platform_checks() -> Vec<Check> {
    let found = Command::new("xcode-select")
        .arg("-p")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .ok_or_else(|| {
            "Run xcode-select --install and follow the dialog. It installs Apple's C and C++ \
             compilers."
                .to_string()
        });
    vec![Check {
        what: "Xcode Command Line Tools",
        found,
    }]
}

#[cfg(windows)]
fn platform_checks() -> Vec<Check> {
    const BUILD_TOOLS: &str = "winget install Microsoft.VisualStudio.2022.BuildTools --override \
        \"--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools \
        --add Microsoft.VisualStudio.Component.VC.ATL --includeRecommended\"";
    let vs = visual_studio();
    let tools = Check {
        what: "Visual Studio Build Tools with Desktop development with C++",
        found: vs
            .as_ref()
            .map(|p| p.display().to_string())
            .ok_or_else(|| format!("Install them: {BUILD_TOOLS}")),
    };
    let atl = Check {
        what: "the C++ ATL component of Visual Studio (Prism's speech includes atlbase.h)",
        found: match &vs {
            None => Err("It comes with the Build Tools command above.".into()),
            Some(vs) if has_atl(vs) => Ok("atlbase.h found".into()),
            Some(vs) => Err(format!(
                "Add it from the Visual Studio Installer, or run: \"{}\" modify --installPath \
                 \"{}\" --add Microsoft.VisualStudio.Component.VC.ATL --passive --norestart",
                installer_dir().join("setup.exe").display(),
                vs.display()
            )),
        },
    };
    vec![tools, atl]
}

#[cfg(windows)]
fn installer_dir() -> PathBuf {
    let base = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    base.join("Microsoft Visual Studio").join("Installer")
}

/// The newest Visual Studio with the x64 C++ tools.
#[cfg(windows)]
fn visual_studio() -> Option<PathBuf> {
    let out = Command::new(installer_dir().join("vswhere.exe"))
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
}

/// Whether any MSVC version in `vs` has ATL's headers.
#[cfg(windows)]
fn has_atl(vs: &Path) -> bool {
    let Ok(versions) = std::fs::read_dir(vs.join(r"VC\Tools\MSVC")) else {
        return false;
    };
    versions
        .flatten()
        .any(|v| v.path().join(r"atlmfc\include\atlbase.h").is_file())
}

/// Every library a Linux build of a game uses, SDL2's backends among them,
/// for one install command.
#[cfg(target_os = "linux")]
const LINUX_PACKAGES: &str = "sudo apt install pkg-config libasound2-dev libpulse-dev \
    libspeechd-dev libx11-dev libxext-dev libxrandr-dev libxcursor-dev libxi-dev libxss-dev \
    libxkbcommon-dev libwayland-dev wayland-protocols libdecor-0-dev libegl1-mesa-dev \
    libgl1-mesa-dev libdbus-1-dev libudev-dev";

#[cfg(target_os = "linux")]
fn platform_checks() -> Vec<Check> {
    let gcc = Check {
        what: "GCC 13 or newer",
        found: match gcc_major("gcc") {
            Some(v) if v >= GCC_MIN => Ok(format!("gcc {v}")),
            found => match (gcc_major("gcc-13"), command_exists("g++-13")) {
                (Some(_), true) => Ok("gcc-13, which hearken play uses itself".into()),
                _ => Err(format!(
                    "{}Install gcc-13 and g++-13 (sudo apt install gcc-13 g++-13).",
                    found.map_or(String::new(), |v| format!("This computer has gcc {v}. "))
                )),
            },
        },
    };
    let pkg_config = command_exists("pkg-config");
    let library = |what: &'static str, modules: &[&str], fix: &str| Check {
        what,
        found: match modules.iter().find(|m| pkg_config && pkg_exists(m)) {
            Some(m) => Ok(format!("{m} found")),
            None => Err(format!("Install {fix}")),
        },
    };
    vec![
        gcc,
        Check {
            what: "pkg-config, which finds the libraries below",
            found: if pkg_config {
                Ok("found".into())
            } else {
                Err("Install pkg-config.".into())
            },
        },
        library(
            "ALSA's development files, for sound",
            &["alsa"],
            "libasound2-dev on Debian and Ubuntu, alsa-lib-devel on Fedora, alsa-lib on Arch.",
        ),
        library(
            "Speech Dispatcher's development files, for speech",
            &["speech-dispatcher"],
            "libspeechd-dev on Debian and Ubuntu, speech-dispatcher-devel on Fedora, \
             speech-dispatcher on Arch.",
        ),
        library(
            "X11 or Wayland development files, for the window",
            &["x11", "wayland-client"],
            "libx11-dev and libwayland-dev on Debian and Ubuntu, libX11-devel and \
             wayland-devel on Fedora.",
        ),
    ]
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn platform_checks() -> Vec<Check> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn pkg_exists(module: &str) -> bool {
    Command::new("pkg-config")
        .args(["--exists", module])
        .status()
        .is_ok_and(|s| s.success())
}

/// GCC's major version, as `program -dumpversion` gives it.
#[cfg(target_os = "linux")]
fn gcc_major(program: &str) -> Option<u32> {
    let out = Command::new(program).arg("-dumpversion").output().ok()?;
    out.status.success().then_some(())?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// The environment a game's build gets on top of the caller's: what a
/// computer set up as `hearken doctor` asks needs for a build to work.
/// Nothing here replaces a variable the caller set.
pub fn build_env() -> Vec<(String, String)> {
    let unset = |name: &str| std::env::var_os(name).is_none();
    let mut env = Vec::new();
    // SDL2's bundled source declares a CMake minimum older than CMake 4
    // accepts; this is CMake's own setting for exactly that.
    if unset("CMAKE_POLICY_VERSION_MINIMUM") {
        env.push(("CMAKE_POLICY_VERSION_MINIMUM".into(), "3.5".into()));
    }
    #[cfg(target_os = "linux")]
    if unset("CC") && unset("CXX") && gcc_major("gcc").is_none_or(|v| v < GCC_MIN) {
        // An older system GCC with gcc-13 beside it, as Ubuntu 22.04 has.
        // The linker has to be gcc-13 too: Prism's objects carry its
        // link-time bytecode.
        if gcc_major("gcc-13").is_some() && command_exists("g++-13") {
            let linker = format!(
                "CARGO_TARGET_{}_UNKNOWN_LINUX_GNU_LINKER",
                std::env::consts::ARCH.to_uppercase()
            );
            env.push(("CC".into(), "gcc-13".into()));
            env.push(("CXX".into(), "g++-13".into()));
            if unset(&linker) {
                env.push((linker, "gcc-13".into()));
            }
        }
    }
    env
}

/// The first line `program args` prints, when it runs.
fn version_line(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status.success().then_some(())?;
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.lines().next().unwrap_or_default().trim().to_string())
}

#[cfg(target_os = "linux")]
fn command_exists(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

/// The first `major.minor` in a version line: `cmake version 3.28.3` is
/// (3, 28).
fn parse_version(line: &str) -> Option<(u32, u32)> {
    let word = line
        .split_whitespace()
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()))?;
    let mut parts = word.split(['.', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cmake_versions() {
        assert_eq!(parse_version("cmake version 3.28.3"), Some((3, 28)));
        assert_eq!(parse_version("cmake version 4.1.0-rc2"), Some((4, 1)));
        assert_eq!(parse_version("cmake version"), None);
        assert!(parse_version("cmake version 3.22.1").unwrap() < CMAKE_MIN);
        assert!(parse_version("cmake version 4.0.0").unwrap() >= CMAKE_MIN);
    }

    #[test]
    fn the_build_keeps_a_cmake_policy_already_set() {
        let set = std::env::var_os("CMAKE_POLICY_VERSION_MINIMUM").is_some();
        let ours = build_env()
            .iter()
            .any(|(k, _)| k == "CMAKE_POLICY_VERSION_MINIMUM");
        assert_eq!(ours, !set);
    }
}
