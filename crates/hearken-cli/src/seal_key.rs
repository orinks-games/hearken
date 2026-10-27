//! The secret a game seals its data files with (`seal = true`).
//!
//! Unlike the pack key, which is new for every release, a game's seal key
//! stays the same for good: a release built with another key could not read
//! the saves the last one wrote. `hearken release` looks for it in
//! `HEARKEN_SEAL_SECRET` (for CI), then in a key file under the developer's
//! config folder, and makes that file the first time. The key never goes
//! into the game's repository; its ID (`seal-id`) does, so a release with
//! the wrong key, or none, is refused instead of shipped.

use std::io::Write;
use std::path::{Path, PathBuf};

use hearken_seal::Key;

use crate::project::Project;

/// Where CI passes the game's secret, as 64 hex digits.
pub const SECRET_VAR: &str = "HEARKEN_SEAL_SECRET";

/// The variable `hearken-seal`'s build script reads the key from. Named
/// apart from [`SECRET_VAR`] so a secret exported for CI does not make
/// every `cargo build` a sealed one.
pub const BUILD_VAR: &str = hearken_seal::KEY_VAR;

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

/// The game's key file: `<config dir>/hearken/seal-keys/<package>.key`.
pub fn key_file(package: &str) -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("hearken")
            .join("seal-keys")
            .join(format!("{package}.key")),
    )
}

/// The key a release of `project` is built with: `None` when the game does
/// not seal. Makes the key file the first time, when nothing says a key
/// should already exist, and says what to do with it.
pub fn release_key(project: &Project) -> Result<Option<Key>, String> {
    release_key_with(project, &Inputs::real(project), &mut std::io::stdout())
}

fn release_key_with(
    project: &Project,
    inputs: &Inputs,
    out: &mut dyn Write,
) -> Result<Option<Key>, String> {
    if !project.seal {
        return Ok(None);
    }
    if !inputs.lock.contains("name = \"hearken-seal\"") {
        return Err(
            "This game's Hearken is too old to seal its data. Update it with \
             `cargo update -p hearken`, or turn `seal` off."
                .to_string(),
        );
    }
    let key = match found_key(inputs)? {
        Some(key) => key,
        None if project.seal_id.is_some() => {
            return Err(format!(
                "This game has a seal-id, but its key was not found. Restore the key file \
                 from your backup{}, or set {SECRET_VAR}. A new key would make every \
                 player's saves unreadable, so none is made.",
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
    match &project.seal_id {
        Some(id) if *id != key.id() => {
            return Err(format!(
                "The key found is {}, but this game's seal-id is {id}. A release with another \
                 key could not read its players' saves.",
                key.id()
            ))
        }
        Some(_) => {}
        None => {
            let _ = writeln!(
                out,
                "Add this line to [package.metadata.hearken] in {}'s Cargo.toml and commit it,\n\
                 so a release with any other key is refused:\n\n    seal-id = \"{}\"\n",
                project.package,
                key.id()
            );
        }
    }
    Ok(Some(key))
}

/// The key from `HEARKEN_SEAL_SECRET`, else from the key file.
fn found_key(inputs: &Inputs) -> Result<Option<Key>, String> {
    if let Some(hex) = &inputs.secret {
        return Key::from_hex(hex)
            .map(Some)
            .ok_or_else(|| format!("{SECRET_VAR} must be 64 hexadecimal digits."));
    }
    let Some(path) = &inputs.key_file else {
        return Ok(None);
    };
    match std::fs::read_to_string(path) {
        Ok(text) => read_key_file(&text)
            .map(Some)
            .ok_or_else(|| format!("{} is not a seal key file.", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

/// The key in a key file: its one line that is not blank or a `#` comment.
fn read_key_file(text: &str) -> Option<Key> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let key = Key::from_hex(lines.next()?)?;
    lines.next().is_none().then_some(key)
}

/// Makes the game's key file and tells the developer to keep it safe.
fn new_key(project: &Project, inputs: &Inputs, out: &mut dyn Write) -> Result<Key, String> {
    let path = inputs
        .key_file
        .as_deref()
        .ok_or("There is no config folder to keep a seal key in; set HEARKEN_SEAL_SECRET.")?;
    let key = Key::generate().map_err(|e| format!("Could not make a seal key: {e}"))?;
    let text = format!(
        "# The seal key of {}: its saves and records are sealed with it.\n\
         # Keep a backup. Without it, no later release can read players' saves.\n\
         # Never commit it. Its ID, safe to commit, is {}.\n{}\n",
        project.package,
        key.id(),
        key.to_hex()
    );
    write_private(path, &text).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    let _ = writeln!(
        out,
        "Made a seal key for {}, in\n    {}\n\n\
         Back it up somewhere safe. If it is lost, players' saves stop loading in the next \
         release.\n\
         For CI, store its 64 hex digits (`hearken seal-key --print` shows them) as a secret,\n\
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

/// `hearken seal-key`: where the game's key is and its ID; with `print`,
/// its hex digits too, for a CI secret. Never makes a key.
pub fn command(project: &Project, print: bool) -> Result<(), String> {
    let inputs = Inputs::real(project);
    let Some(key) = found_key(&inputs)? else {
        return Err(format!(
            "No seal key for {} yet{}. `hearken release` makes one for a game with \
             seal = true.",
            project.package,
            shown(inputs.key_file.as_deref()).replacen(" to ", " at ", 1)
        ));
    };
    match (&inputs.secret, &inputs.key_file) {
        (Some(_), _) => println!("Key: from {SECRET_VAR}"),
        (None, Some(path)) => println!("Key file: {}", path.display()),
        (None, None) => {}
    }
    println!("ID: {}", key.id());
    match &project.seal_id {
        Some(id) if *id == key.id() => println!("It matches this game's seal-id."),
        Some(id) => println!("It does NOT match this game's seal-id, {id}."),
        None => println!("This game has no seal-id yet."),
    }
    if print {
        println!("Secret: {}", key.to_hex());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = "[[package]]\nname = \"hearken-seal\"\n";

    fn project(seal: bool, seal_id: Option<&str>) -> Project {
        Project {
            dir: PathBuf::from("."),
            target_dir: PathBuf::from("target"),
            package: "my-game".into(),
            bin: "my-game".into(),
            version: "0.1.0".into(),
            default_features: Vec::new(),
            portable: false,
            pack: false,
            seal,
            seal_id: seal_id.map(String::from),
            include: Vec::new(),
            rules: None,
        }
    }

    fn inputs(dir: &tempfile::TempDir) -> Inputs {
        Inputs {
            secret: None,
            ci: false,
            key_file: Some(dir.path().join("seal-keys").join("my-game.key")),
            lock: LOCK.into(),
        }
    }

    fn run(project: &Project, inputs: &Inputs) -> (Result<Option<Key>, String>, String) {
        let mut out = Vec::new();
        let key = release_key_with(project, inputs, &mut out);
        (key, String::from_utf8(out).unwrap())
    }

    #[test]
    fn a_game_that_does_not_seal_gets_no_key() {
        let dir = tempfile::tempdir().unwrap();
        let (key, out) = run(&project(false, None), &inputs(&dir));
        assert_eq!(key, Ok(None));
        assert!(out.is_empty());
    }

    #[test]
    fn the_first_release_makes_a_key_file_and_never_prints_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = inputs(&dir);
        let (key, out) = run(&project(true, None), &inputs);
        let key = key.unwrap().unwrap();
        let path = inputs.key_file.as_ref().unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(read_key_file(&text), Some(key.clone()));
        assert!(!out.contains(&key.to_hex()));
        assert!(out.contains(&format!("seal-id = \"{}\"", key.id())));
        assert!(out.contains("Back it up"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // The next release finds the same key, and makes no new one.
        let (again, out) = run(&project(true, Some(&key.id())), &inputs);
        assert_eq!(again, Ok(Some(key.clone())));
        assert!(!out.contains(&key.to_hex()));
    }

    #[test]
    fn the_ci_secret_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let key = Key::generate().unwrap();
        let mut inputs = inputs(&dir);
        inputs.secret = Some(format!("{}\n", key.to_hex()));
        inputs.ci = true;
        let (found, out) = run(&project(true, Some(&key.id())), &inputs);
        assert_eq!(found, Ok(Some(key.clone())));
        assert!(!out.contains(&key.to_hex()));
        assert!(!inputs.key_file.unwrap().exists());
        let mut bad = self::inputs(&dir);
        bad.secret = Some("not hex".into());
        assert!(run(&project(true, None), &bad).0.is_err());
    }

    #[test]
    fn a_missing_key_is_never_replaced_when_the_game_has_a_seal_id() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = inputs(&dir);
        let (key, _) = run(&project(true, Some("k1:0011223344556677")), &inputs);
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
    fn a_key_that_does_not_match_the_seal_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut inputs = inputs(&dir);
        let key = Key::generate().unwrap();
        inputs.secret = Some(key.to_hex());
        let other = Key::generate().unwrap().id();
        let (found, _) = run(&project(true, Some(&other)), &inputs);
        let err = found.unwrap_err();
        assert!(err.contains(&other) && err.contains(&key.id()));
        assert!(!err.contains(&key.to_hex()));
    }

    #[test]
    fn an_engine_without_hearken_seal_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut inputs = inputs(&dir);
        inputs.lock = "[[package]]\nname = \"hearken\"\n".into();
        let (key, _) = run(&project(true, None), &inputs);
        assert!(key.unwrap_err().contains("too old to seal"));
        assert!(!inputs.key_file.unwrap().exists());
    }

    #[test]
    fn a_key_file_holds_one_key_and_comments() {
        let key = Key::generate().unwrap();
        let text = format!("# a comment\n\n{}\n", key.to_hex());
        assert_eq!(read_key_file(&text), Some(key.clone()));
        assert_eq!(read_key_file("# nothing\n"), None);
        assert_eq!(
            read_key_file(&format!("{}\n{}\n", key.to_hex(), key.to_hex())),
            None
        );
    }
}
