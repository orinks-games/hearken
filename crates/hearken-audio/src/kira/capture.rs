//! Recording what the player hears: a tap on Kira's main mix, after every
//! bus and effect, written to a 16-bit stereo WAV file by a writer thread.
//!
//! The tap costs one atomic load per audio block while nothing records.
//! While recording, the audio thread pushes frames into a ring buffer and
//! never waits; a full ring drops frames (counted) rather than stall sound.

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;
use kira::Frame;
use rtrb::{Consumer, Producer, RingBuffer};

/// About five seconds at 48 kHz: the writer drains it every 10 ms.
const RING_FRAMES: usize = 1 << 18;

#[derive(Default)]
struct Shared {
    on: AtomicBool,
    rate: AtomicU32,
    dropped: AtomicU64,
}

/// Goes on the main track when the manager is built.
pub(crate) struct TapBuilder {
    shared: Arc<Shared>,
    producer: Producer<[f32; 2]>,
}

impl EffectBuilder for TapBuilder {
    type Handle = ();

    fn build(self) -> (Box<dyn Effect>, ()) {
        (
            Box::new(Tap {
                shared: self.shared,
                producer: self.producer,
            }),
            (),
        )
    }
}

struct Tap {
    shared: Arc<Shared>,
    producer: Producer<[f32; 2]>,
}

impl Effect for Tap {
    fn init(&mut self, sample_rate: u32, _internal_buffer_size: usize) {
        self.shared.rate.store(sample_rate, Ordering::Relaxed);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.shared.rate.store(sample_rate, Ordering::Relaxed);
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        if !self.shared.on.load(Ordering::Relaxed) {
            return;
        }
        for frame in input.iter() {
            if self.producer.push([frame.left, frame.right]).is_err() {
                self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

type Writer = JoinHandle<(Consumer<[f32; 2]>, io::Result<PathBuf>)>;

/// The game side of the tap: starts and stops recordings.
pub(crate) struct Capture {
    shared: Arc<Shared>,
    /// Held here between recordings, and by the writer during one.
    consumer: Option<Consumer<[f32; 2]>>,
    writer: Option<(Arc<AtomicBool>, Writer)>,
}

/// A tap for the main track and the capture that reads it.
pub(crate) fn tap() -> (TapBuilder, Capture) {
    let shared = Arc::new(Shared::default());
    let (producer, consumer) = RingBuffer::new(RING_FRAMES);
    (
        TapBuilder {
            shared: shared.clone(),
            producer,
        },
        Capture {
            shared,
            consumer: Some(consumer),
            writer: None,
        },
    )
}

impl Capture {
    /// Starts writing the mix to a WAV file at `path`.
    pub(crate) fn start(&mut self, path: &Path) -> Result<(), String> {
        if self.writer.is_some() {
            return Err("already recording".into());
        }
        let mut consumer = self.consumer.take().ok_or("the recorder is busy")?;
        // Frames pushed after the last recording's writer finished.
        while consumer.pop().is_ok() {}
        let file = match File::create(path) {
            Ok(f) => f,
            Err(e) => {
                self.consumer = Some(consumer);
                return Err(format!("could not create {}: {e}", path.display()));
            }
        };
        let rate = match self.shared.rate.load(Ordering::Relaxed) {
            0 => 48_000,
            r => r,
        };
        self.shared.dropped.store(0, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let path = path.to_path_buf();
        let handle = std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let result = write_wav(file, rate, &mut consumer, &thread_stop).map(|()| path);
                (consumer, result)
            })
            .map_err(|e| format!("could not start the recorder: {e}"))?;
        self.writer = Some((stop, handle));
        self.shared.on.store(true, Ordering::Relaxed);
        log::info!("audio capture: recording at {rate} Hz");
        Ok(())
    }

    /// Stops recording and finishes the file; `None` when not recording.
    pub(crate) fn stop(&mut self) -> Option<Result<PathBuf, String>> {
        let (stop, handle) = self.writer.take()?;
        self.shared.on.store(false, Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
        // ponytail: joins on the caller's thread; the writer only drains what
        // is left (at most RING_FRAMES) and patches the header, so it is short.
        let (consumer, result) = match handle.join() {
            Ok(done) => done,
            Err(_) => return Some(Err("the recorder stopped unexpectedly".into())),
        };
        self.consumer = Some(consumer);
        let dropped = self.shared.dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            log::warn!("audio capture: {dropped} frames dropped (the writer fell behind)");
        }
        Some(result.map_err(|e| format!("could not finish the recording: {e}")))
    }
}

/// Writes `consumer` to `file` as 16-bit stereo PCM until `stop` is set and
/// the ring is empty, then fills in the header's sizes.
fn write_wav(
    file: File,
    rate: u32,
    consumer: &mut Consumer<[f32; 2]>,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut out = BufWriter::new(file);
    write_header(&mut out, rate, 0)?;
    let mut frames: u64 = 0;
    loop {
        let stopping = stop.load(Ordering::Relaxed);
        let mut any = false;
        while let Ok([l, r]) = consumer.pop() {
            out.write_all(&sample(l).to_le_bytes())?;
            out.write_all(&sample(r).to_le_bytes())?;
            frames += 1;
            any = true;
        }
        if stopping {
            break;
        }
        if !any {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let data_bytes = u32::try_from(frames * 4).unwrap_or(u32::MAX);
    out.seek(SeekFrom::Start(0))?;
    write_header(&mut out, rate, data_bytes)?;
    out.flush()
}

fn sample(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

fn write_header(out: &mut impl Write, rate: u32, data_bytes: u32) -> io::Result<()> {
    out.write_all(b"RIFF")?;
    out.write_all(&data_bytes.saturating_add(36).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?; // PCM
    out.write_all(&2u16.to_le_bytes())?; // stereo
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * 4).to_le_bytes())?; // bytes a second
    out.write_all(&4u16.to_le_bytes())?; // bytes a frame
    out.write_all(&16u16.to_le_bytes())?; // bits a sample
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recording_writes_a_wav_with_what_the_tap_heard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take.wav");
        let (builder, mut capture) = tap();
        let (mut effect, ()) = builder.build();
        effect.init(44_100, 512);
        let info = kira::info::MockInfoBuilder::new().build();
        let mut frames = [Frame::from_mono(0.5); 100];
        // Nothing is kept while nothing records.
        effect.process(&mut frames, 1.0 / 44_100.0, &info);
        capture.start(&path).unwrap();
        assert!(capture.start(&path).is_err(), "one recording at a time");
        effect.process(&mut frames, 1.0 / 44_100.0, &info);
        let done = capture.stop().unwrap().unwrap();
        assert_eq!(done, path);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            44_100
        );
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 400);
        assert_eq!(bytes.len(), 44 + 400);
        assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 16383);
        assert!(capture.stop().is_none());
    }
}
