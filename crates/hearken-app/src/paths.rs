//! Where a game keeps its settings and its session log.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    /// `<env_prefix>_DATA_DIR` when it is set and non-empty, otherwise
    /// `dir_name` under the platform's local data folder. Created if missing.
    pub fn discover(dir_name: &str, env_prefix: &str) -> Paths {
        let dir = std::env::var_os(format!("{env_prefix}_DATA_DIR"))
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
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
