//! Where a game keeps its settings, its data and its session log.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    /// Where a sealing release leaves its witnesses (see
    /// [`hearken_seal::Vault::with_witnesses`]): outside `data_dir`, so that
    /// emptying the data folder, or switching to a fresh one, does not make
    /// the game forget it has sealed before. Empty for [`Paths::at`].
    pub witnesses: Vec<PathBuf>,
}

impl Paths {
    /// `<env_prefix>_DATA_DIR` when it is set and non-empty; then, for a
    /// portable copy, `saves` beside the executable; otherwise `dir_name`
    /// under the platform's local data folder. Created if missing.
    ///
    /// A release that seals its data (`seal = true`) ignores
    /// `<env_prefix>_DATA_DIR`: it is for development and tests, and would
    /// otherwise let a player point the game at a folder of their own.
    pub fn discover(dir_name: &str, env_prefix: &str) -> Paths {
        Paths::discover_with(dir_name, env_prefix, hearken_seal::build_key().is_some())
    }

    /// [`Paths::discover`], told whether this build seals.
    pub fn discover_with(dir_name: &str, env_prefix: &str, sealing: bool) -> Paths {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let var = format!("{env_prefix}_DATA_DIR");
        let wanted = std::env::var_os(&var).filter(|v| !v.is_empty());
        if sealing && wanted.is_some() {
            log::warn!("{var} is ignored: this release seals its data");
        }
        let dir = wanted
            .filter(|_| !sealing)
            .map(PathBuf::from)
            .or_else(|| exe_dir.as_deref().and_then(portable_dir))
            .or_else(|| dirs::data_local_dir().map(|d| d.join(dir_name)))
            .unwrap_or_else(|| PathBuf::from(dir_name));
        let mut paths = Paths::at(dir);
        paths.witnesses = witnesses(
            dir_name,
            exe_dir.as_deref(),
            dirs::config_dir().as_deref(),
            &paths.data_dir,
        );
        paths
    }

    /// A folder for a separate session (a sandbox) inside this one's data
    /// folder, keeping this one's witnesses.
    pub fn nested(&self, name: &str) -> Paths {
        let mut paths = Paths::at(self.data_dir.join(name));
        paths.witnesses = self.witnesses.clone();
        paths
    }

    /// A data folder at `dir`, created if missing. A folder that cannot be
    /// created is logged, not fatal: saving later reports its own error.
    pub fn at(dir: impl Into<PathBuf>) -> Paths {
        let data_dir = dir.into();
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            log::warn!("could not create {}: {e}", data_dir.display());
        }
        Paths {
            data_dir,
            witnesses: Vec::new(),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("session.log")
    }
}

/// The marker a portable copy carries beside its executable. `hearken
/// release` adds it for a game that sets `portable = true`.
pub const PORTABLE_MARKER: &str = "portable.txt";

/// `exe_dir/saves` when `exe_dir` holds the portable marker.
pub fn portable_dir(exe_dir: &Path) -> Option<PathBuf> {
    exe_dir
        .join(PORTABLE_MARKER)
        .is_file()
        .then(|| exe_dir.join("saves"))
}

/// The witness file names: one in the platform's config folder, named for
/// the game, and, for a portable copy, one beside the executable. Any that
/// would fall inside `data_dir` is left out: it would go with the folder.
fn witnesses(
    dir_name: &str,
    exe_dir: Option<&Path>,
    config_dir: Option<&Path>,
    data_dir: &Path,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(config) = config_dir {
        out.push(config.join(format!("{dir_name}.hearken-seal")));
    }
    if let Some(exe_dir) = exe_dir {
        if portable_dir(exe_dir).is_some() {
            out.push(exe_dir.join("hearken-seal.witness"));
        }
    }
    out.retain(|w| !w.starts_with(data_dir));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portable_copy_saves_beside_itself() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(portable_dir(dir.path()), None);
        std::fs::write(dir.path().join(PORTABLE_MARKER), "").unwrap();
        assert_eq!(portable_dir(dir.path()), Some(dir.path().join("saves")));
    }

    #[test]
    fn witnesses_stay_outside_the_data_folder() {
        let exe = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let data = exe.path().join("saves");
        let installed = witnesses("Game", Some(exe.path()), Some(config.path()), &data);
        assert_eq!(installed, vec![config.path().join("Game.hearken-seal")]);
        std::fs::write(exe.path().join(PORTABLE_MARKER), "").unwrap();
        let portable = witnesses("Game", Some(exe.path()), Some(config.path()), &data);
        assert_eq!(
            portable,
            vec![
                config.path().join("Game.hearken-seal"),
                exe.path().join("hearken-seal.witness"),
            ]
        );
        // A data folder that holds the config folder keeps no witness there.
        let inside = witnesses("Game", None, Some(config.path()), config.path());
        assert!(inside.is_empty());
        assert!(Paths::at(data).witnesses.is_empty());
    }

    #[test]
    fn a_sealing_build_ignores_the_data_dir_override() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("crafted");
        std::env::set_var("HEARKEN_TEST_SEALED_PATHS_DATA_DIR", &wanted);
        let open =
            Paths::discover_with("HearkenSealedPathsTest", "HEARKEN_TEST_SEALED_PATHS", false);
        let sealed =
            Paths::discover_with("HearkenSealedPathsTest", "HEARKEN_TEST_SEALED_PATHS", true);
        std::env::remove_var("HEARKEN_TEST_SEALED_PATHS_DATA_DIR");
        assert_eq!(open.data_dir, wanted);
        assert_ne!(sealed.data_dir, wanted);
        let _ = std::fs::remove_dir(&sealed.data_dir);
    }
}
