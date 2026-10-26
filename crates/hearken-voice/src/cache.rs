//! Synthesized lines kept for reuse: in memory for this run, and as WAV
//! files under `voice-cache/` for the next.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Changes whenever a cached file could sound different for the same text:
/// a new model package or a new local-tts.
const ENGINE_TAG: &str = "kokoro-en-q8/local-tts-61efef0";

/// 64-bit FNV-1a: stable across runs and Rust versions, unlike std's hasher.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// What the cache key depends on besides the line: the engine and the
/// lexicon's contents, so editing a pronunciation re-synthesizes.
pub(crate) fn salt(lexicon: Option<&Path>) -> u64 {
    let mut bytes = ENGINE_TAG.as_bytes().to_vec();
    if let Some(text) = lexicon.and_then(|p| std::fs::read(p).ok()) {
        bytes.extend_from_slice(&text);
    }
    fnv1a(&bytes)
}

/// The rate as cached: whole percent, so 1.0 and 1.0000001 share a file.
pub(crate) fn rate_percent(rate: f32) -> u32 {
    if rate.is_finite() {
        (rate.clamp(0.5, 2.0) * 100.0).round() as u32
    } else {
        100
    }
}

/// The key of one line in one voice at one rate.
pub(crate) fn key(salt: u64, voice: &str, rate: f32, text: &str) -> u64 {
    let mut bytes = salt.to_le_bytes().to_vec();
    bytes.extend_from_slice(voice.as_bytes());
    bytes.push(0x1f);
    bytes.extend_from_slice(&rate_percent(rate).to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
    fnv1a(&bytes)
}

pub(crate) fn file_name(key: u64) -> String {
    format!("{key:016x}.wav")
}

/// Seconds of audio in a 16-bit PCM WAV as `Speech::to_wav` writes it;
/// `None` for anything else, which the disk cache treats as damaged.
pub(crate) fn wav_duration_s(wav: &[u8]) -> Option<f64> {
    if wav.len() < 44
        || &wav[..4] != b"RIFF"
        || &wav[8..16] != b"WAVEfmt "
        || &wav[36..40] != b"data"
    {
        return None;
    }
    let byte_rate = u32::from_le_bytes(wav[28..32].try_into().ok()?);
    let data_len = u32::from_le_bytes(wav[40..44].try_into().ok()?) as usize;
    if byte_rate == 0 || wav.len() < 44 + data_len {
        return None;
    }
    Some(data_len as f64 / f64::from(byte_rate))
}

/// Lines in memory, least recently used dropped first past `cap` bytes.
pub(crate) struct MemCache {
    entries: HashMap<u64, (Arc<[u8]>, u64)>,
    bytes: usize,
    cap: usize,
    tick: u64,
}

impl MemCache {
    pub(crate) fn new(cap: usize) -> Self {
        MemCache {
            entries: HashMap::new(),
            bytes: 0,
            cap,
            tick: 0,
        }
    }

    pub(crate) fn get(&mut self, key: u64) -> Option<Arc<[u8]>> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(&key).map(|(wav, used)| {
            *used = tick;
            wav.clone()
        })
    }

    pub(crate) fn contains(&self, key: u64) -> bool {
        self.entries.contains_key(&key)
    }

    pub(crate) fn insert(&mut self, key: u64, wav: Arc<[u8]>) {
        self.tick += 1;
        self.bytes += wav.len();
        if let Some((old, _)) = self.entries.insert(key, (wav, self.tick)) {
            self.bytes -= old.len();
        }
        while self.bytes > self.cap && self.entries.len() > 1 {
            let oldest = self
                .entries
                .iter()
                .filter(|(k, _)| **k != key)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| *k);
            let Some(oldest) = oldest else { break };
            if let Some((wav, _)) = self.entries.remove(&oldest) {
                self.bytes -= wav.len();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Deletes the oldest files in `dir` until it holds at most `cap` bytes,
/// keeping the newest. Errors are logged; a cache is never worth failing for.
pub(crate) fn trim_dir(dir: &Path, cap: u64) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = read
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| {
                let when = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                (when, meta.len(), e.path())
            })
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= cap {
        return;
    }
    files.sort_by_key(|(when, _, _)| *when);
    for (_, len, path) in files {
        if total <= cap {
            break;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => total -= len,
            Err(e) => log::warn!("voice cache: could not remove {}: {e}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Speech;

    #[test]
    fn keys_differ_by_voice_rate_text_and_salt() {
        let k = key(1, "bf_emma", 1.0, "Tip-off.");
        assert_eq!(k, key(1, "bf_emma", 1.000_001, "Tip-off."));
        assert_ne!(k, key(1, "am_michael", 1.0, "Tip-off."));
        assert_ne!(k, key(1, "bf_emma", 1.1, "Tip-off."));
        assert_ne!(k, key(1, "bf_emma", 1.0, "Tip off."));
        assert_ne!(k, key(2, "bf_emma", 1.0, "Tip-off."));
    }

    #[test]
    fn duration_reads_what_to_wav_writes() {
        let wav = Speech {
            sample_rate: 24_000,
            samples: vec![0.0; 36_000],
        }
        .to_wav();
        assert_eq!(wav_duration_s(&wav), Some(1.5));
        assert_eq!(wav_duration_s(&wav[..100]), None);
        assert_eq!(
            wav_duration_s(b"not a wav file at all, but long enough to read"),
            None
        );
    }

    #[test]
    fn memory_drops_the_least_recently_used() {
        let mut mem = MemCache::new(250);
        let clip = |n: usize| -> Arc<[u8]> { vec![0u8; n].into() };
        mem.insert(1, clip(100));
        mem.insert(2, clip(100));
        assert!(mem.get(1).is_some());
        mem.insert(3, clip(100));
        assert!(mem.contains(1));
        assert!(!mem.contains(2));
        assert!(mem.contains(3));
        assert_eq!(mem.len(), 2);
    }

    #[test]
    fn trimming_keeps_the_newest_files() {
        let dir = tempfile::tempdir().unwrap();
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            let path = dir.path().join(name);
            std::fs::write(&path, vec![0u8; 100]).unwrap();
            let when = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_000_000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        trim_dir(dir.path(), 250);
        assert!(!dir.path().join("a").exists());
        assert!(dir.path().join("b").exists());
        assert!(dir.path().join("c").exists());
    }
}
