//! Developer ID signing and notarization of a macOS release, when the
//! signing variables are set. Without them a release is left as the linker
//! signed it (ad hoc), and Gatekeeper asks players to allow it by hand.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The codesigning identity, e.g. `Developer ID Application: Name (TEAMID)`.
const SIGN_IDENTITY_ENV: &str = "MACOS_SIGN_IDENTITY";
/// The keychain that holds the identity; the default search list if unset.
const SIGN_KEYCHAIN_ENV: &str = "MACOS_SIGN_KEYCHAIN";
/// An App Store Connect API key: its `.p8` file, key ID and issuer ID.
const NOTARY_ENV: [&str; 3] = [
    "MACOS_NOTARY_KEY_PATH",
    "MACOS_NOTARY_KEY_ID",
    "MACOS_NOTARY_ISSUER_ID",
];

fn env_trimmed(name: &str) -> String {
    std::env::var(name).unwrap_or_default().trim().to_string()
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("Could not run {:?}: {e}", cmd.get_program()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed ({status})."))
    }
}

/// The notary variables, all set or all unset; half set is an error.
fn notary_values(get: impl Fn(&str) -> String) -> Result<Option<[String; 3]>, String> {
    let values = NOTARY_ENV.map(&get);
    if values.iter().all(String::is_empty) {
        return Ok(None);
    }
    let missing: Vec<&str> = NOTARY_ENV
        .iter()
        .zip(&values)
        .filter(|(_, v)| v.is_empty())
        .map(|(n, _)| *n)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Notarization is half set up; missing {}.",
            missing.join(", ")
        ));
    }
    Ok(Some(values))
}

/// Checks the variables before the build, so a half set up release fails
/// before minutes of compiling rather than after.
pub fn check() -> Result<(), String> {
    if notary_values(env_trimmed)?.is_some() && env_trimmed(SIGN_IDENTITY_ENV).is_empty() {
        return Err(format!(
            "Notarization needs a Developer ID signature; set {SIGN_IDENTITY_ENV}."
        ));
    }
    Ok(())
}

/// Every library in the staged folder, signed before the executable.
fn libraries(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            libraries(&path, found);
        } else if path.extension().is_some_and(|e| e == "dylib") {
            found.push(path);
        }
    }
}

fn codesign(identity: &str, keychain: &str, target: &Path) -> Command {
    let mut cmd = Command::new("codesign");
    // Hardened runtime and a secure timestamp: notarization requires both.
    cmd.args(["--force", "--options", "runtime", "--timestamp"]);
    if !keychain.is_empty() {
        cmd.args(["--keychain", keychain]);
    }
    cmd.args(["--sign", identity]).arg(target);
    cmd
}

/// Signs the staged executable and its libraries with the Developer ID
/// identity in `MACOS_SIGN_IDENTITY`. Returns whether it signed.
pub fn sign(stage: &Path, exe: &Path) -> Result<bool, String> {
    let identity = env_trimmed(SIGN_IDENTITY_ENV);
    if identity.is_empty() {
        println!("Not signing for macOS: {SIGN_IDENTITY_ENV} is not set.");
        return Ok(false);
    }
    let keychain = env_trimmed(SIGN_KEYCHAIN_ENV);
    let mut libs = Vec::new();
    libraries(stage, &mut libs);
    for lib in &libs {
        run(&mut codesign(&identity, &keychain, lib))?;
    }
    run(&mut codesign(&identity, &keychain, exe))?;
    run(Command::new("codesign")
        .args(["--verify", "--strict", "--verbose=2"])
        .arg(exe))?;
    println!("Signed as {identity}.");
    Ok(true)
}

/// Sends the archive to Apple's notary service and waits for its verdict.
/// A bare executable cannot be stapled, so Gatekeeper looks the ticket up
/// online the first time a player opens the game.
pub fn notarize(archive: &Path) -> Result<(), String> {
    let Some([key_path, key_id, issuer]) = notary_values(env_trimmed)? else {
        println!("Not notarizing: no App Store Connect key is set.");
        return Ok(());
    };
    let auth = [
        "--key",
        key_path.as_str(),
        "--key-id",
        key_id.as_str(),
        "--issuer",
        issuer.as_str(),
    ];
    println!("Notarizing {}...", archive.display());
    let result = Command::new("xcrun")
        .args(["notarytool", "submit"])
        .arg(archive)
        .args(auth)
        .args(["--wait", "--timeout", "45m", "--output-format", "json"])
        .output()
        .map_err(|e| format!("Could not run xcrun notarytool: {e}"))?;
    let submission: serde_json::Value =
        serde_json::from_slice(&result.stdout).unwrap_or(serde_json::Value::Null);
    let field = |k: &str| submission[k].as_str().unwrap_or_default().to_string();
    let (status, id) = (field("status"), field("id"));
    if status != "Accepted" {
        if !id.is_empty() {
            // Apple's log says which file it rejected, and why.
            let _ = Command::new("xcrun")
                .args(["notarytool", "log", &id])
                .args(auth)
                .status();
        }
        return Err(format!(
            "Apple's notary service returned {}: {}",
            if status.is_empty() {
                "no verdict"
            } else {
                &status
            },
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    println!("Notarization accepted: {id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notary_variables_are_all_or_none() {
        assert_eq!(notary_values(|_| String::new()), Ok(None));
        let all = notary_values(|n| format!("{n}-value")).unwrap().unwrap();
        assert_eq!(all[1], "MACOS_NOTARY_KEY_ID-value");
        let half = notary_values(|n| {
            if n == "MACOS_NOTARY_KEY_ID" {
                String::new()
            } else {
                "x".into()
            }
        });
        assert_eq!(
            half,
            Err("Notarization is half set up; missing MACOS_NOTARY_KEY_ID.".into())
        );
    }
}
