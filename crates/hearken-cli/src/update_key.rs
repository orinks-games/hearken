//! The key a game signs its update manifests with (`update = true`).
//!
//! Like the seal key, it stays the same for good: every release a player
//! has trusts only manifests signed with it. `hearken release` looks for it
//! in `HEARKEN_UPDATE_SECRET` (for CI), then in a key file under the
//! developer's config folder, and makes that file the first time. The
//! secret never goes into the game's repository. Its public half does, as
//! `update-key`, which releases are built with and checked against, so a
//! release signed with another key, or none, is refused instead of shipped.

use std::io::Write;
use std::path::{Path, PathBuf};

use hearken_update::{PublicKey, SecretKey};

use crate::project::Project;

/// Where CI passes the game's update secret, as 64 hex digits.
pub const SECRET_VAR: &str = "HEARKEN_UPDATE_SECRET";

/// The variables `hearken-update`'s build script reads. The public key and
/// what the release is, never the secret.
pub const KEY_VAR: &str = "HEARKEN_UPDATE_KEY";
pub const GAME_VAR: &str = "HEARKEN_UPDATE_GAME";
pub const VERSION_VAR: &str = "HEARKEN_UPDATE_VERSION";
pub const CHANNEL_VAR: &str = "HEARKEN_UPDATE_CHANNEL";

/// Everything `release_key` reads from the world, so tests can stand in.
struct Inputs {
    secret: Option<String>,
    ci: bool,
    key_file: Option<PathBuf>,
    lock: String,
}

impl Inputs {
    fn real(project: &Project) -> Inputs {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Inputs {
            secret: var(SECRET_VAR),
            ci: var("CI").is_some_and(|v| v != "false" && v != "0"),
            key_file: key_file(&project.package),
            lock: std::fs::read_to_string(project.dir.join("Cargo.lock")).unwrap_or_default(),
        }
    }
}

/// The game's key file: `<config dir>/hearken/update-keys/<package>.key`.
pub fn key_file(package: &str) -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("hearken")
            .join("update-keys")
            .join(format!("{package}.key")),
    )
}

/// The key a release of `project` signs its manifest with: `None` when the
/// game does not update itself. Makes the key file the first time, when
/// nothing says a key should already exist, and says what to do with it.
pub fn release_key(project: &Project) -> Result<Option<SecretKey>, String> {
    release_key_with(project, &Inputs::real(project), &mut std::io::stdout())
}

fn committed(project: &Project) -> Result<Option<PublicKey>, String> {
    match &project.update_key {
        None => Ok(None),
        Some(text) => PublicKey::parse(text).map(Some).ok_or_else(|| {
            format!("update-key {text:?} is not a public key: it is ed25519: and 64 hex digits.")
        }),
    }
}

fn release_key_with(
    project: &Project,
    inputs: &Inputs,
    out: &mut dyn Write,
) -> Result<Option<SecretKey>, String> {
    if !project.update {
        return Ok(None);
    }
    if !inputs.lock.contains("name = \"hearken-update\"") {
        return Err(
            "This game's Hearken is too old to update itself. Update it with \
             `cargo update -p hearken` and turn on its `update` feature, or turn `update` off."
                .to_string(),
        );
    }
    let expected = committed(project)?;
    let key = match found_key(inputs)? {
        Some(key) => key,
        None if expected.is_some() => {
            return Err(format!(
                "This game has an update-key, but its secret was not found. Restore the key \
                 file from your backup{}, or set {SECRET_VAR}. Players' copies trust only that \
                 key, so a new one would never reach them; none is made.",
                shown(inputs.key_file.as_deref())
            ))
        }
        None if inputs.ci => {
            return Err(format!(
                "{SECRET_VAR} is not set. Keys are never made on CI: run `hearken release` \
                 once on your own computer, then store the key as a CI secret."
            ))
        }
        None => new_key(project, inputs, out)?,
    };
    match expected {
        Some(public) if public != key.public() => {
            return Err(format!(
                "The update secret found is for {}, but this game's update-key is {public}. \
                 Players' copies would refuse every update it signed.",
                key.public()
            ))
        }
        Some(_) => {}
        None => {
            let _ = writeln!(
                out,
                "Add this line to [package.metadata.hearken] in {}'s Cargo.toml and commit it,\n\
                 so a release signed with any other key is refused:\n\n    update-key = \"{}\"\n",
                project.package,
                key.public()
            );
        }
    }
    Ok(Some(key))
}

/// The key from `HEARKEN_UPDATE_SECRET`, else from the key file.
fn found_key(inputs: &Inputs) -> Result<Option<SecretKey>, String> {
    if let Some(hex) = &inputs.secret {
        return SecretKey::from_hex(hex)
            .map(Some)
            .ok_or_else(|| format!("{SECRET_VAR} must be 64 hexadecimal digits."));
    }
    let Some(path) = &inputs.key_file else {
        return Ok(None);
    };
    match std::fs::read_to_string(path) {
        Ok(text) => read_key_file(&text)
            .map(Some)
            .ok_or_else(|| format!("{} is not an update key file.", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

/// The key in a key file: its one line that is not blank or a `#` comment.
fn read_key_file(text: &str) -> Option<SecretKey> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let key = SecretKey::from_hex(lines.next()?)?;
    lines.next().is_none().then_some(key)
}

/// Makes the game's key file and tells the developer to keep it safe.
fn new_key(project: &Project, inputs: &Inputs, out: &mut dyn Write) -> Result<SecretKey, String> {
    let path = inputs.key_file.as_deref().ok_or_else(|| {
        format!("There is no config folder to keep an update key in; set {SECRET_VAR}.")
    })?;
    let key = SecretKey::generate().map_err(|e| format!("Could not make an update key: {e}"))?;
    let text = format!(
        "# The update key of {}: its update manifests are signed with it.\n\
         # Keep a backup. Without it, players' copies accept no later update.\n\
         # Never commit it. Its public half, safe to commit, is {}.\n{}\n",
        project.package,
        key.public(),
        key.to_hex()
    );
    write_private(path, &text).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    let _ = writeln!(
        out,
        "Made an update key for {}, in\n    {}\n\n\
         Back it up somewhere safe. If it is lost, players' copies will not take updates \
         from any later release.\n\
         For CI, store its 64 hex digits (`hearken update-key --print` shows them) as a secret,\n\
         and pass it to `hearken release` as {SECRET_VAR}.\n",
        project.package,
        path.display()
    );
    Ok(key)
}

/// Writes a new file only its owner can read. Never overwrites one.
fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

fn shown(path: Option<&Path>) -> String {
    path.map(|p| format!(" to {}", p.display()))
        .unwrap_or_default()
}

/// `hearken update-key`: where the game's key is and its public half; with
/// `print`, the secret's hex digits too, for a CI secret. Never makes a key.
pub fn command(project: &Project, print: bool) -> Result<(), String> {
    let inputs = Inputs::real(project);
    let Some(key) = found_key(&inputs)? else {
        return Err(format!(
            "No update key for {} yet{}. `hearken release` makes one for a game with \
             update = true.",
            project.package,
            shown(inputs.key_file.as_deref()).replacen(" to ", " at ", 1)
        ));
    };
    match (&inputs.secret, &inputs.key_file) {
        (Some(_), _) => println!("Key: from {SECRET_VAR}"),
        (None, Some(path)) => println!("Key file: {}", path.display()),
        (None, None) => {}
    }
    println!("Public key: {}", key.public());
    match committed(project)? {
        Some(public) if public == key.public() => println!("It matches this game's update-key."),
        Some(public) => println!("It does NOT match this game's update-key, {public}."),
        None => println!("This game has no update-key yet."),
    }
    if print {
        println!("Secret: {}", key.to_hex());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = "[[package]]\nname = \"hearken-update\"\n";

    fn project(update: bool, update_key: Option<String>) -> Project {
        let mut project = Project::for_tests("my-game");
        project.update = update;
        project.update_key = update_key;
        project
    }

    fn inputs(dir: &tempfile::TempDir) -> Inputs {
        Inputs {
            secret: None,
            ci: false,
            key_file: Some(dir.path().join("update-keys").join("my-game.key")),
            lock: LOCK.into(),
        }
    }

    fn run(project: &Project, inputs: &Inputs) -> (Result<Option<SecretKey>, String>, String) {
        let mut out = Vec::new();
        let key = release_key_with(project, inputs, &mut out);
        (key, String::from_utf8(out).unwrap())
    }

    #[test]
    fn a_game_that_does_not_update_gets_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let (key, out) = run(&project(false, None), &inputs(&dir));
        assert_eq!(key, Ok(None));
        assert!(out.is_empty());
    }

    #[test]
    fn the_first_release_makes_a_key_file_and_never_prints_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = inputs(&dir);
        let (key, out) = run(&project(true, None), &inputs);
        let key = key.unwrap().unwrap();
        let path = inputs.key_file.as_ref().unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(read_key_file(&text), Some(key.clone()));
        assert!(!out.contains(&key.to_hex()));
        assert!(out.contains(&format!("update-key = \"{}\"", key.public())));
        assert!(out.contains("Back it up"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // The next release finds the same key, and makes no new one.
        let (again, _) = run(&project(true, Some(key.public().to_string())), &inputs);
        assert_eq!(again, Ok(Some(key)));
    }

    #[test]
    fn the_ci_secret_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let key = SecretKey::generate().unwrap();
        let mut inputs = inputs(&dir);
        inputs.secret = Some(format!("{}\n", key.to_hex()));
        inputs.ci = true;
        let (found, out) = run(&project(true, Some(key.public().to_string())), &inputs);
        assert_eq!(found, Ok(Some(key.clone())));
        assert!(!out.contains(&key.to_hex()));
        assert!(!inputs.key_file.unwrap().exists());
        let mut bad = self::inputs(&dir);
        bad.secret = Some("not hex".into());
        assert!(run(&project(true, None), &bad).0.is_err());
    }

    #[test]
    fn a_missing_secret_is_never_replaced_when_the_game_has_an_update_key() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = inputs(&dir);
        let public = SecretKey::generate().unwrap().public().to_string();
        let (key, _) = run(&project(true, Some(public)), &inputs);
        assert!(key.unwrap_err().contains("not found"));
        assert!(!inputs.key_file.unwrap().exists());
    }

    #[test]
    fn ci_never_makes_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut inputs = inputs(&dir);
        inputs.ci = true;
        let (key, _) = run(&project(true, None), &inputs);
        assert!(key.unwrap_err().contains("never made on CI"));
        assert!(!inputs.key_file.unwrap().exists());
    }

    #[test]
    fn a_secret_that_does_not_match_the_update_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut inputs = inputs(&dir);
        let key = SecretKey::generate().unwrap();
        inputs.secret = Some(key.to_hex());
        let other = SecretKey::generate().unwrap().public().to_string();
        let (found, _) = run(&project(true, Some(other.clone())), &inputs);
        let err = found.unwrap_err();
        assert!(err.contains(&other) && err.contains(&key.public().to_string()));
        assert!(!err.contains(&key.to_hex()));
    }

    #[test]
    fn a_committed_key_that_is_not_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (key, _) = run(&project(true, Some("ed25519:abc".into())), &inputs(&dir));
        assert!(key.unwrap_err().contains("not a public key"));
    }

    #[test]
    fn an_engine_without_hearken_update_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut inputs = inputs(&dir);
        inputs.lock = "[[package]]\nname = \"hearken\"\n".into();
        let (key, _) = run(&project(true, None), &inputs);
        assert!(key.unwrap_err().contains("too old to update itself"));
        assert!(!inputs.key_file.unwrap().exists());
    }
}
