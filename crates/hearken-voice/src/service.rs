//! `Voice`: the AI commentary service a game installs on its context.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use hearken_app::audio::Start;
use hearken_app::{Bus, Commentary, Ctx, EventPriority, Game, Line, Sound, TRANSCRIPT_TARGET};

use crate::cache::{self, MemCache};
use crate::settings::{HasVoiceSettings, Role, VoiceMode};
use crate::synth::Loader;
use crate::worker::{Job, Reply, Request, Worker};

/// Why the AI voices are not in use, when the package is missing.
pub const NOT_INSTALLED: &str =
    "The AI voices are not installed, so the screen reader speaks the commentary.";

/// Why, in a build made without the `voice` feature.
pub const NO_MODEL_IN_BUILD: &str =
    "This copy of the game was built without the AI voices, so the screen reader speaks the commentary.";

/// How the service is set up. Every field has a default but `roles`.
#[derive(Clone, Debug)]
pub struct VoiceConfig {
    /// The game's speakers.
    pub roles: &'static [Role],
    /// The voices folder; `None` looks where [`crate::find_voices_in`] does.
    pub voices: Option<PathBuf>,
    /// Where synthesized lines are kept between runs; `None` uses
    /// `voice-cache` in the game's data folder.
    pub cache_dir: Option<PathBuf>,
    /// Keep nothing on disk (lines are still kept in memory).
    pub no_disk_cache: bool,
    /// CMU-format pronunciations; `None` uses `lexicon.txt` in the voices
    /// folder when there is one.
    pub lexicon: Option<PathBuf>,
    /// Seconds an `Ambient` line may wait, synthesis included, before it
    /// is dropped as stale.
    pub ambient_wait_s: f64,
    /// The same for an `Important` line. `Critical` lines never go stale.
    pub important_wait_s: f64,
    /// Two identical lines this close together are one moment.
    pub repeat_window_s: f64,
    /// Bytes of synthesized lines kept in memory.
    pub memory_cache_bytes: usize,
    /// Bytes kept under `voice-cache`; the oldest files go first.
    pub disk_cache_bytes: u64,
}

impl VoiceConfig {
    pub fn new(roles: &'static [Role]) -> Self {
        VoiceConfig {
            roles,
            voices: None,
            cache_dir: None,
            no_disk_cache: false,
            lexicon: None,
            // Synthesis takes 1 to 6 s on a modest machine (see the voice
            // spike note), so these are longer than the event pacer's 3 s
            // and 0.8 s: an uncached line still has time to be heard.
            ambient_wait_s: 4.0,
            important_wait_s: 8.0,
            repeat_window_s: hearken_app::PacerConfig::DEFAULT.repeat_window_s,
            memory_cache_bytes: 64 << 20,
            disk_cache_bytes: 256 << 20,
        }
    }
}

/// Makes the engine, or says why there is none.
type Make = Box<dyn Fn(&VoiceConfig, Option<&std::path::Path>) -> Result<Loader, String>>;

enum State {
    Loading,
    Ready,
    Unavailable(String),
}

struct Queued<C> {
    /// The line as the player's verbosity level resolved it.
    text: String,
    line: Line<C>,
    key: u64,
    asked_at: f64,
    wav: Option<Arc<[u8]>>,
}

struct Playing {
    sound: Sound,
    ends_at: f64,
}

/// AI commentary: Kokoro voices synthesized on a worker thread, cached,
/// and played one line at a time on `Bus::VOICE`.
///
/// Install it with `ctx.set_commentary(Box::new(Voice::new(ctx, config)))`,
/// then speak with `ctx.commentate(Line::new("play_by_play", text))`.
///
/// The queue keeps the pacer's rules: the same line within
/// `repeat_window_s` is said once, a keyed line only when its text changes,
/// a line that would start after its priority's wait is dropped (its
/// synthesis still fills the cache), and a `Critical` line cuts off what is
/// playing and clears the queue. While no model is installed, it fails to
/// load, or the player chose the screen reader, lines go to the event voice
/// through `ctx.say_line`.
pub struct Voice<G: Game> {
    config: VoiceConfig,
    make: Make,
    state: State,
    worker: Option<Worker>,
    speakers: Vec<String>,
    salt: u64,
    mem: MemCache,
    queue: VecDeque<Queued<G::Category>>,
    /// Keys asked for urgently and not back yet.
    in_flight: HashSet<u64>,
    playing: Option<Playing>,
    /// When each (speaker, text) was last said.
    recent: HashMap<(String, String), f64>,
    /// The last text of each keyed line.
    by_key: HashMap<String, String>,
    first_times: HashSet<(String, String)>,
}

impl<G: Game> Voice<G>
where
    G::Settings: HasVoiceSettings,
{
    /// The Kokoro voices from the installed package. Without the package,
    /// or in a build without the `voice` feature, every line goes to the
    /// screen reader and Settings says why.
    pub fn new(ctx: &Ctx<G>, config: VoiceConfig) -> Self {
        Self::with_loader(ctx, config, kokoro_loader)
    }

    /// The service over another engine: `make` returns its [`Loader`] or
    /// why there is none. Tests use it with a stand-in.
    pub fn with_loader(
        ctx: &Ctx<G>,
        config: VoiceConfig,
        make: impl Fn(&VoiceConfig, Option<&std::path::Path>) -> Result<Loader, String> + 'static,
    ) -> Self {
        let mut voice = Voice {
            mem: MemCache::new(config.memory_cache_bytes),
            config,
            make: Box::new(make),
            state: State::Loading,
            worker: None,
            speakers: Vec::new(),
            salt: 0,
            queue: VecDeque::new(),
            in_flight: HashSet::new(),
            playing: None,
            recent: HashMap::new(),
            by_key: HashMap::new(),
            first_times: HashSet::new(),
        };
        voice.start(ctx);
        voice
    }

    /// Whether the model has loaded.
    pub fn is_ready(&self) -> bool {
        matches!(self.state, State::Ready)
    }

    /// Lines waiting to play.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Finds the voices and starts the worker.
    fn start(&mut self, ctx: &Ctx<G>) {
        self.worker = None;
        self.in_flight.clear();
        let voices = self
            .config
            .voices
            .clone()
            .or_else(|| crate::find_voices_in(G::ENV_PREFIX, Some(&ctx.paths.data_dir)));
        let lexicon = self
            .config
            .lexicon
            .clone()
            .or_else(|| voices.as_ref().map(|v| v.join("lexicon.txt")))
            .filter(|p| p.is_file());
        self.salt = cache::salt(lexicon.as_deref());
        let mut config = self.config.clone();
        config.lexicon = lexicon;
        match (self.make)(&config, voices.as_deref()) {
            Ok(loader) => {
                let cache_dir = (!self.config.no_disk_cache).then(|| {
                    self.config
                        .cache_dir
                        .clone()
                        .unwrap_or_else(|| ctx.paths.data_dir.join("voice-cache"))
                });
                self.worker = Some(Worker::spawn(
                    loader,
                    cache_dir,
                    self.config.disk_cache_bytes,
                ));
                self.state = State::Loading;
            }
            Err(reason) => {
                log::info!("voice: {reason}");
                self.state = State::Unavailable(reason);
            }
        }
    }

    fn mode(ctx: &Ctx<G>) -> VoiceMode {
        ctx.settings.game.voice_settings().mode
    }

    /// The voice, rate and cache key `line` (saying `text`) plays with.
    fn place(&self, ctx: &Ctx<G>, line: &Line<G::Category>, text: &str) -> (String, f32, u64) {
        let settings = ctx.settings.game.voice_settings();
        let voice = settings.voice_for(&line.speaker, self.config.roles);
        let rate = settings.rate();
        let key = cache::key(self.salt, &voice, rate, text);
        (voice, rate, key)
    }

    /// Whether this is the same moment said again.
    fn is_repeat(&mut self, now: f64, line: &Line<G::Category>, text: &str) -> bool {
        if let Some(key) = &line.key {
            if self.by_key.get(key).is_some_and(|last| last == text) {
                return true;
            }
            self.by_key.insert(key.clone(), text.to_string());
        }
        let window = self.config.repeat_window_s;
        let said = (line.speaker.clone(), text.to_string());
        if self.recent.get(&said).is_some_and(|at| now - at < window) {
            return true;
        }
        if self.recent.len() > 256 {
            self.recent.retain(|_, at| now - *at < window);
        }
        self.recent.insert(said, now);
        false
    }

    fn budget(&self, priority: EventPriority) -> f64 {
        match priority {
            EventPriority::Ambient => self.config.ambient_wait_s,
            EventPriority::Important => self.config.important_wait_s,
            EventPriority::Critical => f64::INFINITY,
        }
    }

    /// Cuts off the line playing.
    fn cut(&mut self, ctx: &mut Ctx<G>) {
        if let Some(playing) = self.playing.take() {
            ctx.audio.stop(playing.sound, 40);
        }
    }

    /// The fallback: `text` on the event voice, the verbosity rules already
    /// applied.
    fn fall_back(ctx: &mut Ctx<G>, item: Queued<G::Category>) {
        let mut line = Line::new(item.line.speaker, item.text).priority(item.line.priority);
        line.key = item.line.key;
        ctx.say_line(line);
    }

    /// Takes the worker's replies.
    fn drain(&mut self, ctx: &mut Ctx<G>) {
        loop {
            let reply = match &self.worker {
                Some(worker) => worker.try_recv(),
                None => return,
            };
            match reply {
                Ok(None) => return,
                Ok(Some(Reply::Loaded { speakers })) => {
                    self.speakers = speakers;
                    self.state = State::Ready;
                }
                Ok(Some(Reply::LoadFailed(error))) => {
                    self.unavailable(ctx, format!("The AI voices could not load ({error}), so the screen reader speaks the commentary."));
                    return;
                }
                Ok(Some(Reply::Ready { key, wav })) => {
                    self.in_flight.remove(&key);
                    for item in self.queue.iter_mut().filter(|i| i.key == key) {
                        item.wav = Some(wav.clone());
                    }
                    self.mem.insert(key, wav);
                }
                Ok(Some(Reply::Failed { key, error })) => {
                    self.in_flight.remove(&key);
                    log::warn!("voice: could not speak a line: {error}");
                    let (failed, kept) = std::mem::take(&mut self.queue)
                        .into_iter()
                        .partition(|i| i.key == key);
                    self.queue = kept;
                    for item in failed {
                        Self::fall_back(ctx, item);
                    }
                }
                Err(()) => {
                    self.unavailable(ctx, "The AI voices stopped working, so the screen reader speaks the commentary.".into());
                    return;
                }
            }
        }
    }

    /// The model is gone: what is queued and still timely goes to the
    /// screen reader, and so does everything after.
    fn unavailable(&mut self, ctx: &mut Ctx<G>, reason: String) {
        log::warn!("voice: {reason}");
        self.state = State::Unavailable(reason);
        self.worker = None;
        self.in_flight.clear();
        let now = ctx.clock;
        for item in std::mem::take(&mut self.queue) {
            if now - item.asked_at <= self.budget(item.line.priority) {
                Self::fall_back(ctx, item);
            }
        }
    }

    /// Plays the next line when the voice is free and it is ready, dropping
    /// stale ones on the way.
    fn pump(&mut self, ctx: &mut Ctx<G>) {
        self.drain(ctx);
        let now = ctx.clock;
        if self.playing.as_ref().is_some_and(|p| now >= p.ends_at) {
            self.playing = None;
        }
        if self.playing.is_some() {
            return;
        }
        while let Some(head) = self.queue.front() {
            if now - head.asked_at > self.budget(head.line.priority) {
                log::info!(target: TRANSCRIPT_TARGET, "[voice] stale, dropped: {}", head.text);
                self.queue.pop_front();
                continue;
            }
            let Some(wav) = head.wav.clone() else {
                return;
            };
            let Some(item) = self.queue.pop_front() else {
                return;
            };
            let length = cache::wav_duration_s(&wav).unwrap_or(0.0);
            let (voice, _, _) = self.place(ctx, &item.line, &item.text);
            let sound =
                ctx.audio
                    .play_clip(&format!("voice:{voice}"), wav, Bus::VOICE, Start::ONCE);
            self.playing = Some(Playing {
                sound,
                ends_at: now + length,
            });
            ctx.hold_duck(length);
            ctx.transcribe(format!("[voice] {}", item.text));
            return;
        }
    }
}

impl<G: Game> Commentary<G> for Voice<G>
where
    G::Settings: HasVoiceSettings,
{
    fn speak(&mut self, ctx: &mut Ctx<G>, line: Line<G::Category>) {
        match Self::mode(ctx) {
            VoiceMode::Off => {
                log::info!(target: TRANSCRIPT_TARGET, "[voice] off: {}", line.text);
                return;
            }
            VoiceMode::ScreenReader => return ctx.say_line(line),
            VoiceMode::Ai => {}
        }
        self.drain(ctx);
        if matches!(self.state, State::Unavailable(_)) {
            return ctx.say_line(line);
        }
        let Some(text) = crate::rules::resolve(ctx, &line, &mut self.first_times) else {
            return;
        };
        if self.is_repeat(ctx.clock, &line, &text) {
            return;
        }
        if line.priority == EventPriority::Critical {
            self.cut(ctx);
            self.queue.clear();
            if let Some(worker) = &self.worker {
                worker.send(Request::Clear);
            }
            self.in_flight.clear();
        }
        let (voice, rate, key) = self.place(ctx, &line, &text);
        let wav = self.mem.get(key);
        if wav.is_none() && !self.in_flight.contains(&key) {
            if let Some(worker) = &self.worker {
                worker.send(Request::Synth(Job {
                    key,
                    voice,
                    text: text.clone(),
                    rate,
                    urgent: true,
                }));
                self.in_flight.insert(key);
            }
        }
        self.queue.push_back(Queued {
            text,
            line,
            key,
            asked_at: ctx.clock,
            wav,
        });
        self.pump(ctx);
    }

    fn prime(&mut self, ctx: &mut Ctx<G>, lines: &[Line<G::Category>]) {
        if Self::mode(ctx) != VoiceMode::Ai {
            return;
        }
        let Some(worker) = &self.worker else {
            return;
        };
        for line in lines {
            // Both forms, so the line is ready whatever the verbosity.
            for text in std::iter::once(&line.text).chain(line.terse.as_ref()) {
                let (voice, rate, key) = self.place(ctx, line, text);
                if self.mem.contains(key) || self.in_flight.contains(&key) {
                    continue;
                }
                worker.send(Request::Synth(Job {
                    key,
                    voice,
                    text: text.clone(),
                    rate,
                    urgent: false,
                }));
            }
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        self.pump(ctx);
    }

    fn stop(&mut self, ctx: &mut Ctx<G>) {
        self.cut(ctx);
        self.queue.clear();
        if let Some(worker) = &self.worker {
            worker.send(Request::Clear);
        }
        self.in_flight.clear();
    }

    fn is_speaking(&self) -> bool {
        self.playing.is_some()
    }

    fn speakers(&self) -> Vec<String> {
        self.speakers.clone()
    }

    fn status(&self) -> Option<String> {
        match &self.state {
            State::Unavailable(reason) => Some(reason.clone()),
            State::Loading | State::Ready => None,
        }
    }

    fn reload(&mut self, ctx: &mut Ctx<G>) {
        self.stop(ctx);
        self.start(ctx);
    }
}

/// Kokoro from the installed package, with the lexicon when there is one.
#[cfg(feature = "voice")]
fn kokoro_loader(config: &VoiceConfig, voices: Option<&std::path::Path>) -> Result<Loader, String> {
    let dir = voices
        .and_then(crate::package_dir)
        .ok_or_else(|| NOT_INSTALLED.to_string())?;
    let lexicon = config.lexicon.clone();
    Ok(Box::new(move || {
        let mut kokoro = crate::Kokoro::load(&dir)?;
        if let Some(lexicon) = lexicon {
            let n = kokoro.add_lexicon(&lexicon)?;
            log::info!("voice: {n} pronunciations from {}", lexicon.display());
        }
        Ok(Box::new(kokoro) as Box<dyn crate::Synth>)
    }))
}

#[cfg(not(feature = "voice"))]
fn kokoro_loader(
    _config: &VoiceConfig,
    _voices: Option<&std::path::Path>,
) -> Result<Loader, String> {
    Err(NO_MODEL_IN_BUILD.to_string())
}
