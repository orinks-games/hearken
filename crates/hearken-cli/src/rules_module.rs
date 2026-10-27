//! A release's rules module: the game's rules crate built for WebAssembly,
//! for a server to replay runs with. Its SHA-256 is the release's rules ID,
//! baked into the game so each run it records names the module that can
//! replay it.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::project::Project;

/// The variable `hearken-replay` reads the rules ID from at build time.
pub const RULES_VAR: &str = "HEARKEN_RULES_ID";

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

/// Rust flags for the module: paths written into it (panic messages name
/// source files) are made the same on every machine, so the same source
/// builds the same module wherever it can, and memory is capped.
pub fn rustflags(workspace: &Path, cargo_home: Option<&Path>) -> String {
    let mut flags = vec![
        format!("--remap-path-prefix={}=/game", workspace.display()),
        format!("-Clink-arg=--max-memory={MAX_MEMORY}"),
    ];
    if let Some(home) = cargo_home {
        flags.push(format!("--remap-path-prefix={}=/cargo", home.display()));
    }
    flags.join("\x1f")
}

/// Builds the game's rules crate for WebAssembly and checks the module.
pub fn build(project: &Project, rules: &crate::project::Rules) -> Result<Module, String> {
    println!("Building the rules module from {}...", rules.package);
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")));
    let built = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--locked",
            "--lib",
            "--target",
            TARGET,
        ])
        .args(["-p", &rules.package])
        // The module carries no keys and names no module of its own.
        .env(RULES_VAR, "")
        .env("HEARKEN_PACK_KEY", "")
        .env(crate::seal_key::BUILD_VAR, "")
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            rustflags(&project.dir, cargo_home.as_deref()),
        )
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
/// could not be replayed exactly) and exports what a server calls.
pub fn check(bytes: &[u8]) -> Result<(), String> {
    let sections = sections(bytes).ok_or("is not a WebAssembly module.")?;
    if let Some(import) = sections.imports.first() {
        return Err(format!(
            "imports {import}. Rules may not import anything: a clock, randomness or \
             wasm-bindgen in the rules crate does this."
        ));
    }
    let missing: Vec<&str> = EXPORTS
        .into_iter()
        .filter(|e| !sections.exports.iter().any(|x| x == e))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "lacks {}. Add `hearken_replay::export_rules!(...)` to the rules crate, and make it \
             a cdylib.",
            missing.join(", ")
        ));
    }
    Ok(())
}

#[derive(Default)]
struct Sections {
    imports: Vec<String>,
    exports: Vec<String>,
}

/// The import and export names of a module; `None` when it does not parse.
fn sections(bytes: &[u8]) -> Option<Sections> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(8)? != b"\0asm\x01\0\0\0" {
        return None;
    }
    let mut out = Sections::default();
    while r.at < bytes.len() {
        let id = r.byte()?;
        let size = r.u32()? as usize;
        let body = r.take(size)?;
        let mut s = Reader { bytes: body, at: 0 };
        match id {
            2 => {
                // The first import's name is enough: one is a refusal.
                if s.u32()? > 0 {
                    let module = s.name()?;
                    let name = s.name()?;
                    out.imports.push(format!("{module}.{name}"));
                }
            }
            7 => {
                for _ in 0..s.u32()? {
                    out.exports.push(s.name()?);
                    s.byte()?;
                    s.u32()?;
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

    fn u32(&mut self) -> Option<u32> {
        let mut n = 0u32;
        for i in 0..5 {
            let b = self.byte()?;
            n |= u32::from(b & 0x7f).checked_shl(7 * i)?;
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

    /// A module with one function import and the given exports (all as
    /// function 0), made by hand.
    fn module(import: bool, exports: &[&str]) -> Vec<u8> {
        let mut m = b"\0asm\x01\0\0\0".to_vec();
        // Type section: one () -> () type.
        m.extend_from_slice(&[1, 4, 1, 0x60, 0, 0]);
        if import {
            let body = [&[1u8, 3][..], b"env", &[3], b"now", &[0, 0]].concat();
            m.push(2);
            m.push(body.len() as u8);
            m.extend_from_slice(&body);
        }
        let mut body = vec![exports.len() as u8];
        for e in exports {
            body.push(e.len() as u8);
            body.extend_from_slice(e.as_bytes());
            body.extend_from_slice(&[0, 0]);
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
    fn module_paths_are_the_same_on_every_machine() {
        let flags = rustflags(Path::new("/w"), Some(Path::new("/h/.cargo")));
        let flags: Vec<&str> = flags.split('\x1f').collect();
        assert_eq!(
            flags,
            [
                "--remap-path-prefix=/w=/game",
                "-Clink-arg=--max-memory=268435456",
                "--remap-path-prefix=/h/.cargo=/cargo"
            ]
        );
    }
}
