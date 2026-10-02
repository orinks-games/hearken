//! Calling a station: one GET, asking for ICY titles, saying who we are and
//! nothing about the player.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::num::NonZeroUsize;
use std::sync::Arc;

use ureq::ErrorKind;
use url::{Position, Url};

use crate::stream::{Fault, Shared, StreamOptions};

const USER_AGENT: &str = concat!("Hearken/", env!("CARGO_PKG_VERSION"));

/// The most a Shoutcast v1 reply's head may hold before it is not one.
const MAX_HEAD: u64 = 16 * 1024;

/// A station that answered: what it says it sends, and the body.
pub(crate) struct Response {
    /// Where the reply came from, after any redirects: a playlist's relative
    /// entries are relative to this.
    pub(crate) url: Url,
    pub(crate) content_type: Option<String>,
    /// Body bytes between ICY title blocks, when the station sends them.
    pub(crate) icy_metaint: Option<NonZeroUsize>,
    /// `Sync` because Symphonia's media sources must be.
    pub(crate) body: Box<dyn Read + Send + Sync>,
}

/// Calls `url`. Its body fails the first read after the stream closes.
pub(crate) fn open(
    url: &str,
    shared: &Arc<Shared>,
    opts: &StreamOptions,
) -> Result<Response, Fault> {
    let mut response = call(url, opts)?;
    response.body = Box::new(UntilClosed {
        body: response.body,
        shared: shared.clone(),
    });
    Ok(response)
}

/// A body that stops once every handle is gone, wherever it is being read
/// from (a probe, a playlist, the decoder): a dropped stream lets go of the
/// socket within one read.
struct UntilClosed {
    body: Box<dyn Read + Send + Sync>,
    shared: Arc<Shared>,
}

impl Read for UntilClosed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.shared.closed() {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        self.body.read(buf)
    }
}

fn call(url: &str, opts: &StreamOptions) -> Result<Response, Fault> {
    let url = Url::parse(url).map_err(|_| bad_url())?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(opts.timeout)
        .timeout_read(opts.timeout)
        .redirects(10)
        .user_agent(USER_AGENT)
        .build();
    match agent
        .request_url("GET", &url)
        .set("Icy-MetaData", "1")
        .call()
    {
        Ok(response) => Ok(Response {
            url: Url::parse(response.get_url()).unwrap_or(url),
            content_type: response.header("content-type").map(str::to_owned),
            icy_metaint: metaint(response.header("icy-metaint")),
            body: response.into_reader(),
        }),
        Err(ureq::Error::Status(code, _)) => Err(status(code)),
        Err(ureq::Error::Transport(error)) => {
            // Where it went wrong: a redirect may have led to an old server.
            let at = error.url().unwrap_or(&url);
            match error.kind() {
                ErrorKind::InvalidUrl | ErrorKind::UnknownScheme => Err(bad_url()),
                // A loop answers the same way however often it is called.
                ErrorKind::TooManyRedirects => Err(Fault::Fatal("too many redirects".into())),
                ErrorKind::BadStatus if at.scheme() == "http" => shoutcast_v1(at, opts),
                kind => Err(Fault::Retry(match io_error(&error) {
                    Some(io) => format!("{kind}: {}", io.kind()),
                    None => kind.to_string(),
                })),
            }
        }
    }
}

/// The socket error under a ureq one, if any.
fn io_error<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a io::Error> {
    let source = error.source()?;
    source.downcast_ref().or_else(|| io_error(source))
}

pub(crate) fn bad_url() -> Fault {
    Fault::Fatal("bad URL".into())
}

fn status(code: u16) -> Fault {
    let reason = format!("HTTP {code}");
    match code {
        500.. => Fault::Retry(reason),
        _ => Fault::Fatal(reason),
    }
}

fn metaint(value: Option<&str>) -> Option<NonZeroUsize> {
    value?.trim().parse().ok()
}

/// Calls again over a plain socket, for an old Shoutcast v1 server: it
/// answers `ICY 200 OK`, which ureq will not take.
fn shoutcast_v1(url: &Url, opts: &StreamOptions) -> Result<Response, Fault> {
    let mut error = io::Error::from(io::ErrorKind::AddrNotAvailable);
    for addr in url.socket_addrs(|| Some(80))? {
        match TcpStream::connect_timeout(&addr, opts.timeout) {
            Ok(socket) => return request(socket, url, opts),
            Err(e) => error = e,
        }
    }
    Err(error.into())
}

/// An HTTP/1.0 GET with the same headers and timeouts as ureq's, taking a
/// reply that starts `ICY 200` or `HTTP/1.x 200`.
fn request(mut socket: TcpStream, url: &Url, opts: &StreamOptions) -> Result<Response, Fault> {
    socket.set_read_timeout(Some(opts.timeout))?;
    socket.set_write_timeout(Some(opts.timeout))?;
    write!(
        socket,
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: {USER_AGENT}\r\nIcy-MetaData: 1\r\n\r\n",
        &url[Position::BeforePath..Position::AfterQuery],
        &url[Position::BeforeHost..Position::AfterPort],
    )?;
    let mut reader = BufReader::new(socket);
    let mut head = Vec::new();
    let mut limited = (&mut reader).take(MAX_HEAD);
    loop {
        let start = head.len();
        if limited.read_until(b'\n', &mut head)? == 0 {
            return Err(Fault::Retry("no reply".into()));
        }
        if head[start..].trim_ascii().is_empty() {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head);
    let mut lines = head.lines();
    let bad = || Fault::Retry("bad status line".into());
    let mut status_line = lines.next().unwrap_or("").split_whitespace();
    let (Some(version), Some(code)) = (status_line.next(), status_line.next()) else {
        return Err(bad());
    };
    if version != "ICY" && !version.starts_with("HTTP/1.") {
        return Err(bad());
    }
    match code.parse() {
        Ok(200) => {}
        Ok(code) => return Err(status(code)),
        Err(_) => return Err(bad()),
    }
    let header = |name: &str| {
        lines.clone().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
        })
    };
    Ok(Response {
        url: url.clone(),
        content_type: header("content-type").map(str::to_owned),
        icy_metaint: metaint(header("icy-metaint")),
        body: Box::new(reader),
    })
}
