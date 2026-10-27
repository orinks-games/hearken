//! Where a game keeps its settings, its data and its session log.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
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
        Paths::at(dir)
    }

    /// A data folder at `dir`, created if missing. A folder that cannot be
    /// created is logged, not fatal: saving later reports its own error.
    pub fn at(dir: impl Into<PathBuf>) -> Paths {
        let data_dir = dir.into();
        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            log::warn!("could not create {}: {e}", data_dir.display());
        }
        Paths { data_dir }
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
