//! The built-in voice: genny's formant synth (`hearken-genny`) played
//! through the game's own sound device, for a player with no screen reader
//! and no speech engine (a handheld, a bare Linux install).
//!
//! It is one more backend in the registry, named [`BUILTIN_BACKEND`], ranked
//! below every Prism voice, so `pick_backend` reaches it only when nothing
//! else can speak; Settings can also choose it outright (see
//! [`crate::SpeechSettings::self_voicing`]). The registry keeps one engine
//! and every `acquire` hands out a handle to it, the way Prism caches one
//! instance per backend, so the three-second health check costs nothing.
//!
//! Lines are rendered a sentence at a time on a worker thread and queued for
//! the output stream, so a long help page starts speaking after its first
//! sentence. An interrupting line bumps a generation counter: the renderer
//! drops anything older and the queue is cleared at once.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::backend::{BackendId, SpeechError, VoiceBackend, VoiceFeatures, VoiceRegistry};
use crate::BUILTIN_BACKEND;

/// The built-in voice's registry id; Prism's ids are hashes of the name.
pub const BUILTIN_ID: BackendId = 0x4745_4E4E_5900_0001;

/// Below Prism's UIA last resort: the built-in voice speaks only when
/// nothing on the machine can.
pub const BUILTIN_PRIORITY: i32 = crate::UIA_LAST_RESORT_PRIORITY - 1;

/// The speed multiplier for the 0..1 rate setting: 0.6x to 1.2x across
/// the lower half, then up to 3x, so the middle sounds natural and the top
/// is as quick as a fast screen reader user wants.
pub fn rate_multiplier(rate: f64) -> f64 {
    let r = rate.clamp(0.0, 1.0);
    if r <= 0.5 {
        0.6 + 1.2 * r
    } else {
        1.2 + 3.6 * (r - 0.5)
    }
}

/// Pitch setting 0..1 to a factor on the voice's own pitch, 0.5 = as built.
pub fn pitch_factor(pitch: f64) -> f64 {
    2f64.powf((pitch.clamp(0.0, 1.0) - 0.5) * 1.2)
}

struct Voicing {
    voice: usize,
    rate: f64,
    pitch: f64,
}

struct Shared {
    queue: Mutex<VecDeque<f32>>,
    generation: AtomicU64,
    /// Lines handed to the renderer and not finished yet.
    pending: AtomicUsize,
    /// f32 bits of the volume, 0..1.
    volume: AtomicU32,
    voicing: Mutex<Voicing>,
    /// The output stream is open and healthy.
    output_ok: AtomicBool,
    quit: AtomicBool,
}

struct Job {
    text: String,
    generation: u64,
}

/// One synth, one renderer thread and one output stream, shared by every
/// handle the registry gives out.
pub struct BuiltinEngine {
    shared: Arc<Shared>,
    jobs: mpsc::Sender<Job>,
}

impl BuiltinEngine {
    /// Open the default sound device and start the renderer. A machine with
    /// no sound device yields an engine whose voice is never usable.
    pub fn start() -> Arc<BuiltinEngine> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
            pending: AtomicUsize::new(0),
            volume: AtomicU32::new(1.0f32.to_bits()),
            voicing: Mutex::new(Voicing {
                voice: 0,
                rate: 0.5,
                pitch: 0.5,
            }),
            output_ok: AtomicBool::new(false),
            quit: AtomicBool::new(false),
        });
        let sample_rate = output::open(Arc::clone(&shared));
        let (jobs, rx) = mpsc::channel::<Job>();
        if let Some(sr) = sample_rate {
            let worker = Arc::clone(&shared);
            let spawned = std::thread::Builder::new()
                .name("builtin-voice".into())
                .spawn(move || render_loop(&worker, &rx, sr));
            if let Err(err) = spawned {
                log::warn!("Built-in voice: no renderer thread: {err}");
                shared.output_ok.store(false, Ordering::Release);
            }
        }
        Arc::new(BuiltinEngine { shared, jobs })
    }

    /// An engine with no sound device that still renders, its samples
    /// left in the queue: for tests, and for checking a line's length.
    pub fn offline() -> Arc<BuiltinEngine> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            generation: AtomicU64::new(0),
            pending: AtomicUsize::new(0),
            volume: AtomicU32::new(1.0f32.to_bits()),
            voicing: Mutex::new(Voicing {
                voice: 0,
                rate: 0.5,
                pitch: 0.5,
            }),
            output_ok: AtomicBool::new(true),
            quit: AtomicBool::new(false),
        });
        let (jobs, rx) = mpsc::channel::<Job>();
        let worker = Arc::clone(&shared);
        std::thread::spawn(move || render_loop(&worker, &rx, 16_000.0));
        Arc::new(BuiltinEngine { shared, jobs })
    }

    /// Samples rendered and not yet played.
    pub fn queued(&self) -> usize {
        self.shared.queue.lock().map_or(0, |q| q.len())
    }

    /// Wait until everything handed over is rendered (tests).
    pub fn wait_rendered(&self, limit: Duration) -> bool {
        let start = std::time::Instant::now();
        while self.shared.pending.load(Ordering::Acquire) > 0 {
            if start.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }

    fn usable(&self) -> bool {
        self.shared.output_ok.load(Ordering::Acquire)
    }

    fn say(&self, text: &str, interrupt: bool) {
        let generation = if interrupt {
            self.cut()
        } else {
            self.shared.generation.load(Ordering::Acquire)
        };
        self.shared.pending.fetch_add(1, Ordering::AcqRel);
        if self
            .jobs
            .send(Job {
                text: text.to_string(),
                generation,
            })
            .is_err()
        {
            self.shared.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// Drop everything queued or being rendered; the new generation.
    fn cut(&self) -> u64 {
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.clear();
        }
        generation
    }

    fn speaking(&self) -> bool {
        self.shared.pending.load(Ordering::Acquire) > 0
            || self.shared.queue.lock().is_ok_and(|q| !q.is_empty())
    }
}

impl Drop for BuiltinEngine {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::Release);
    }
}

fn render_loop(shared: &Shared, rx: &mpsc::Receiver<Job>, sr: f64) {
    let voices: Vec<hearken_genny::Voice> = hearken_genny::VOICE_NAMES
        .iter()
        .filter_map(|name| hearken_genny::voice(name))
        .collect();
    while let Ok(job) = rx.recv() {
        if shared.quit.load(Ordering::Acquire) {
            return;
        }
        for sentence in hearken_genny::sentences(&job.text) {
            if shared.generation.load(Ordering::Acquire) != job.generation {
                break;
            }
            let (mut voice, rate) = {
                let v = shared.voicing.lock().expect("built-in voice settings lock");
                let voice = voices
                    .get(v.voice)
                    .cloned()
                    .unwrap_or_else(|| voices[0].clone());
                (voice, (v.pitch, v.rate))
            };
            voice.pitch *= pitch_factor(rate.0);
            let multiplier = rate_multiplier(rate.1);
            let mut samples = hearken_genny::speak(sentence, &voice, sr, multiplier);
            if samples.is_empty() {
                continue;
            }
            // The pause genny leaves between sentences of one line.
            let gap = (0.2 / multiplier * sr) as usize;
            samples.extend(std::iter::repeat_n(0.0, gap));
            for chunk in samples.chunks(4096) {
                if shared.generation.load(Ordering::Acquire) != job.generation {
                    break;
                }
                if let Ok(mut queue) = shared.queue.lock() {
                    queue.extend(chunk);
                }
            }
        }
        shared.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The output stream, on a thread of its own that holds it (a cpal stream
/// may not move between threads on every platform). Passing stream errors
/// are logged and ignored, as the game's own audio does; a lost device is
/// reopened every second until one answers.
mod output {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    /// Open the default output; the device's sample rate, or `None`.
    pub(super) fn open(shared: Arc<Shared>) -> Option<f64> {
        let (tx, rx) = mpsc::channel::<Option<f64>>();
        let spawned = std::thread::Builder::new()
            .name("builtin-voice-output".into())
            .spawn(move || {
                let lost = Arc::new(AtomicBool::new(false));
                let Some((mut stream, rate)) = build(&shared, &lost) else {
                    let _ = tx.send(None);
                    return;
                };
                let _ = tx.send(Some(rate));
                while !shared.quit.load(Ordering::Acquire) {
                    std::thread::park_timeout(Duration::from_millis(250));
                    if !lost.swap(false, Ordering::AcqRel) {
                        continue;
                    }
                    drop(stream);
                    shared.output_ok.store(false, Ordering::Release);
                    // Back at the rate the renderer was started with, or
                    // not at all: a voice an octave off is worse than none
                    // for the second it takes the next check to come round.
                    loop {
                        if shared.quit.load(Ordering::Acquire) {
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                        match build(&shared, &lost) {
                            Some((again, r)) if (r - rate).abs() < 0.5 => {
                                stream = again;
                                break;
                            }
                            Some(_) => log::warn!(
                                "Built-in voice: the new sound device runs at another rate"
                            ),
                            None => {}
                        }
                    }
                }
                drop(stream);
            });
        if spawned.is_err() {
            return None;
        }
        rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
    }

    fn build(shared: &Arc<Shared>, lost: &Arc<AtomicBool>) -> Option<(cpal::Stream, f64)> {
        let Some(device) = cpal::default_host().default_output_device() else {
            log::warn!("Built-in voice: no sound device");
            return None;
        };
        let config = match device.default_output_config() {
            Ok(config) => config.config(),
            Err(err) => {
                log::warn!("Built-in voice: no output format: {err}");
                return None;
            }
        };
        let channels = usize::from(config.channels.max(1));
        let sample_rate = f64::from(config.sample_rate);
        let callback_shared = Arc::clone(shared);
        let lost = Arc::clone(lost);
        let built = device.build_output_stream(
            config,
            move |data: &mut [f32], _| {
                let volume = f32::from_bits(callback_shared.volume.load(Ordering::Relaxed));
                let mut queue = callback_shared.queue.lock().ok();
                for frame in data.chunks_mut(channels) {
                    let s = queue.as_mut().and_then(|q| q.pop_front()).unwrap_or(0.0) * volume;
                    frame.fill(s);
                }
            },
            move |err| match err.kind() {
                cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
                    log::warn!("Built-in voice: the sound device went away ({err})");
                    lost.store(true, Ordering::Release);
                }
                _ => log::debug!("Built-in voice: stream error ({err})"),
            },
            None,
        );
        let stream = match built.and_then(|stream| stream.play().map(|()| stream)) {
            Ok(stream) => stream,
            Err(err) => {
                log::warn!("Built-in voice: could not open the sound device: {err}");
                return None;
            }
        };
        shared.output_ok.store(true, Ordering::Release);
        log::info!("Built-in voice ready ({sample_rate} Hz)");
        Some((stream, sample_rate))
    }
}

/// A handle on the shared engine: what `acquire` returns.
pub struct BuiltinVoice {
    engine: Arc<BuiltinEngine>,
}

impl VoiceBackend for BuiltinVoice {
    fn name(&self) -> String {
        BUILTIN_BACKEND.to_string()
    }

    fn features(&self) -> VoiceFeatures {
        VoiceFeatures {
            is_supported_at_runtime: self.engine.usable(),
            ..VoiceFeatures::ADJUSTABLE
        }
    }

    fn output(&mut self, text: &str, interrupt: bool) -> Result<(), SpeechError> {
        self.speak(text, interrupt)
    }

    fn speak(&mut self, text: &str, interrupt: bool) -> Result<(), SpeechError> {
        self.engine.say(text, interrupt);
        Ok(())
    }

    fn braille(&mut self, _text: &str) -> Result<(), SpeechError> {
        Err(prismer::Error::NotImplemented)
    }

    fn stop(&mut self) -> Result<(), SpeechError> {
        self.engine.cut();
        Ok(())
    }

    fn is_speaking(&self) -> Result<bool, SpeechError> {
        Ok(self.engine.speaking())
    }

    fn speaking_report(&self) -> Option<bool> {
        Some(self.engine.speaking())
    }

    fn set_rate(&mut self, rate: f64) -> Result<(), SpeechError> {
        self.engine
            .shared
            .voicing
            .lock()
            .expect("voicing lock")
            .rate = rate;
        Ok(())
    }

    fn set_pitch(&mut self, pitch: f64) -> Result<(), SpeechError> {
        self.engine
            .shared
            .voicing
            .lock()
            .expect("voicing lock")
            .pitch = pitch;
        Ok(())
    }

    fn set_volume(&mut self, volume: f64) -> Result<(), SpeechError> {
        let v = volume.clamp(0.0, 1.0) as f32;
        self.engine
            .shared
            .volume
            .store(v.to_bits(), Ordering::Relaxed);
        Ok(())
    }

    fn voices_count(&self) -> Result<usize, SpeechError> {
        Ok(hearken_genny::VOICE_NAMES.len())
    }

    fn voice_name(&self, index: usize) -> Result<String, SpeechError> {
        hearken_genny::VOICE_NAMES
            .get(index)
            .map(|name| name.to_string())
            .ok_or(prismer::Error::RangeOutOfBounds)
    }

    fn set_voice(&mut self, index: usize) -> Result<(), SpeechError> {
        if index >= hearken_genny::VOICE_NAMES.len() {
            return Err(prismer::Error::RangeOutOfBounds);
        }
        self.engine
            .shared
            .voicing
            .lock()
            .expect("voicing lock")
            .voice = index;
        Ok(())
    }
}

/// A registry with the built-in voice after every backend of `inner` (or
/// alone, when Prism could not start at all).
pub struct WithBuiltin {
    inner: Option<Box<dyn VoiceRegistry>>,
    engine: OnceLock<Arc<BuiltinEngine>>,
    start: fn() -> Arc<BuiltinEngine>,
}

impl WithBuiltin {
    pub fn new(inner: Option<Box<dyn VoiceRegistry>>) -> WithBuiltin {
        WithBuiltin::with_engine(inner, BuiltinEngine::start)
    }

    /// Tests supply an engine of their own.
    pub fn with_engine(
        inner: Option<Box<dyn VoiceRegistry>>,
        start: fn() -> Arc<BuiltinEngine>,
    ) -> WithBuiltin {
        WithBuiltin {
            inner,
            engine: OnceLock::new(),
            start,
        }
    }

    /// The engine, once something acquired the built-in voice.
    pub fn engine(&self) -> Option<Arc<BuiltinEngine>> {
        self.engine.get().cloned()
    }

    fn inner_count(&self) -> usize {
        self.inner.as_ref().map_or(0, |r| r.backend_count())
    }
}

impl VoiceRegistry for WithBuiltin {
    fn backend_count(&self) -> usize {
        self.inner_count() + 1
    }

    fn id_at(&self, index: usize) -> Option<BackendId> {
        let count = self.inner_count();
        if index < count {
            self.inner.as_ref()?.id_at(index)
        } else if index == count {
            Some(BUILTIN_ID)
        } else {
            None
        }
    }

    fn id_by_name(&self, name: &str) -> Option<BackendId> {
        if name.eq_ignore_ascii_case(BUILTIN_BACKEND) {
            return Some(BUILTIN_ID);
        }
        self.inner.as_ref()?.id_by_name(name)
    }

    fn name_of(&self, id: BackendId) -> Option<String> {
        if id == BUILTIN_ID {
            return Some(BUILTIN_BACKEND.to_string());
        }
        self.inner.as_ref()?.name_of(id)
    }

    fn priority_of(&self, id: BackendId) -> i32 {
        if id == BUILTIN_ID {
            return BUILTIN_PRIORITY;
        }
        self.inner.as_ref().map_or(i32::MIN, |r| r.priority_of(id))
    }

    fn acquire(&self, id: BackendId) -> Result<Box<dyn VoiceBackend>, SpeechError> {
        if id == BUILTIN_ID {
            let engine = Arc::clone(self.engine.get_or_init(self.start));
            return Ok(Box::new(BuiltinVoice { engine }));
        }
        match &self.inner {
            Some(inner) => inner.acquire(id),
            None => Err(prismer::Error::BackendNotAvailable),
        }
    }

    fn settle(&self) {
        if let Some(inner) = &self.inner {
            inner.settle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_middle_is_natural_and_ends_are_wide() {
        assert!((rate_multiplier(0.0) - 0.6).abs() < 1e-9);
        assert!((rate_multiplier(0.5) - 1.2).abs() < 1e-9);
        assert!((rate_multiplier(1.0) - 3.0).abs() < 1e-9);
        assert!((pitch_factor(0.5) - 1.0).abs() < 1e-9);
    }
}
