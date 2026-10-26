//! The thread that owns the model: it loads once, then turns requests into
//! WAV bytes, reading and writing the disk cache on the way.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use crate::cache;
use crate::synth::{Loader, Synth};

/// One line to synthesize.
pub(crate) struct Job {
    pub key: u64,
    pub voice: String,
    pub text: String,
    pub rate: f32,
    /// A line waiting to be said goes ahead of every primed one.
    pub urgent: bool,
}

pub(crate) enum Request {
    Synth(Job),
    /// Drop the urgent lines not yet started (F2); primed lines stay.
    Clear,
}

pub(crate) enum Reply {
    Loaded { speakers: Vec<String> },
    LoadFailed(String),
    Ready { key: u64, wav: Arc<[u8]> },
    Failed { key: u64, error: String },
}

/// The main thread's end. Dropping it lets the thread finish the line it is
/// on and exit; nothing waits for it.
pub(crate) struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
}

impl Worker {
    pub(crate) fn spawn(loader: Loader, cache_dir: Option<PathBuf>, disk_cap: u64) -> Worker {
        let (tx, requests) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("hearken-voice".into())
            .spawn(move || run(loader, cache_dir, disk_cap, requests, replies));
        if let Err(e) = spawned {
            log::error!("voice: could not start the worker thread: {e}");
        }
        Worker { tx, rx }
    }

    /// False when the thread is gone.
    pub(crate) fn send(&self, request: Request) -> bool {
        self.tx.send(request).is_ok()
    }

    /// The next reply, `Ok(None)` when there is none yet, `Err` when the
    /// thread is gone.
    pub(crate) fn try_recv(&self) -> Result<Option<Reply>, ()> {
        match self.rx.try_recv() {
            Ok(reply) => Ok(Some(reply)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(()),
        }
    }
}

fn run(
    loader: Loader,
    cache_dir: Option<PathBuf>,
    disk_cap: u64,
    requests: Receiver<Request>,
    replies: Sender<Reply>,
) {
    let started = std::time::Instant::now();
    let mut synth = match loader() {
        Ok(synth) => synth,
        Err(e) => {
            let _ = replies.send(Reply::LoadFailed(e.to_string()));
            return;
        }
    };
    log::info!(
        "voice: model loaded in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    if replies
        .send(Reply::Loaded {
            speakers: synth.speakers(),
        })
        .is_err()
    {
        return;
    }
    if let Some(dir) = &cache_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::warn!("voice cache: could not create {}: {e}", dir.display());
        }
        cache::trim_dir(dir, disk_cap);
    }

    let mut urgent: VecDeque<Job> = VecDeque::new();
    let mut primed: VecDeque<Job> = VecDeque::new();
    loop {
        if urgent.is_empty() && primed.is_empty() {
            match requests.recv() {
                Ok(request) => take(request, &mut urgent, &mut primed),
                Err(_) => return,
            }
        }
        loop {
            match requests.try_recv() {
                Ok(request) => take(request, &mut urgent, &mut primed),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let Some(job) = urgent.pop_front().or_else(|| primed.pop_front()) else {
            continue;
        };
        let reply = speak(synth.as_mut(), cache_dir.as_deref(), job);
        if replies.send(reply).is_err() {
            return;
        }
    }
}

fn speak(synth: &mut dyn Synth, cache_dir: Option<&std::path::Path>, job: Job) -> Reply {
    let path = cache_dir.map(|dir| dir.join(cache::file_name(job.key)));
    if let Some(path) = &path {
        if let Ok(bytes) = std::fs::read(path) {
            if cache::wav_duration_s(&bytes).is_some() {
                return Reply::Ready {
                    key: job.key,
                    wav: bytes.into(),
                };
            }
            log::warn!(
                "voice cache: {} is damaged; speaking it again",
                path.display()
            );
        }
    }
    let started = std::time::Instant::now();
    let speech = match synth.synthesize(&job.text, &job.voice, job.rate) {
        Ok(speech) => speech,
        Err(e) => {
            return Reply::Failed {
                key: job.key,
                error: e.to_string(),
            }
        }
    };
    log::debug!(
        "voice: {:.2} s for {:.2} s of {:?}",
        started.elapsed().as_secs_f64(),
        speech.duration_s(),
        job.text
    );
    let wav: Arc<[u8]> = speech.to_wav().into();
    if let Some(path) = &path {
        // Through a temporary file, so a crash never leaves half a clip.
        let tmp = path.with_extension("wav.tmp");
        let written = std::fs::write(&tmp, &wav[..]).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(e) = written {
            log::warn!("voice cache: could not write {}: {e}", path.display());
            let _ = std::fs::remove_file(&tmp);
        }
    }
    Reply::Ready { key: job.key, wav }
}

/// Files `request` into the right queue.
fn take(request: Request, urgent: &mut VecDeque<Job>, primed: &mut VecDeque<Job>) {
    match request {
        Request::Clear => urgent.clear(),
        Request::Synth(job) if job.urgent => {
            // Asked for now: the primed copy, if any, is no longer needed.
            primed.retain(|p| p.key != job.key);
            if !urgent.iter().any(|u| u.key == job.key) {
                urgent.push_back(job);
            }
        }
        Request::Synth(job) => {
            if !urgent.iter().chain(primed.iter()).any(|q| q.key == job.key) {
                primed.push_back(job);
            }
        }
    }
}
