//! The sound device under Kira: a cpal output that survives the device
//! going away.
//!
//! Kira's own cpal backend restarts its stream when the default device
//! changes, but it panics its watcher thread if the new device fails to
//! open, and it never tries again after a disconnect that leaves no default
//! device for a moment. Either way the game goes silent until it is
//! restarted, which is what a Bluetooth headset connecting or dropping out
//! does on Windows. This backend keeps Kira's renderer and watches the same
//! way, but a device that fails to open is retried on the next check, and
//! nothing on the watcher thread panics.
//!
//! A stream can also end without saying the device is gone, so the watcher
//! judges it dead in three ways, and closes and reopens it on each:
//!
//! - it reports that the device was lost or the stream is no longer valid;
//! - it reports any other error (other than an underrun or a reroute) and
//!   then stops asking for sound for 0.5 s, which is how a Windows stream
//!   ends on an error;
//! - it stops asking for sound for 2 s with no error at all, as a Bluetooth
//!   link that is neither up nor down does. A stream that stays silent after
//!   a reopen is given 4, 8, then 16 s before the next one, whichever way
//!   the first was judged.
//!
//! The rules live in [`health`]; the stream's callback only counts itself.
//!
//! The stream handling follows Kira 0.12's `CpalBackend` (MIT or Apache-2.0).

mod health;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use kira::backend::{Backend, Renderer};
use rtrb::{Consumer, Producer, RingBuffer};

use health::{Event, Health, Verdict, Why};

/// How often the watcher checks the stream and the default device.
const CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// How long the watcher waits for a dropped stream to hand the renderer
/// back before it stops waiting and looks again on each check.
const RENDERER_RETURN_WAIT: Duration = Duration::from_millis(500);

/// Stream errors held for the watcher, and the most it reads per check: a
/// stream that reports errors in a loop must not hold the watcher.
const ERROR_RING: usize = 64;

/// The default output device, re-opened whenever it changes or fails.
pub struct DeviceBackend {
    state: State,
    stop: Arc<AtomicBool>,
}

enum State {
    /// Set up, waiting for Kira's renderer.
    Ready {
        device: Device,
        config: StreamConfig,
    },
    /// The watcher thread owns the stream.
    Started,
}

/// Why the sound device could not be set up.
#[derive(Debug)]
pub enum DeviceError {
    /// The system reports no default output device.
    NoDevice,
    /// The audio library refused the device.
    Cpal(cpal::Error),
    /// The watcher thread could not be started.
    Thread(std::io::Error),
    /// The device was set up twice.
    AlreadyStarted,
}

impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceError::NoDevice => f.write_str("no default output device"),
            DeviceError::Cpal(e) => e.fmt(f),
            DeviceError::Thread(e) => write!(f, "could not start the device watcher: {e}"),
            DeviceError::AlreadyStarted => f.write_str("the device was already started"),
        }
    }
}

impl std::error::Error for DeviceError {}

impl From<cpal::Error> for DeviceError {
    fn from(e: cpal::Error) -> Self {
        DeviceError::Cpal(e)
    }
}

impl Backend for DeviceBackend {
    type Settings = ();
    type Error = DeviceError;

    fn setup((): (), _internal_buffer_size: usize) -> Result<(Self, u32), DeviceError> {
        let (device, config) = default_device()?;
        let sample_rate = config.sample_rate;
        Ok((
            DeviceBackend {
                state: State::Ready { device, config },
                stop: Arc::new(AtomicBool::new(false)),
            },
            sample_rate,
        ))
    }

    fn start(&mut self, renderer: Renderer) -> Result<(), DeviceError> {
        let State::Ready { device, config } = std::mem::replace(&mut self.state, State::Started)
        else {
            return Err(DeviceError::AlreadyStarted);
        };
        let mut watcher = Watcher {
            output: Output::Idle(renderer),
            device_id: device_id(&device),
            sample_rate: config.sample_rate,
            failed_opens: 0,
            health: Health::default(),
        };
        // The first open reports its error to the caller, as Kira's does, so
        // a machine with no working device starts silent with the notice.
        watcher.open(&device, config)?;
        let stop = Arc::clone(&self.stop);
        std::thread::Builder::new()
            .name("audio-device".to_string())
            .spawn(move || watcher.run(&stop))
            .map_err(DeviceError::Thread)?;
        Ok(())
    }
}

impl Drop for DeviceBackend {
    /// Tells the watcher to close the stream; it exits within one check.
    /// Nothing joins it.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

// The renderer is moved between states, never copied per frame; boxing it
// would only add an allocation. Kira's own backend allows this too.
#[allow(clippy::large_enum_variant)]
enum Output {
    /// No stream; the renderer waits for a device.
    Idle(Renderer),
    Running {
        stream: Stream,
        renderer_back: Consumer<Renderer>,
        errors: Consumer<cpal::Error>,
        /// Callbacks so far; the stream's callback adds one each time.
        beats: Arc<AtomicU64>,
    },
    /// A dropped stream has not handed the renderer back yet; each check
    /// looks for it.
    Returning(Consumer<Renderer>),
    /// Only while `open` or `close` moves the renderer; never left here.
    Moving,
}

struct Watcher {
    output: Output,
    device_id: String,
    sample_rate: u32,
    /// Opens failed in a row, so a missing device is logged once, not twice
    /// a second.
    failed_opens: u32,
    /// Whether the running stream still plays.
    health: Health,
}

impl Watcher {
    fn run(mut self, stop: &AtomicBool) {
        loop {
            std::thread::sleep(CHECK_INTERVAL);
            if stop.load(Ordering::SeqCst) {
                break;
            }
            self.check();
        }
        self.close();
        log::info!("audio: device watcher stopped");
    }

    fn check(&mut self) {
        if let Output::Returning(renderer_back) = &mut self.output {
            let Ok(renderer) = renderer_back.pop() else {
                return;
            };
            log::info!("audio: the sound device handed the mixer back");
            self.output = Output::Idle(renderer);
        }
        if let Output::Running { errors, beats, .. } = &mut self.output {
            let health = &mut self.health;
            // The callbacks first, then the errors: a suspect error is judged
            // by the next interval, not cleared by callbacks that ran before
            // it.
            let beats = beats.load(Ordering::Relaxed);
            let mut reopen = judge(health, Event::Check { beats }, None);
            for _ in 0..ERROR_RING {
                let Ok(error) = errors.pop() else {
                    break;
                };
                let why = judge(health, Event::Error(error.kind()), Some(&error));
                reopen = reopen.or(why);
            }
            if reopen.is_some() {
                self.close();
            }
        }
        match &self.output {
            Output::Idle(_) => self.reopen(),
            Output::Running { .. } => self.follow_default(),
            Output::Returning(_) | Output::Moving => {}
        }
    }

    /// Moves to a new default device when the system changes it. Kira skips
    /// this on macOS, where querying the device while playing crackles
    /// (tesselode/kira#38); a disconnect there still reopens.
    fn follow_default(&mut self) {
        if cfg!(target_os = "macos") {
            return;
        }
        let Ok((device, config)) = default_device() else {
            return;
        };
        if device_id(&device) != self.device_id || config.sample_rate != self.sample_rate {
            log::info!("audio: the default sound device changed");
            self.close();
            if let Err(e) = self.open(&device, config) {
                self.note_failed_open(&e);
            }
        }
    }

    /// Tries the default device again after a disconnect or a failed open.
    fn reopen(&mut self) {
        let opened = default_device().and_then(|(device, config)| self.open(&device, config));
        match opened {
            Ok(()) => {
                if self.failed_opens > 0 {
                    log::info!(
                        "audio: sound device reopened after {} attempt(s)",
                        self.failed_opens
                    );
                }
                self.failed_opens = 0;
            }
            Err(e) => self.note_failed_open(&e),
        }
    }

    fn note_failed_open(&mut self, e: &DeviceError) {
        if self.failed_opens == 0 {
            log::warn!("audio: could not open the sound device, will keep trying ({e})");
        }
        self.failed_opens = self.failed_opens.saturating_add(1);
    }

    /// Starts a stream on `device`. Only from `Idle`; on failure the
    /// renderer stays idle for the next try.
    fn open(&mut self, device: &Device, config: StreamConfig) -> Result<(), DeviceError> {
        let mut renderer = match std::mem::replace(&mut self.output, Output::Moving) {
            Output::Idle(renderer) => renderer,
            other => {
                // Already running or waiting for the renderer: nothing to
                // start.
                self.output = other;
                return Ok(());
            }
        };
        if config.sample_rate != self.sample_rate {
            renderer.on_change_sample_rate(config.sample_rate);
            self.sample_rate = config.sample_rate;
        }
        self.device_id = device_id(device);
        let (renderer, renderer_back) = ReturnOnDrop::new(renderer);
        let (mut error_tx, errors) = RingBuffer::new(ERROR_RING);
        let beats = Arc::new(AtomicU64::new(0));
        let channels = config.channels;
        let built = device.build_output_stream(
            config,
            {
                let mut renderer = renderer;
                let beats = Arc::clone(&beats);
                move |data: &mut [f32], _| {
                    beats.fetch_add(1, Ordering::Relaxed);
                    if let Some(r) = renderer.get() {
                        r.on_start_processing();
                        r.process(data, channels);
                    } else {
                        data.fill(0.0);
                    }
                }
            },
            move |error| {
                // Full means the watcher is behind; the newest error is
                // dropped, and the next check still sees the earlier ones.
                let _ = error_tx.push(error);
            },
            None,
        );
        // A stream that failed to build or play drops the callback, which
        // hands the renderer back.
        let result = built
            .map_err(DeviceError::from)
            .and_then(|stream| stream.play().map(|()| stream).map_err(DeviceError::from));
        match result {
            Ok(stream) => {
                // A reopen for silence logs when sound comes back instead,
                // not on every attempt.
                if self.health.stalls() == 0 {
                    log::info!("audio: sound device open ({} Hz)", self.sample_rate);
                }
                self.health.next(Event::Opened);
                self.output = Output::Running {
                    stream,
                    renderer_back,
                    errors,
                    beats,
                };
                Ok(())
            }
            Err(e) => {
                self.output = take_renderer(renderer_back);
                Err(e)
            }
        }
    }

    /// Drops the stream and takes the renderer back.
    fn close(&mut self) {
        self.output = match std::mem::replace(&mut self.output, Output::Moving) {
            Output::Running {
                stream,
                renderer_back,
                ..
            } => {
                drop(stream);
                take_renderer(renderer_back)
            }
            other => other,
        };
    }
}

/// Feeds one event to the stream's health and logs what it decided.
/// Returns why the stream should be reopened, if it should.
fn judge(health: &mut Health, event: Event, error: Option<&cpal::Error>) -> Option<Why> {
    let error = error.map(ToString::to_string).unwrap_or_default();
    match health.next(event) {
        Verdict::Keep => {
            if matches!(event, Event::Error(_)) {
                log::debug!("audio: stream error ({error})");
            }
            None
        }
        Verdict::Suspect => {
            log::warn!("audio: the sound device reported an error ({error})");
            None
        }
        Verdict::Resumed => {
            log::info!("audio: sound is playing again");
            None
        }
        Verdict::Reopen(Why::Lost) => {
            log::warn!("audio: the sound device went away ({error})");
            Some(Why::Lost)
        }
        Verdict::Reopen(Why::Stalled) => {
            if health.stalls() == 1 {
                log::warn!("audio: the sound device stopped taking sound; reopening it");
            }
            Some(Why::Stalled)
        }
    }
}

/// Waits a bounded time for a dropped stream's callback to return the
/// renderer. cpal drops the callback with the stream, so this is normally
/// immediate; if not, each later check looks again.
fn take_renderer(mut renderer_back: Consumer<Renderer>) -> Output {
    let deadline = std::time::Instant::now() + RENDERER_RETURN_WAIT;
    loop {
        if let Ok(renderer) = renderer_back.pop() {
            return Output::Idle(renderer);
        }
        if std::time::Instant::now() >= deadline {
            log::error!(
                "audio: the sound device kept the mixer; sound is off until it hands it back"
            );
            return Output::Returning(renderer_back);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Holds the renderer inside the stream callback and sends it back to the
/// watcher when the callback is dropped.
struct ReturnOnDrop {
    renderer: Option<Renderer>,
    back: Producer<Renderer>,
}

impl ReturnOnDrop {
    fn new(renderer: Renderer) -> (Self, Consumer<Renderer>) {
        let (back, renderer_back) = RingBuffer::new(1);
        (
            ReturnOnDrop {
                renderer: Some(renderer),
                back,
            },
            renderer_back,
        )
    }

    fn get(&mut self) -> Option<&mut Renderer> {
        self.renderer.as_mut()
    }
}

impl Drop for ReturnOnDrop {
    fn drop(&mut self) {
        if let Some(renderer) = self.renderer.take() {
            // The ring holds one and this is its only push.
            let _ = self.back.push(renderer);
        }
    }
}

fn default_device() -> Result<(Device, StreamConfig), DeviceError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(DeviceError::NoDevice)?;
    let config = device.default_output_config()?.config();
    Ok((device, config))
}

fn device_id(device: &Device) -> String {
    device
        .id()
        .map(|id| id.to_string())
        .unwrap_or_else(|_| "unknown device".to_string())
}
