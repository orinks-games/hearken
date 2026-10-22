//! Where a game keeps its settings and its session log.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    /// `<env_prefix>_DATA_DIR` when it is set and non-empty; then, for a
    /// portable copy, `saves` beside the executable; otherwise `dir_name`
    /// under the platform's local data folder. Created if missing.
    pub fn discover(dir_name: &str, env_prefix: &str) -> Paths {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let dir = std::env::var_os(format!("{env_prefix}_DATA_DIR"))
            .filter(|v| !v.is_empty())
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
}
