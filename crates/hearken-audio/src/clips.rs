//! Loose sound files on disk, and an in-memory clip source for tests.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Something that turns a sound key into its file bytes.
pub trait ClipSource: Send {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>>;
}

/// Refuses a key that could escape the sounds folder: empty, rooted (`/` or
/// `\`), drive-letter-qualified (`:`), or containing a `..` component.
fn is_safe(key: &str) -> bool {
    if key.is_empty() || key.starts_with('/') || key.starts_with('\\') || key.contains(':') {
        return false;
    }
    !Path::new(key)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
}

/// Sound files loaded from a folder on disk, by key and extension.
///
/// Every load, including a miss, is cached for the run, so a key with no
/// file is not asked of the disk again.
pub struct LooseClips {
    root: Option<PathBuf>,
    cache: HashMap<String, Option<Arc<[u8]>>>,
}

impl LooseClips {
    /// Extensions tried, in order, for a key with no extension of its own.
    pub const EXTENSIONS: [&'static str; 3] = ["ogg", "wav", "mp3"];

    /// Loads clips from `root`.
    pub fn at(root: impl Into<PathBuf>) -> LooseClips {
        LooseClips {
            root: Some(root.into()),
            cache: HashMap::new(),
        }
    }

    /// Loads nothing: every key is a miss.
    pub fn none() -> LooseClips {
        LooseClips {
            root: None,
            cache: HashMap::new(),
        }
    }

    /// Picks a sounds folder: the first of `<env_prefix>_SOUNDS` (non-empty),
    /// the executable's own `sounds` folder, or the current directory's
    /// `sounds` folder, that exists as a directory; [`LooseClips::none`]
    /// otherwise.
    pub fn discover(env_prefix: &str) -> LooseClips {
        let mut candidates: Vec<(String, PathBuf)> = Vec::new();
        let var = format!("{env_prefix}_SOUNDS");
        if let Ok(dir) = std::env::var(&var) {
            if !dir.is_empty() {
                candidates.push((var, PathBuf::from(dir)));
            }
        }
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("sounds")))
        {
            candidates.push(("the executable's folder".to_string(), dir));
        }
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(("the current directory".to_string(), cwd.join("sounds")));
        }
        for (label, path) in candidates {
            if path.is_dir() {
                log::info!("sounds: using {label} ({})", path.display());
                return LooseClips::at(path);
            }
        }
        log::info!("sounds: no sounds folder found; playing built-in defaults");
        LooseClips::none()
    }

    /// The folder clips are loaded from, if any.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}

impl ClipSource for LooseClips {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>> {
        if let Some(cached) = self.cache.get(key) {
            return cached.clone();
        }
        let found = self.root.as_ref().and_then(|root| {
            if !is_safe(key) {
                log::warn!("sound key {key:?} is not a safe path; ignoring");
                return None;
            }
            LooseClips::EXTENSIONS.iter().find_map(|ext| {
                std::fs::read(root.join(format!("{key}.{ext}")))
                    .ok()
                    .map(Arc::from)
            })
        });
        self.cache.insert(key.to_string(), found.clone());
        found
    }
}

/// Clips held in memory, for tests.
#[derive(Default)]
pub struct MemoryClips {
    clips: HashMap<String, Arc<[u8]>>,
}

impl MemoryClips {
    /// Adds a clip, returning `self` for chaining.
    pub fn with(mut self, key: &str, bytes: impl Into<Arc<[u8]>>) -> MemoryClips {
        self.clips.insert(key.to_string(), bytes.into());
        self
    }
}

impl ClipSource for MemoryClips {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>> {
        self.clips.get(key).cloned()
    }
}
