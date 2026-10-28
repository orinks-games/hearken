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
    /// Where this game would leave its witness if it ran the other way
    /// (installed rather than portable, or the reverse): checked, never
    /// written (see [`hearken_seal::Vault::checking_witnesses`]). So adding
    /// or deleting `portable.txt`, which moves the data folder, does not
    /// also make the game forget it has sealed. Empty for [`Paths::at`].
    pub checked_witnesses: Vec<PathBuf>,
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
        (paths.witnesses, paths.checked_witnesses) = witnesses(
            dir_name,
            exe_dir.as_deref(),
            dirs::config_dir().as_deref(),
            &paths.data_dir,
        );
        paths
    }

    /// A folder for a separate session (a sandbox) inside this one's data
    /// folder. It has no witnesses: a witness lists names for one folder, and
    /// sharing it would let the sandbox's saves decide the real folder's.
    pub fn nested(&self, name: &str) -> Paths {
        Paths::at(self.data_dir.join(name))
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
            checked_witnesses: Vec::new(),
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

/// The witnesses to write, then those only to check. A portable copy writes
/// one beside the executable, so it leaves nothing in the player's user
/// folders; an installed one writes one in the platform's config folder,
/// named for the game. Each checks the other's, so adding or removing
/// `portable.txt` does not lose the record. One that would fall inside
/// `data_dir` is left out: it would go with the folder.
fn witnesses(
    dir_name: &str,
    exe_dir: Option<&Path>,
    config_dir: Option<&Path>,
    data_dir: &Path,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let beside_exe: Vec<PathBuf> = exe_dir
        .map(|dir| dir.join(format!("{dir_name}.hearken-seal")))
        .into_iter()
        .collect();
    let in_config: Vec<PathBuf> = config_dir
        .map(|dir| dir.join(format!("{dir_name}.hearken-seal")))
        .into_iter()
        .collect();
    let portable = exe_dir.is_some_and(|dir| portable_dir(dir).is_some());
    let (mut written, mut checked) = if portable {
        (beside_exe, in_config)
    } else {
        (in_config, beside_exe)
    };
    written.retain(|w| !w.starts_with(data_dir));
    checked.retain(|w| !w.starts_with(data_dir));
    (written, checked)
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
        let config_witness = config.path().join("Game.hearken-seal");
        let exe_witness = exe.path().join("Game.hearken-seal");
        let installed = witnesses("Game", Some(exe.path()), Some(config.path()), &data);
        assert_eq!(
            installed,
            (vec![config_witness.clone()], vec![exe_witness.clone()])
        );
        std::fs::write(exe.path().join(PORTABLE_MARKER), "").unwrap();
        let portable = witnesses("Game", Some(exe.path()), Some(config.path()), &data);
        assert_eq!(portable, (vec![exe_witness], vec![config_witness]));
        // A data folder that holds the config folder keeps no witness there.
        let inside = witnesses("Game", None, Some(config.path()), config.path());
        assert_eq!(inside, (Vec::new(), Vec::new()));
        let at = Paths::at(data);
        assert!(at.witnesses.is_empty() && at.checked_witnesses.is_empty());
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
