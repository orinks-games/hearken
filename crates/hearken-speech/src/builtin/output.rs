//! The output stream, on a thread of its own that holds it (a cpal stream
//! may not move between threads on every platform). Nothing joins the
//! thread; it ends within a check of the engine being dropped.
//!
//! The stream is closed and the default device opened again when the
//! device reports it is gone, when it reports any other error (not an
//! underrun or a reroute) and then stops asking for sound, or when it stops
//! asking for sound for 2 s. A stream that stays silent after a reopen is
//! given 4, 8, then 16 s before the next one. The new stream is kept
//! whatever its sample rate: the voice switches to that rate and drops what
//! was queued at the old one. While no device opens, the voice reports
//! itself unusable and tries again every second.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::Shared;

/// How often the thread looks at the stream.
const CHECK: Duration = Duration::from_millis(250);

/// The wait before each try at opening a device again.
const RETRY: Duration = Duration::from_secs(1);

/// A stream that is playing, with what its callbacks report.
struct Output {
    /// Held only: dropping it closes the stream.
    _stream: cpal::Stream,
    rate: u32,
    /// Set by an error that says the device is gone.
    lost: Arc<AtomicBool>,
    /// Set by any other error that may have ended the stream; the thread
    /// takes it at each check.
    suspect: Arc<AtomicBool>,
    /// Callbacks so far; the data callback adds one each time.
    beats: Arc<AtomicU64>,
}

/// Open the default output and keep it open; false when no device opened
/// at the first try. On true the stream's rate is in `shared`.
pub(super) fn open(shared: Arc<Shared>) -> bool {
    let (tx, rx) = mpsc::channel::<bool>();
    let spawned = std::thread::Builder::new()
        .name("builtin-voice-output".into())
        .spawn(move || {
            let out = match build(&shared) {
                Ok(out) => out,
                Err(err) => {
                    log::warn!("Built-in voice: could not open the sound device ({err})");
                    let _ = tx.send(false);
                    return;
                }
            };
            keep(&shared, &out, true);
            let _ = tx.send(true);
            run(&shared, out);
        });
    if spawned.is_err() {
        return false;
    }
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false)
}

fn run(shared: &Arc<Shared>, mut out: Output) {
    let mut stall = Stall::default();
    while !shared.quit.load(Ordering::Acquire) {
        std::thread::park_timeout(CHECK);
        let lost = out.lost.load(Ordering::Acquire);
        // The callbacks first, then the errors: a suspect error is judged by
        // the next check, not cleared by callbacks that ran before it.
        let verdict = stall.next(Event::Check {
            beats: out.beats.load(Ordering::Relaxed),
        });
        if out.suspect.swap(false, Ordering::AcqRel) {
            stall.next(Event::Suspect);
        }
        match verdict {
            Verdict::Keep if !lost => continue,
            Verdict::Resumed => {
                log::info!("Built-in voice: sound is playing again");
                if !lost {
                    continue;
                }
            }
            Verdict::Reopen if stall.stalls() == 1 => {
                log::warn!("Built-in voice: the sound device stopped taking sound; reopening it");
            }
            Verdict::Keep | Verdict::Reopen => {}
        }
        drop(out);
        shared.output_ok.store(false, Ordering::Release);
        let Some(again) = reopen(shared, stall.stalls() == 0) else {
            return;
        };
        stall.next(Event::Opened);
        out = again;
    }
}

/// Opens the default device, trying every second until one opens or the
/// engine is dropped. `announce` logs the ready line; a reopen for silence
/// logs when sound comes back instead.
fn reopen(shared: &Arc<Shared>, announce: bool) -> Option<Output> {
    let mut failed = false;
    loop {
        if shared.quit.load(Ordering::Acquire) {
            return None;
        }
        std::thread::sleep(RETRY);
        match build(shared) {
            Ok(out) => {
                if failed {
                    log::info!("Built-in voice: sound device reopened");
                }
                keep(shared, &out, announce);
                return Some(out);
            }
            Err(err) => {
                if !failed {
                    log::warn!(
                        "Built-in voice: could not open the sound device, will keep trying ({err})"
                    );
                }
                failed = true;
            }
        }
    }
}

/// The voice plays on `out` from now on, at its rate.
fn keep(shared: &Shared, out: &Output, announce: bool) {
    shared.follow_rate(out.rate);
    shared.output_ok.store(true, Ordering::Release);
    if announce {
        log::info!("Built-in voice ready ({} Hz)", out.rate);
    }
}

fn build(shared: &Arc<Shared>) -> Result<Output, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| "no sound device".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|err| format!("no output format: {err}"))?
        .config();
    let channels = usize::from(config.channels.max(1));
    let rate = config.sample_rate;
    let lost = Arc::new(AtomicBool::new(false));
    let suspect = Arc::new(AtomicBool::new(false));
    let beats = Arc::new(AtomicU64::new(0));
    let callback_shared = Arc::clone(shared);
    let callback_beats = Arc::clone(&beats);
    let error_lost = Arc::clone(&lost);
    let error_suspect = Arc::clone(&suspect);
    // One warning per stream, however many errors follow.
    let warned = AtomicBool::new(false);
    let built = device.build_output_stream(
        config,
        move |data: &mut [f32], _| {
            callback_beats.fetch_add(1, Ordering::Relaxed);
            let volume = f32::from_bits(callback_shared.volume.load(Ordering::Relaxed));
            let mut queue = callback_shared.queue.lock().ok();
            for frame in data.chunks_mut(channels) {
                let s = queue.as_mut().and_then(|q| q.pop_front()).unwrap_or(0.0) * volume;
                frame.fill(s);
            }
        },
        move |err| match err.kind() {
            cpal::ErrorKind::Xrun
            | cpal::ErrorKind::RealtimeDenied
            | cpal::ErrorKind::DeviceChanged => {
                log::debug!("Built-in voice: stream error ({err})");
            }
            cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
                error_lost.store(true, Ordering::Release);
                if !warned.swap(true, Ordering::AcqRel) {
                    log::warn!("Built-in voice: the sound device went away ({err})");
                }
            }
            _ => {
                error_suspect.store(true, Ordering::Release);
                if !warned.swap(true, Ordering::AcqRel) {
                    log::warn!("Built-in voice: the sound device reported an error ({err})");
                }
            }
        },
        None,
    );
    let stream = built
        .and_then(|stream| stream.play().map(|()| stream))
        .map_err(|err| err.to_string())?;
    Ok(Output {
        _stream: stream,
        rate,
        lost,
        suspect,
        beats,
    })
}

/// Checks with no callback before a stream counts as stalled: 2 s at
/// [`CHECK`]. Each reopen in one episode doubles it, up to [`MAX_BACKOFF`]
/// doublings, so a device that is slow to start is never reopened in a
/// tight loop.
const STALL_CHECKS: u32 = 8;
const MAX_BACKOFF: u32 = 3;

/// Whether the stream still asks for sound. A compact copy of
/// `hearken-audio`'s `kira/device/health.rs` (this crate cannot depend on
/// that one); the device-gone errors are handled by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Stall {
    /// The callback count seen at the last check.
    last_beats: u64,
    /// Checks in a row with no callback.
    quiet_checks: u32,
    /// An error that may have ended the stream arrived; one quiet check
    /// settles it, unless this episode already reopened.
    suspect: bool,
    /// Reopens for silence since sound last played.
    stalls: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    /// A new stream started; its callback count starts at zero.
    Opened,
    Check {
        beats: u64,
    },
    /// The stream reported an error other than a lost device, an underrun
    /// or a reroute.
    Suspect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Keep,
    /// Keep, and log that sound came back after a stall.
    Resumed,
    /// Close the stream and open the default device again.
    Reopen,
}

impl Stall {
    fn next(&mut self, event: Event) -> Verdict {
        match event {
            Event::Opened => {
                self.last_beats = 0;
                self.quiet_checks = 0;
                self.suspect = false;
                Verdict::Keep
            }
            Event::Suspect => {
                self.suspect = true;
                Verdict::Keep
            }
            Event::Check { beats } if beats != self.last_beats => {
                self.last_beats = beats;
                self.quiet_checks = 0;
                self.suspect = false;
                if self.stalls > 0 {
                    self.stalls = 0;
                    Verdict::Resumed
                } else {
                    Verdict::Keep
                }
            }
            Event::Check { .. } => {
                self.quiet_checks = self.quiet_checks.saturating_add(1);
                let limit = if self.suspect && self.stalls == 0 {
                    1
                } else {
                    STALL_CHECKS << self.stalls.min(MAX_BACKOFF)
                };
                if self.quiet_checks >= limit {
                    self.quiet_checks = 0;
                    self.stalls = self.stalls.saturating_add(1);
                    Verdict::Reopen
                } else {
                    Verdict::Keep
                }
            }
        }
    }

    /// Reopens for silence since sound last played: 1 on the first.
    fn stalls(&self) -> u32 {
        self.stalls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(last_beats: u64, quiet_checks: u32, suspect: bool, stalls: u32) -> Stall {
        Stall {
            last_beats,
            quiet_checks,
            suspect,
            stalls,
        }
    }

    const fn check(beats: u64) -> Event {
        Event::Check { beats }
    }

    #[test]
    fn transitions() {
        use Verdict::{Keep, Reopen, Resumed};
        #[rustfmt::skip]
        let table: &[(&str, Stall, Event, Verdict, Stall)] = &[
            ("callbacks keep playing", s(4, 3, false, 0), check(9), Keep, s(9, 0, false, 0)),
            ("seven quiet checks keep", s(9, 6, false, 0), check(9), Keep, s(9, 7, false, 0)),
            ("the eighth reopens", s(9, 7, false, 0), check(9), Reopen, s(9, 0, false, 1)),
            ("after one stall it waits 16", s(0, 14, false, 1), check(0), Keep, s(0, 15, false, 1)),
            ("and reopens on the 16th", s(0, 15, false, 1), check(0), Reopen, s(0, 0, false, 2)),
            ("after two it reopens on the 32nd", s(0, 31, false, 2), check(0), Reopen, s(0, 0, false, 3)),
            ("after three it waits 64", s(0, 62, false, 3), check(0), Keep, s(0, 63, false, 3)),
            ("and reopens on the 64th", s(0, 63, false, 3), check(0), Reopen, s(0, 0, false, 4)),
            ("the wait stops growing at 64", s(0, 63, false, 9), check(0), Reopen, s(0, 0, false, 10)),
            ("callbacks after stalls report sound back", s(0, 5, false, 3), check(2), Resumed, s(2, 0, false, 0)),
            ("an error makes the stream suspect", s(9, 0, false, 0), Event::Suspect, Keep, s(9, 0, true, 0)),
            ("a suspect stream reopens on one quiet check", s(9, 0, true, 0), check(9), Reopen, s(9, 0, true, 1)),
            ("a suspect stream that calls back is cleared", s(9, 0, true, 0), check(12), Keep, s(12, 0, false, 0)),
            ("a suspect stream after a reopen waits 16", s(0, 0, true, 1), check(0), Keep, s(0, 1, true, 1)),
            ("and reopens on the 16th", s(0, 15, true, 1), check(0), Reopen, s(0, 0, true, 2)),
            ("a new stream starts its count over", s(800, 5, true, 2), Event::Opened, Keep, s(0, 0, false, 2)),
        ];
        for (name, start, event, verdict, end) in table {
            let mut stall = *start;
            assert_eq!(stall.next(*event), *verdict, "{name}: verdict");
            assert_eq!(stall, *end, "{name}: state");
        }
    }
}
