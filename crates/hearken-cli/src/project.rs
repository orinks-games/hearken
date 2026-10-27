//! What `cargo metadata` says about a game: its package, binary, version,
//! default features and the `[package.metadata.hearken]` table.
//!
//! ```toml
//! [package.metadata.hearken]
//! portable = true                                  # saves beside the game
//! pack = true                                      # sounds in an encrypted sounds.pak
//! seal = true                                      # data written through ctx.vault is sealed
//! seal-id = "k1:3f9a0c2e7b14d685"                  # the seal key's ID, checked by release
//! rules = "my-game-rules"                          # built for WebAssembly, for verified runs
//! include = ["sounds", "README.md", "LICENSE"]     # shipped in a release
//! bin = "my-game"                                  # when it has several
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The Cargo feature a release build leaves out.
pub const AGENT_SERVER: &str = "agent-server";

pub struct Project {
    /// The workspace root.
    pub dir: PathBuf,
    /// Where Cargo puts builds: `target` in the workspace, unless
    /// `CARGO_TARGET_DIR` or a Cargo config moves it.
    pub target_dir: PathBuf,
    pub package: String,
    pub bin: String,
    pub version: String,
    pub default_features: Vec<String>,
    pub portable: bool,
    /// Whether a release ships the `sounds` folder as an encrypted
    /// `sounds.pak` instead of loose files.
    pub pack: bool,
    /// Whether a release seals the game's data files (see `seal_key`).
    pub seal: bool,
    /// The committed ID of the game's seal key, which a release must match.
    pub seal_id: Option<String>,
    /// Files and folders, relative to `dir`, that a release ships.
    pub include: Vec<String>,
    /// The game's rules crate, which a release also builds for WebAssembly
    /// so a server can replay its runs (see `rules_module`).
    pub rules: Option<Rules>,
}

/// A game's rules crate: a workspace package that is a `cdylib` and calls
/// `hearken_replay::export_rules!`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub package: String,
    /// Its library's name, as the `.wasm` file is named.
    pub lib: String,
}

impl Project {
    /// The game in the workspace at `dir`: the package named `package`, or
    /// else the one with a `[package.metadata.hearken]` table.
    pub fn load(dir: &Path, package: Option<&str>) -> Result<Project, String> {
        let out = Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .current_dir(dir)
            .output()
            .map_err(|e| format!("Could not run cargo: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "{} is not a Cargo workspace: {}",
                dir.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let meta: Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("Could not read cargo metadata: {e}"))?;
        Project::from_metadata(&meta, package)
    }

    fn from_metadata(meta: &Value, package: Option<&str>) -> Result<Project, String> {
        let packages = meta["packages"].as_array().cloned().unwrap_or_default();
        let found: Vec<&Value> = packages
            .iter()
            .filter(|p| match package {
                Some(name) => p["name"] == name,
                None => p["metadata"]["hearken"].is_object(),
            })
            .collect();
        let pkg = match (found.as_slice(), package) {
            ([one], _) => *one,
            ([], Some(name)) => return Err(format!("This workspace has no package {name}.")),
            _ => {
                return Err(
                    "Name the game, or add a [package.metadata.hearken] table to \
                     its Cargo.toml so hearken can find it."
                        .to_string(),
                )
            }
        };
        let hearken = &pkg["metadata"]["hearken"];
        let strings = |v: &Value| -> Vec<String> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        };
        let name = pkg["name"].as_str().unwrap_or_default().to_string();
        let bins: Vec<&str> = pkg["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.contains(&"bin".into()))
            })
            .filter_map(|t| t["name"].as_str())
            .collect();
        let bin = match hearken["bin"].as_str() {
            Some(bin) => bin.to_string(),
            None if bins.contains(&name.as_str()) => name.clone(),
            None => bins
                .first()
                .map(|b| b.to_string())
                .ok_or_else(|| format!("{name} has no program to run."))?,
        };
        let include = match hearken.get("include") {
            Some(list) => strings(list),
            None => ["sounds", "README.md", "LICENSE"]
                .map(String::from)
                .to_vec(),
        };
        let rules = match hearken.get("rules") {
            None => None,
            Some(Value::String(rules)) => Some(rules_crate(&packages, rules)?),
            Some(_) => return Err("`rules` names the game's rules crate, as a string.".into()),
        };
        let dir = PathBuf::from(meta["workspace_root"].as_str().unwrap_or("."));
        let target_dir = meta["target_directory"]
            .as_str()
            .map_or_else(|| dir.join("target"), PathBuf::from);
        Ok(Project {
            dir,
            target_dir,
            bin,
            version: pkg["version"].as_str().unwrap_or("0.0.0").to_string(),
            default_features: strings(&pkg["features"]["default"]),
            portable: hearken["portable"].as_bool().unwrap_or(false),
            pack: hearken["pack"].as_bool().unwrap_or(false),
            seal: hearken["seal"].as_bool().unwrap_or(false),
            seal_id: hearken["seal-id"].as_str().map(String::from),
            include,
            rules,
            package: name,
        })
    }

    /// `cargo build` arguments for a release (`dev` false: the agent server
    /// left out) or a developer build (everything the game turns on).
    pub fn build_args(&self, dev: bool) -> Vec<String> {
        let mut args: Vec<String> = [
            "build",
            "--release",
            "-p",
            &self.package,
            "--bin",
            &self.bin,
        ]
        .map(String::from)
        .to_vec();
        if !dev {
            // A release builds exactly the dependencies Cargo.lock pins.
            args.push("--locked".into());
        }
        args.extend(self.feature_args(dev));
        args
    }

    /// The feature flags a build of the game gets: none for a developer
    /// build, and for a release, the game's defaults without the agent
    /// server.
    pub fn feature_args(&self, dev: bool) -> Vec<String> {
        let mut args = Vec::new();
        if !dev && self.default_features.iter().any(|f| f == AGENT_SERVER) {
            let keep: Vec<&str> = self
                .default_features
                .iter()
                .map(String::as_str)
                .filter(|f| *f != AGENT_SERVER)
                .collect();
            args.push("--no-default-features".into());
            if !keep.is_empty() {
                args.push("--features".into());
                args.push(keep.join(","));
            }
        }
        args
    }

    /// The folder a release build lands in.
    pub fn release_dir(&self) -> PathBuf {
        self.target_dir.join("release")
    }

    pub fn exe(&self) -> PathBuf {
        self.release_dir()
            .join(format!("{}{}", self.bin, std::env::consts::EXE_SUFFIX))
    }

    /// Builds the game; `dev` keeps the agent server.
    pub fn build(&self, dev: bool) -> Result<(), String> {
        self.build_with(dev, &[])
    }

    /// Builds the game with `env` set for Cargo.
    pub fn build_with(&self, dev: bool, env: &[(&str, &str)]) -> Result<(), String> {
        println!("Building {}...", self.package);
        let built = Command::new("cargo")
            .args(self.build_args(dev))
            .envs(env.iter().copied())
            .current_dir(&self.dir)
            .status()
            .is_ok_and(|s| s.success());
        built
            .then_some(())
            .ok_or_else(|| format!("{} did not build.", self.package))
    }
}

/// The rules crate `name`, which must be a workspace package with a
/// `cdylib` library.
fn rules_crate(packages: &[Value], name: &str) -> Result<Rules, String> {
    let pkg = packages
        .iter()
        .find(|p| p["name"] == name)
        .ok_or_else(|| format!("The rules crate {name} is not in this workspace."))?;
    let lib = pkg["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|t| {
            t["kind"]
                .as_array()
                .is_some_and(|k| k.contains(&"cdylib".into()))
        })
        .and_then(|t| t["name"].as_str())
        .ok_or_else(|| {
            format!(
                "The rules crate {name} must be a cdylib: add `crate-type = [\"cdylib\", \"rlib\"]` \
                 under [lib] in its Cargo.toml."
            )
        })?;
    Ok(Rules {
        package: name.to_string(),
        lib: lib.replace('-', "_"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(features: Value, hearken: Value) -> Value {
        serde_json::json!({
            "workspace_root": "/games/parlor",
            "packages": [
                { "name": "parlor-core", "version": "0.1.0", "features": {},
                  "metadata": null, "targets": [{ "kind": ["lib"], "name": "parlor_core" }] },
                { "name": "parlor", "version": "0.2.0", "features": features,
                  "metadata": { "hearken": hearken },
                  "targets": [{ "kind": ["bin"], "name": "parlor" }] }
            ]
        })
    }

    #[test]
    fn finds_the_game_by_its_hearken_table() {
        let meta = game(
            serde_json::json!({}),
            serde_json::json!({ "portable": true }),
        );
        let p = Project::from_metadata(&meta, None).unwrap();
        assert_eq!((p.package.as_str(), p.bin.as_str()), ("parlor", "parlor"));
        assert_eq!(p.version, "0.2.0");
        assert!(p.portable);
        assert!(!p.pack);
        assert!(!p.seal);
        assert_eq!(p.seal_id, None);
        assert_eq!(p.include, ["sounds", "README.md", "LICENSE"]);
        assert_eq!(p.target_dir, Path::new("/games/parlor").join("target"));
        assert_eq!(p.rules, None);
    }

    #[test]
    fn finds_the_rules_crate() {
        let mut meta = game(
            serde_json::json!({}),
            serde_json::json!({ "rules": "parlor-core" }),
        );
        let err = Project::from_metadata(&meta, None).err().unwrap();
        assert!(err.contains("must be a cdylib"), "{err}");
        meta["packages"][0]["targets"][0]["kind"] = serde_json::json!(["cdylib", "rlib"]);
        let p = Project::from_metadata(&meta, None).unwrap();
        assert_eq!(
            p.rules,
            Some(Rules {
                package: "parlor-core".into(),
                lib: "parlor_core".into()
            })
        );
        let meta = game(
            serde_json::json!({}),
            serde_json::json!({ "rules": "missing" }),
        );
        assert!(Project::from_metadata(&meta, None).is_err());
    }

    #[test]
    fn builds_are_found_where_cargo_puts_them() {
        let mut meta = game(serde_json::json!({}), serde_json::json!({}));
        meta["target_directory"] = "/cache/cargo-target".into();
        let p = Project::from_metadata(&meta, None).unwrap();
        assert!(p
            .exe()
            .starts_with(Path::new("/cache/cargo-target").join("release")));
    }

    #[test]
    fn reads_the_seal_settings() {
        let hearken = serde_json::json!({ "seal": true, "seal-id": "k1:0011223344556677" });
        let p = Project::from_metadata(&game(serde_json::json!({}), hearken), None).unwrap();
        assert!(p.seal);
        assert_eq!(p.seal_id.as_deref(), Some("k1:0011223344556677"));
    }

    #[test]
    fn a_release_leaves_the_agent_server_out() {
        let features = serde_json::json!({ "default": ["agent-server", "kira"] });
        let p = Project::from_metadata(&game(features, serde_json::json!({})), None).unwrap();
        let release = p.build_args(false);
        assert!(release.ends_with(&[
            "--no-default-features".into(),
            "--features".into(),
            "kira".into()
        ]));
        assert!(!p
            .build_args(true)
            .iter()
            .any(|a| a == "--no-default-features"));
    }

    #[test]
    fn a_game_without_an_agent_server_builds_as_it_is() {
        let features = serde_json::json!({ "default": ["kira"] });
        let p = Project::from_metadata(&game(features, serde_json::json!({})), None).unwrap();
        let mut release = p.build_args(false);
        assert_eq!(release.pop().as_deref(), Some("--locked"));
        assert_eq!(release, p.build_args(true));
    }
}
