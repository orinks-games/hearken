//! A station on a local port: a small HTTP server inside the tests that
//! serves canned replies, so no radio test touches the real network.

// Each test file uses its own share of this.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hearken_radio::{Stream, StreamOptions, StreamState};

/// What the server answers a request with.
pub enum Reply {
    /// Status, Content-Type and body, then close.
    Body(u16, &'static str, Vec<u8>),
    /// 200, the body repeated forever; with title bytes, an ICY block every
    /// 8_192 body bytes when the request asks for one.
    Endless(&'static str, Vec<u8>, Option<Vec<u8>>),
    /// Like `Endless` without ICY, closing after this many body bytes.
    Cut(&'static str, Vec<u8>, usize),
    /// Like `Cut`, but going silent after this many body bytes, the
    /// connection held open.
    Stall(&'static str, Vec<u8>, usize),
    /// Like `Endless` without ICY, as fast as the client takes it.
    Flood(&'static str, Vec<u8>),
    /// Accept, read the request, send nothing.
    Hang,
    /// A Shoutcast v1 server: `ICY 200 OK`, `audio/mpeg`, the body repeated
    /// forever.
    IcyV1(Vec<u8>),
    /// Status and `Location`, no body, then close.
    Redirect(u16, String),
    /// 200 and the body, gzipped with `Content-Encoding` when the request
    /// offers to take gzip, plain when it does not; then close.
    Gzip(&'static str, Vec<u8>),
}

/// How many body bytes go out at a time, and how often: 32 KiB/s, four
/// times a 64 kbps station, so a test buffers in a fraction of a second.
const PACE: (usize, Duration) = (4_096, Duration::from_millis(125));
const METAINT: usize = 8_192;

pub struct TestServer {
    port: u16,
    log: Arc<Log>,
}

#[derive(Default)]
struct Log {
    hits: Mutex<HashMap<String, usize>>,
    /// Bytes written after the head, by path.
    sent: Mutex<HashMap<String, usize>>,
    last: Mutex<String>,
    open: AtomicUsize,
}

impl TestServer {
    pub fn start(route: impl Fn(&str) -> Reply + Send + Sync + 'static) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Log::default());
        let route = Arc::new(route);
        let accepting = log.clone();
        thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let (log, route) = (accepting.clone(), route.clone());
                log.open.fetch_add(1, Ordering::SeqCst);
                thread::spawn(move || {
                    // A client that hangs up mid-reply is the usual way out.
                    let _ = serve(socket, &log, &*route);
                    log.open.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        TestServer { port, log }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn hits(&self, path: &str) -> usize {
        self.log
            .hits
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    pub fn open_connections(&self) -> usize {
        self.log.open.load(Ordering::SeqCst)
    }

    /// Bytes sent after the head for `path` (ICY blocks included), across
    /// every request for it.
    pub fn bytes_sent(&self, path: &str) -> usize {
        self.log
            .sent
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .unwrap_or(0)
    }

    /// The raw head of the last request.
    pub fn last_request(&self) -> String {
        self.log.last.lock().unwrap().clone()
    }
}

fn serve(socket: TcpStream, log: &Log, route: &dyn Fn(&str) -> Reply) -> io::Result<()> {
    let mut reader = BufReader::new(&socket);
    let mut head = String::new();
    while reader.read_line(&mut head)? > 2 {}
    let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
    // The head first, so a test that sees the hit can read it.
    *log.last.lock().unwrap() = head.clone();
    *log.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
    let mut out = &socket;
    let sent = |n: usize| *log.sent.lock().unwrap().entry(path.clone()).or_default() += n;
    match route(&path) {
        Reply::Body(status, content_type, body) => {
            let reason = if status < 400 { "OK" } else { "Error" };
            write!(
                out,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            out.write_all(&body)?;
            sent(body.len());
            Ok(())
        }
        Reply::Endless(content_type, body, title) => {
            let asked = head.to_ascii_lowercase().contains("icy-metadata: 1");
            let title = title.filter(|_| asked);
            let metaint = match title {
                Some(_) => format!("icy-metaint: {METAINT}\r\n"),
                None => String::new(),
            };
            write!(
                out,
                "HTTP/1.0 200 OK\r\nContent-Type: {content_type}\r\n{metaint}\r\n"
            )?;
            paced(out, &body, None, title.as_deref(), PACE.1, &sent)
        }
        Reply::Cut(content_type, body, n) => {
            write!(
                out,
                "HTTP/1.0 200 OK\r\nContent-Type: {content_type}\r\n\r\n"
            )?;
            paced(out, &body, Some(n), None, PACE.1, &sent)
        }
        Reply::Stall(content_type, body, n) => {
            write!(
                out,
                "HTTP/1.0 200 OK\r\nContent-Type: {content_type}\r\n\r\n"
            )?;
            paced(out, &body, Some(n), None, PACE.1, &sent)?;
            io::copy(&mut reader, &mut io::sink()).map(drop)
        }
        Reply::Flood(content_type, body) => {
            write!(
                out,
                "HTTP/1.0 200 OK\r\nContent-Type: {content_type}\r\n\r\n"
            )?;
            paced(out, &body, None, None, Duration::ZERO, &sent)
        }
        Reply::Hang => io::copy(&mut reader, &mut io::sink()).map(drop),
        Reply::IcyV1(body) => {
            write!(out, "ICY 200 OK\r\ncontent-type: audio/mpeg\r\n\r\n")?;
            paced(out, &body, None, None, PACE.1, &sent)
        }
        Reply::Gzip(content_type, body) => {
            let offered = head.to_ascii_lowercase().contains("accept-encoding: gzip");
            let (coding, body) = if offered {
                let mut encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(&body)?;
                ("Content-Encoding: gzip\r\n", encoder.finish()?)
            } else {
                ("", body)
            };
            write!(
                out,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{coding}\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            out.write_all(&body)?;
            sent(body.len());
            Ok(())
        }
        Reply::Redirect(status, location) => write!(
            out,
            "HTTP/1.1 {status} Found\r\nLocation: {location}\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n"
        ),
    }
}

/// Sends `body` over and over, [`PACE`]'s chunk every `pause`, until
/// `limit` body bytes have gone or the client hangs up, with an ICY block
/// after every [`METAINT`] body bytes when there is a title, telling `sent`
/// what each write took.
fn paced(
    mut out: &TcpStream,
    body: &[u8],
    limit: Option<usize>,
    title: Option<&[u8]>,
    pause: Duration,
    sent: &dyn Fn(usize),
) -> io::Result<()> {
    let mut bytes = body.iter().copied().cycle();
    let mut body_bytes = 0;
    loop {
        let mut chunk = Vec::new();
        for _ in 0..PACE.0 {
            if limit == Some(body_bytes) {
                out.write_all(&chunk)?;
                sent(chunk.len());
                return Ok(());
            }
            chunk.push(bytes.next().unwrap());
            body_bytes += 1;
            if let Some(title) = title.filter(|_| body_bytes % METAINT == 0) {
                chunk.extend(icy_block(title));
            }
        }
        out.write_all(&chunk)?;
        sent(chunk.len());
        thread::sleep(pause);
    }
}

/// `StreamTitle='<title>';` zero-padded to a multiple of 16, behind a
/// length byte counting 16-byte units.
fn icy_block(title: &[u8]) -> Vec<u8> {
    let parts: [&[u8]; 3] = [b"StreamTitle='", title, b"';"];
    let mut text = parts.concat();
    let units = text.len().div_ceil(16);
    text.resize(units * 16, 0);
    [vec![units as u8], text].concat()
}

/// Short timeouts and buffers, so failures and retries happen in test time.
pub fn fast() -> StreamOptions {
    StreamOptions {
        timeout: Duration::from_secs(1),
        retry_delays: vec![Duration::from_millis(50); 2],
        healthy_after: Duration::from_secs(1),
        stall: Duration::from_secs(1),
        prebuffer: Duration::from_millis(200),
        buffer: Duration::from_secs(1),
        ..StreamOptions::default()
    }
}

/// Polls `done` until it holds, or panics naming `what` after `within`.
pub fn wait_for(what: &str, within: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !done() {
        assert!(Instant::now() < deadline, "{what}: not within {within:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn playing(stream: &Stream) -> bool {
    stream.state() == StreamState::Playing
}

pub fn failed(stream: &Stream) -> Option<String> {
    match stream.state() {
        StreamState::Failed(reason) => Some(reason),
        _ => None,
    }
}

/// Stands in for the audio thread: drains the feed every 10 ms, adding up
/// how long the audio heard lasts, until dropped.
pub struct Listener {
    stop: Arc<AtomicBool>,
    heard_ns: Arc<AtomicU64>,
}

impl Listener {
    /// How much audio this listener has heard.
    pub fn heard(&self) -> Duration {
        Duration::from_nanos(self.heard_ns.load(Ordering::SeqCst))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub fn listen(stream: &Stream) -> Listener {
    let feed = stream.feed();
    let stop = Arc::new(AtomicBool::new(false));
    let heard_ns = Arc::new(AtomicU64::new(0));
    let (stopped, heard) = (stop.clone(), heard_ns.clone());
    thread::spawn(move || {
        let mut out = [[0.0; 2]; 4_096];
        while !stopped.load(Ordering::SeqCst) {
            loop {
                let read = feed.read(&mut out);
                if read.frames == 0 {
                    break;
                }
                let ns = read.frames as u64 * 1_000_000_000 / u64::from(read.sample_rate);
                heard.fetch_add(ns, Ordering::SeqCst);
            }
            thread::sleep(Duration::from_millis(10));
        }
    });
    Listener { stop, heard_ns }
}

/// Everything the stream's feed has buffered right now.
pub fn drain(stream: &Stream) -> Vec<[f32; 2]> {
    let feed = stream.feed();
    let mut out = [[0.0; 2]; 4_096];
    let mut frames = Vec::new();
    loop {
        let read = feed.read(&mut out);
        if read.frames == 0 {
            return frames;
        }
        frames.extend_from_slice(&out[..read.frames]);
    }
}

pub fn rms(frames: &[[f32; 2]]) -> f32 {
    let sum: f32 = frames.iter().map(|[l, r]| l * l + r * r).sum();
    (sum / (2 * frames.len()) as f32).sqrt()
}
