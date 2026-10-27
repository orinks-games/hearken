//! A release's rules module: the game's rules crate built for WebAssembly,
//! for a server to replay runs with. Its SHA-256 is the release's rules ID,
//! baked into the game so each run it records names the module that can
//! replay it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::Project;

/// The variable `hearken-replay` reads the rules ID from at build time.
pub const RULES_VAR: &str = "HEARKEN_RULES_ID";

/// Set to 1 for every build `hearken release` makes, so the engine knows a
/// release from a developer build (`hearken_replay::BuildId::is_release`).
pub const RELEASE_VAR: &str = "HEARKEN_RELEASE";

/// The target a rules module is built for.
pub const TARGET: &str = "wasm32-unknown-unknown";

/// The most memory a rules module may grow to: 256 MiB. A run that needs
/// more is refused by the module rather than taking a server down.
pub const MAX_MEMORY: u64 = 256 << 20;

/// What a rules module must export (`hearken_replay::export_rules!`).
const EXPORTS: [&str; 7] = [
    "memory",
    "hearken_abi",
    "hearken_alloc",
    "hearken_free",
    "hearken_rules",
    "hearken_inspect",
    "hearken_verify",
];

/// A built rules module.
pub struct Module {
    pub path: PathBuf,
    /// Its SHA-256, as 64 hex digits.
    pub id: String,
}

/// Paths written into the module (panic messages name source files) made
/// the same on every machine: the workspace, Cargo's home, and each path
/// dependency outside the workspace. The most specific prefix goes last,
/// since rustc applies the last that matches.
pub fn remaps(
    workspace: &Path,
    cargo_home: Option<&Path>,
    outside: &[(String, PathBuf)],
) -> String {
    let mut prefixes: Vec<(PathBuf, String)> = vec![(workspace.to_path_buf(), "/game".into())];
    if let Some(home) = cargo_home {
        prefixes.push((home.to_path_buf(), "/cargo".into()));
    }
    for (name, dir) in outside {
        prefixes.push((dir.clone(), format!("/path/{name}")));
    }
    prefixes.sort_by_key(|(p, _)| p.as_os_str().len());
    prefixes
        .iter()
        .map(|(from, to)| format!("--remap-path-prefix={}={to}", from.display()))
        .collect::<Vec<_>>()
        .join("\x1f")
}

/// Where the developer has set Rust flags of their own: an environment
/// variable, or a Cargo config file that mentions `rustflags`. Cargo lets
/// the first of those replace the rest, so the module's path remapping is
/// left out rather than drop the developer's flags.
fn own_rustflags(dir: &Path, cargo_home: Option<&Path>) -> Option<String> {
    for var in [
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTFLAGS",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS",
    ] {
        if std::env::var_os(var).is_some_and(|v| !v.is_empty()) {
            return Some(var.to_string());
        }
    }
    let mut dirs: Vec<PathBuf> = dir.ancestors().map(|d| d.join(".cargo")).collect();
    dirs.extend(cargo_home.map(Path::to_path_buf));
    for d in dirs {
        for name in ["config.toml", "config"] {
            let file = d.join(name);
            if std::fs::read_to_string(&file).is_ok_and(|t| t.contains("rustflags")) {
                return Some(file.display().to_string());
            }
        }
    }
    None
}

/// `cargo tree` lines as package and features: `serde v1.0.0|default,std`.
fn features_by_package(tree: &str) -> std::collections::BTreeMap<String, String> {
    tree.lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(" (*)");
            let (package, features) = line.split_once('|')?;
            Some((package.trim().to_string(), features.trim().to_string()))
        })
        .collect()
}

fn cargo_tree(dir: &Path, args: &[String]) -> Result<String, String> {
    let out = Command::new("cargo")
        .arg("tree")
        .args(args)
        .args(["--locked", "--prefix", "none", "--format", "{p}|{f}"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("Could not run cargo tree: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The features the game's release turns on in its rules crate. The module
/// is built with exactly these, so the server runs the rules the player
/// played.
fn desktop_features(
    project: &Project,
    rules: &crate::project::Rules,
) -> Result<Vec<String>, String> {
    let mut args: Vec<String> = [
        "-p",
        &project.package,
        "-e",
        "features",
        "-i",
        &rules.package,
    ]
    .map(String::from)
    .to_vec();
    args.extend(project.feature_args(false));
    args.extend(["--depth", "0"].map(String::from));
    let tree = cargo_tree(&project.dir, &args).map_err(|_| {
        format!(
            "{} does not use its rules crate {}: the game must play the rules a server \
             replays, so add it to the game's [dependencies].",
            project.package, rules.package
        )
    })?;
    let line = tree.lines().next().unwrap_or_default();
    let Some((_, features)) = line.split_once('|') else {
        return Err(format!(
            "{} does not use its rules crate {}.",
            project.package, rules.package
        ));
    };
    Ok(features
        .split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(String::from)
        .collect())
}

/// Dependencies of the rules crate that the game's build compiles with other
/// features than the module does: a feature can change what code runs, so
/// each is named for the developer to check.
fn feature_differences(
    project: &Project,
    rules: &crate::project::Rules,
    features: &[String],
) -> Vec<String> {
    let mut game: Vec<String> = ["-p", &project.package, "-e", "normal"]
        .map(String::from)
        .to_vec();
    game.extend(project.feature_args(false));
    let mut module: Vec<String> = ["-p", &rules.package, "-e", "normal", "--target", TARGET]
        .map(String::from)
        .to_vec();
    module.push("--no-default-features".into());
    if !features.is_empty() {
        module.extend(["--features".to_string(), features.join(",")]);
    }
    let (Ok(game), Ok(module)) = (
        cargo_tree(&project.dir, &game),
        cargo_tree(&project.dir, &module),
    ) else {
        return Vec::new();
    };
    let game = features_by_package(&game);
    features_by_package(&module)
        .into_iter()
        .filter_map(|(package, wasm)| {
            let desktop = game.get(&package)?;
            (desktop != &wasm).then(|| format!("{package}: game [{desktop}], module [{wasm}]"))
        })
        .collect()
}

/// Path dependencies outside the workspace, by name, for remapping.
fn outside_paths(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(out) = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            TARGET,
        ])
        .current_dir(dir)
        .output()
    else {
        return Vec::new();
    };
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let root = PathBuf::from(meta["workspace_root"].as_str().unwrap_or_default());
    meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["source"].is_null())
        .filter_map(|p| {
            let manifest = PathBuf::from(p["manifest_path"].as_str()?);
            let dir = manifest.parent()?.to_path_buf();
            let name = p["name"].as_str()?.to_string();
            (!dir.starts_with(&root)).then_some((name, dir))
        })
        .collect()
}

/// Builds the game's rules crate for WebAssembly, with the features the
/// game's own build gives it, and checks the module.
pub fn build(project: &Project, rules: &crate::project::Rules) -> Result<Module, String> {
    println!("Building the rules module from {}...", rules.package);
    let features = desktop_features(project, rules)?;
    for difference in feature_differences(project, rules, &features) {
        println!(
            "Warning: the rules module and the game build {difference}. Check none of these \
             features change the rules, or turn them on in {} itself.",
            rules.package
        );
    }
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")));
    let mut cargo = Command::new("cargo");
    cargo
        .args([
            "rustc",
            "--release",
            "--locked",
            "--lib",
            "--target",
            TARGET,
        ])
        .args(["-p", &rules.package, "--no-default-features"]);
    if !features.is_empty() {
        cargo.args(["--features", &features.join(",")]);
    }
    // For the module alone: its memory is capped.
    cargo.args(["--", &format!("-Clink-arg=--max-memory={MAX_MEMORY}")]);
    match own_rustflags(&project.dir, cargo_home.as_deref()) {
        None => {
            let outside = outside_paths(&project.dir);
            cargo.env(
                "CARGO_ENCODED_RUSTFLAGS",
                remaps(&project.dir, cargo_home.as_deref(), &outside),
            );
        }
        Some(source) => println!(
            "The rules module keeps the Rust flags from {source}, so the paths it was built \
             from are written into it as they are on this computer."
        ),
    }
    let built = cargo
        // The module carries no keys and names no module of its own.
        .env(RULES_VAR, "")
        .env("HEARKEN_PACK_KEY", "")
        .env(crate::seal_key::BUILD_VAR, "")
        .env(RELEASE_VAR, "1")
        .current_dir(&project.dir)
        .status()
        .is_ok_and(|s| s.success());
    if !built {
        return Err(format!(
            "The rules module did not build. Rules build for {TARGET}: add it with `rustup target \
             add {TARGET}` (or to rust-toolchain.toml's targets), and keep {} free of audio, \
             speech and window code.",
            rules.package
        ));
    }
    let path = project
        .target_dir
        .join(TARGET)
        .join("release")
        .join(format!("{}.wasm", rules.lib));
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Could not read the rules module {}: {e}", path.display()))?;
    check(&bytes).map_err(|e| format!("The rules module {} {e}", path.display()))?;
    Ok(Module {
        id: hearken_replay::BuildId::of_module(&bytes).to_hex(),
        path,
    })
}

/// Checks a module imports nothing (rules that read a clock or the host
/// could not be replayed exactly), exports what a server calls as the
/// right kinds, and caps its memory.
pub fn check(bytes: &[u8]) -> Result<(), String> {
    let sections = sections(bytes).ok_or("is not a well-formed WebAssembly module.")?;
    if let Some(import) = sections.imports.first() {
        return Err(format!(
            "imports {import}. Rules may not import anything: a clock, randomness or \
             wasm-bindgen in the rules crate does this."
        ));
    }
    let missing: Vec<&str> = EXPORTS
        .into_iter()
        .filter(|name| {
            let kind = if *name == "memory" {
                MEMORY_KIND
            } else {
                FUNC_KIND
            };
            !sections
                .exports
                .iter()
                .any(|(n, k)| n == name && *k == kind)
        })
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "lacks {}. Add `hearken_replay::export_rules!(...)` to the rules crate, and make it \
             a cdylib.",
            missing.join(", ")
        ));
    }
    match sections.max_pages {
        Some(pages) if u64::from(pages) * PAGE <= MAX_MEMORY => Ok(()),
        _ => Err(format!(
            "can grow its memory past {} MiB. It must be built by `hearken release`, which caps it.",
            MAX_MEMORY >> 20
        )),
    }
}

const FUNC_KIND: u8 = 0;
const MEMORY_KIND: u8 = 2;
const PAGE: u64 = 64 * 1024;

#[derive(Default)]
struct Sections {
    imports: Vec<String>,
    exports: Vec<(String, u8)>,
    /// The first memory's maximum, in 64 KiB pages; `None` when it has none.
    max_pages: Option<u32>,
}

/// The imports, exports and memory cap of a module; `None` when it does not
/// parse: a section repeated or unknown, or a number that overflows.
fn sections(bytes: &[u8]) -> Option<Sections> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(8)? != b"\0asm\x01\0\0\0" {
        return None;
    }
    let mut out = Sections::default();
    let mut seen = [false; 14];
    while r.at < bytes.len() {
        let id = r.byte()?;
        let size = r.u32()? as usize;
        let body = r.take(size)?;
        let mut s = Reader { bytes: body, at: 0 };
        if id != 0 {
            let slot = seen.get_mut(usize::from(id))?;
            if *slot {
                return None;
            }
            *slot = true;
        }
        match id {
            2 => {
                // The first import's name is enough: one is a refusal.
                if s.u32()? > 0 {
                    let module = s.name()?;
                    let name = s.name()?;
                    out.imports.push(format!("{module}.{name}"));
                }
            }
            5 => {
                if s.u32()? > 0 {
                    let flags = s.byte()?;
                    // Bit 2 is a 64-bit memory, which rules never need.
                    if flags & !0b11 != 0 {
                        return None;
                    }
                    s.u32()?;
                    if flags & 1 == 1 {
                        out.max_pages = Some(s.u32()?);
                    }
                }
            }
            7 => {
                for _ in 0..s.u32()? {
                    let name = s.name()?;
                    let kind = s.byte()?;
                    s.u32()?;
                    out.exports.push((name, kind));
                }
            }
            _ => {}
        }
    }
    Some(out)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let out = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(out)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    /// A LEB128 `u32`; `None` when it runs past 32 bits.
    fn u32(&mut self) -> Option<u32> {
        let mut n = 0u32;
        for i in 0..5 {
            let b = self.byte()?;
            if i == 4 && b & 0xf0 != 0 {
                return None;
            }
            n |= u32::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                return Some(n);
            }
        }
        None
    }

    fn name(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module with one function import and the given exports (memory as
    /// a memory, the rest as function 0), its memory capped at 16 pages,
    /// made by hand.
    fn module(import: bool, exports: &[&str]) -> Vec<u8> {
        module_with(import, exports, &[5, 4, 1, 1, 1, 16])
    }

    fn module_with(import: bool, exports: &[&str], memory: &[u8]) -> Vec<u8> {
        let mut m = b"\0asm\x01\0\0\0".to_vec();
        // Type section: one () -> () type.
        m.extend_from_slice(&[1, 4, 1, 0x60, 0, 0]);
        if import {
            let body = [&[1u8, 3][..], b"env", &[3], b"now", &[0, 0]].concat();
            m.push(2);
            m.push(body.len() as u8);
            m.extend_from_slice(&body);
        }
        m.extend_from_slice(memory);
        let mut body = vec![exports.len() as u8];
        for e in exports {
            body.push(e.len() as u8);
            body.extend_from_slice(e.as_bytes());
            body.extend_from_slice(&[if *e == "memory" { 2 } else { 0 }, 0]);
        }
        m.push(7);
        m.push(body.len() as u8);
        m.extend_from_slice(&body);
        m
    }

    #[test]
    fn a_module_with_every_export_and_no_import_passes() {
        assert_eq!(check(&module(false, &EXPORTS)), Ok(()));
    }

    #[test]
    fn an_import_is_refused() {
        let err = check(&module(true, &EXPORTS)).unwrap_err();
        assert!(err.starts_with("imports env.now."), "{err}");
    }

    #[test]
    fn a_missing_export_is_named() {
        let err = check(&module(false, &EXPORTS[..6])).unwrap_err();
        assert!(err.contains("hearken_verify"), "{err}");
        assert!(check(b"not wasm").is_err());
        assert!(check(&module(false, &EXPORTS)[..20]).is_err());
    }

    #[test]
    fn exports_must_be_the_right_kind() {
        let mut m = module(false, &EXPORTS);
        // The last export, hearken_verify, made a global.
        let at = m.len() - 2;
        m[at] = 3;
        assert!(check(&m).unwrap_err().contains("hearken_verify"));
    }

    #[test]
    fn memory_must_be_capped() {
        let uncapped = module_with(false, &EXPORTS, &[5, 3, 1, 0, 1]);
        assert!(check(&uncapped).unwrap_err().contains("256 MiB"));
        // 4097 pages, one past 256 MiB.
        let over = module_with(false, &EXPORTS, &[5, 5, 1, 1, 1, 0x81, 0x20]);
        assert!(check(&over).unwrap_err().contains("256 MiB"));
    }

    #[test]
    fn repeated_or_unknown_sections_and_overflowing_numbers_are_refused() {
        let mut twice = module(false, &EXPORTS);
        twice.extend_from_slice(&[5, 4, 1, 1, 1, 16]);
        assert!(check(&twice).is_err());
        let mut unknown = module(false, &EXPORTS);
        unknown.extend_from_slice(&[99, 0]);
        assert!(check(&unknown).is_err());
        let mut r = Reader {
            bytes: &[0xff, 0xff, 0xff, 0xff, 0x7f],
            at: 0,
        };
        assert_eq!(r.u32(), None);
    }

    #[test]
    fn module_paths_are_the_same_on_every_machine() {
        let outside = [(
            "hearken-replay".to_string(),
            PathBuf::from("/w/../engine/crates/r"),
        )];
        let flags = remaps(Path::new("/w"), Some(Path::new("/h/.cargo")), &outside);
        let flags: Vec<&str> = flags.split('\x1f').collect();
        assert_eq!(
            flags,
            [
                "--remap-path-prefix=/w=/game",
                "--remap-path-prefix=/h/.cargo=/cargo",
                "--remap-path-prefix=/w/../engine/crates/r=/path/hearken-replay"
            ]
        );
    }

    #[test]
    fn reads_features_from_cargo_tree() {
        let tree = "dash-rules v1.0.0 (/g/rules)|bonus\nserde v1.0.0|default,std (*)\n";
        let map = features_by_package(tree);
        assert_eq!(map["dash-rules v1.0.0 (/g/rules)"], "bonus");
        assert_eq!(map["serde v1.0.0"], "default,std");
    }
}
