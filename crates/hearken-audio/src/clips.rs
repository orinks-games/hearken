//! Loose sound files on disk, a release's sound pack, an in-memory clip
//! source, and a layering of sources so a game's built-in sounds give way to
//! files of the same key.

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
    /// Extensions appended to every key, in order, when looking for its file.
    pub const EXTENSIONS: [&'static str; 5] = hearken_pack::EXTENSIONS;

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
        LooseClips::from_env(env_prefix).unwrap_or_else(LooseClips::beside_game)
    }

    /// The folder `<env_prefix>_SOUNDS` names, when it is set to one.
    fn from_env(env_prefix: &str) -> Option<LooseClips> {
        let var = format!("{env_prefix}_SOUNDS");
        let dir = std::env::var(&var).ok().filter(|d| !d.is_empty())?;
        let path = PathBuf::from(&dir);
        if path.is_dir() {
            log::info!("sounds: using {var} ({})", path.display());
            Some(LooseClips::at(path))
        } else {
            log::warn!("sounds: {var} is set to {dir:?} but that is not a directory; ignoring");
            None
        }
    }

    /// The executable's `sounds` folder, else the current directory's.
    fn beside_game() -> LooseClips {
        LooseClips::beside(exe_dir().as_deref())
    }

    /// `exe_dir`'s `sounds` folder, else the current directory's.
    fn beside(exe_dir: Option<&Path>) -> LooseClips {
        let mut candidates: Vec<(String, PathBuf)> = Vec::new();
        if let Some(dir) = exe_dir.map(|d| d.join("sounds")) {
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

/// The folder the running program is in.
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// A release's sounds, from the encrypted `sounds.pak` that `hearken
/// release` writes beside the game (see the `hearken-pack` crate).
///
/// Loads are cached for the run, misses included, as [`LooseClips`] does,
/// but the cache holds each sound still encrypted: a sound is decrypted,
/// and checked, for each play and sits in memory readable only while it is
/// in use. A sound whose bytes were changed plays nothing, and is logged
/// once.
pub struct PackedClips {
    pack: hearken_pack::Pack,
    cache: HashMap<String, Option<hearken_pack::Sealed>>,
}

impl PackedClips {
    /// Opens the pack at `path` with `key`.
    pub fn open(path: &Path, key: &[u8; 32]) -> std::io::Result<PackedClips> {
        Ok(PackedClips {
            pack: hearken_pack::Pack::open(path, key)?,
            cache: HashMap::new(),
        })
    }

    /// The `sounds.pak` beside the executable, when this build was made by
    /// `hearken release` with the key it was packed with.
    pub fn beside_game() -> Option<PackedClips> {
        let key = hearken_pack::build_key()?;
        PackedClips::in_dir(&exe_dir()?, &key).ok()
    }

    /// The `sounds.pak` in `dir`, logged either way.
    fn in_dir(dir: &Path, key: &[u8; 32]) -> std::io::Result<PackedClips> {
        let path = dir.join(hearken_pack::FILE_NAME);
        match PackedClips::open(&path, key) {
            Ok(clips) => {
                log::info!(
                    "sounds: using the sound pack ({}, {} sounds)",
                    path.display(),
                    clips.pack.len()
                );
                Ok(clips)
            }
            Err(e) => {
                log::error!("sounds: could not open {}: {e}", path.display());
                Err(e)
            }
        }
    }
}

impl ClipSource for PackedClips {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>> {
        if !self.cache.contains_key(key) {
            let sealed = match self.pack.read_sealed(&key.replace('\\', "/")) {
                Ok(sealed) => sealed,
                Err(e) => {
                    log::warn!("sounds: could not read {key:?} from the sound pack: {e}");
                    None
                }
            };
            self.cache.insert(key.to_string(), sealed);
        }
        let sealed = self.cache.get(key)?.as_ref()?;
        match self.pack.unseal(sealed) {
            Ok(bytes) => Some(Arc::from(bytes)),
            Err(e) => {
                log::error!("sounds: {e}; it will not play");
                self.cache.insert(key.to_string(), None);
                None
            }
        }
    }
}

/// What a packed build says when its `sounds.pak` is missing or damaged.
pub const DAMAGED_PACK_NOTICE: &str = "This copy of the game's sounds is missing or damaged, \
so only its built-in sounds will play. Reinstall the game to fix it.";

/// Where a game's sounds come from, and what the player should hear if
/// they are broken.
pub struct Discovered {
    pub clips: Box<dyn ClipSource>,
    /// Set when a packed build's `sounds.pak` is missing or damaged.
    pub notice: Option<String>,
}

/// Where a game's sounds come from. A build made by `hearken release` with
/// a sound pack reads its `sounds.pak` beside the executable and nothing
/// else: no `<env_prefix>_SOUNDS`, no loose `sounds` folder, so a player
/// cannot swap the game's sounds. Every other build reads the folder
/// `<env_prefix>_SOUNDS` names, else the loose `sounds` folder
/// [`LooseClips::discover`] finds.
pub fn discover_clips(env_prefix: &str) -> Discovered {
    discover_clips_from(env_prefix, hearken_pack::build_key(), exe_dir().as_deref())
}

/// [`discover_clips`] with every input explicit, for tests.
pub fn discover_clips_from(
    env_prefix: &str,
    pack_key: Option<[u8; 32]>,
    exe_dir: Option<&Path>,
) -> Discovered {
    let Some(key) = pack_key else {
        let clips: Box<dyn ClipSource> = match LooseClips::from_env(env_prefix) {
            Some(loose) => Box::new(loose),
            None => Box::new(LooseClips::beside(exe_dir)),
        };
        return Discovered {
            clips,
            notice: None,
        };
    };
    match exe_dir.map(|dir| PackedClips::in_dir(dir, &key)) {
        Some(Ok(packed)) => Discovered {
            clips: Box::new(packed),
            notice: None,
        },
        _ => Discovered {
            clips: Box::new(LooseClips::none()),
            notice: Some(DAMAGED_PACK_NOTICE.to_string()),
        },
    }
}

impl ClipSource for Box<dyn ClipSource> {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>> {
        (**self).load(key)
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

/// Several sources asked in order; the first that has a key wins.
///
/// The engine puts the loose sounds folder first and the game's built-in
/// clips after it, so dropping a recording named for a key into the folder
/// is all it takes to replace that sound.
pub struct LayeredClips {
    layers: Vec<Box<dyn ClipSource>>,
}

impl LayeredClips {
    pub fn new(layers: Vec<Box<dyn ClipSource>>) -> LayeredClips {
        LayeredClips { layers }
    }
}

impl ClipSource for LayeredClips {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>> {
        self.layers.iter_mut().find_map(|layer| layer.load(key))
    }
}

/// Clips a function makes on first request, such as synthesized
/// placeholders, cached for the run (misses included).
type Make = Box<dyn FnMut(&str) -> Option<Vec<u8>> + Send>;

pub struct MadeClips {
    make: Make,
    cache: HashMap<String, Option<Arc<[u8]>>>,
}

impl MadeClips {
    pub fn new(make: impl FnMut(&str) -> Option<Vec<u8>> + Send + 'static) -> MadeClips {
        MadeClips {
            make: Box::new(make),
            cache: HashMap::new(),
        }
    }
}

impl ClipSource for MadeClips {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>> {
        if let Some(cached) = self.cache.get(key) {
            return cached.clone();
        }
        let made = (self.make)(key).map(Arc::from);
        self.cache.insert(key.to_string(), made.clone());
        made
    }
}
