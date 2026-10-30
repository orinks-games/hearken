//! How bytes reach the updater. The engine's own client speaks HTTPS
//! through ureq (the `http` feature); tests and games with their own
//! transport hand in anything that implements [`Fetch`].

use std::io::Read;

/// One GET's failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// The server answered with this status.
    Status(u16),
    /// No answer: no network, a refused connection, a timeout.
    Offline(String),
    /// The body was longer than allowed.
    TooLarge,
    /// The URL is not one the updater fetches: not HTTPS.
    NotAllowed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Status(code) => write!(f, "the server answered {code}"),
            FetchError::Offline(why) => write!(f, "no answer: {why}"),
            FetchError::TooLarge => f.write_str("the answer was too large"),
            FetchError::NotAllowed(url) => write!(f, "not an HTTPS address: {url}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// A body being read: its declared length, when the server gave one.
pub struct Body {
    pub length: Option<u64>,
    pub reader: Box<dyn Read + Send>,
}

/// The answer to a request for part of a file, as the server gave it. The
/// updater checks all of it ([`crate::partial`]); anything but exactly the
/// part asked for means the whole file is downloaded instead.
pub struct RangeBody {
    /// The status: 206 for the part asked for. A server that ignores the
    /// range answers 200 with the whole file.
    pub status: u16,
    /// The `Content-Range` header, as sent: `bytes 100-199/5000`.
    pub content_range: Option<String>,
    /// The declared length, when the server gave one.
    pub length: Option<u64>,
    pub reader: Box<dyn Read + Send>,
}

/// GET requests.
pub trait Fetch: Send + Sync {
    /// The whole body of `url`, refusing more than `limit` bytes.
    fn get(&self, url: &str, headers: &[(&str, &str)], limit: u64) -> Result<Vec<u8>, FetchError>;

    /// `url`'s body as a stream, for a download too big to hold. The reader
    /// should fail a read that waits too long rather than block for good.
    fn open(&self, url: &str) -> Result<Body, FetchError>;

    /// Whether [`Fetch::open_range`] asks servers for parts of files. Off
    /// unless a `Fetch` says so, so one written before ranges existed is
    /// never offered a smaller download it cannot make.
    fn supports_ranges(&self) -> bool {
        false
    }

    /// Part of `url`, the bytes in `range` (end exclusive), as a stream,
    /// with what the server said about it; `Ok(None)` when this `Fetch`
    /// does not ask for parts. Like [`Fetch::open`], a read should fail
    /// rather than wait for good.
    fn open_range(
        &self,
        url: &str,
        range: std::ops::Range<u64>,
    ) -> Result<Option<RangeBody>, FetchError> {
        let _ = (url, range);
        Ok(None)
    }
}

/// Whether the updater fetches `url`: HTTPS anywhere, or plain HTTP to this
/// computer for tests and a developer's own server. Every manifest is
/// signed, so this is about privacy and plain sense, not trust.
pub fn url_allowed(url: &str) -> bool {
    if url.contains(char::is_whitespace) || url.contains(['@', '#']) {
        return false;
    }
    let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|base| {
            url.strip_prefix(base)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        });
    let https = url
        .strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
    https || local
}

/// Reads all of `reader`, failing once it passes `limit` bytes.
pub fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, FetchError> {
    let mut out = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| FetchError::Offline(e.to_string()))?;
    if out.len() as u64 > limit {
        return Err(FetchError::TooLarge);
    }
    Ok(out)
}

#[cfg(feature = "http")]
pub use http::HttpFetch;

#[cfg(feature = "http")]
mod http {
    use std::time::Duration;

    use super::{read_limited, url_allowed, Body, Fetch, FetchError, RangeBody};

    /// How long a small request may take in all.
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
    /// How long connecting may take.
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

    /// HTTPS through ureq, with the system's certificates.
    pub struct HttpFetch {
        small: ureq::Agent,
        download: ureq::Agent,
    }

    impl Default for HttpFetch {
        fn default() -> Self {
            let user_agent = concat!("hearken-update/", env!("CARGO_PKG_VERSION"));
            HttpFetch {
                // Redirects are followed by `call`, which checks each one.
                small: ureq::AgentBuilder::new()
                    .redirects(0)
                    .timeout_connect(CONNECT_TIMEOUT)
                    .timeout(REQUEST_TIMEOUT)
                    .user_agent(user_agent)
                    .build(),
                // No total deadline: a big archive on a slow line takes as
                // long as it takes. Each read gives up after the idle bound,
                // so a connection that goes quiet still ends.
                download: ureq::AgentBuilder::new()
                    .redirects(0)
                    .timeout_connect(CONNECT_TIMEOUT)
                    .timeout_read(crate::download::IDLE_TIMEOUT)
                    .user_agent(user_agent)
                    .build(),
            }
        }
    }

    /// How many redirects one request follows.
    const MAX_REDIRECTS: usize = 5;

    /// GETs `url` on `agent`, following redirects by hand. ureq would
    /// follow them itself and only show where it landed after it had
    /// already sent the request there; here every hop's address is checked
    /// before anything is sent to it (see [`hop_allowed`]), so a redirect
    /// can never make the game reach a plain HTTP server, this computer, or
    /// an address on the player's network written as an IP.
    fn call(
        agent: &ureq::Agent,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<ureq::Response, FetchError> {
        let mut current = url.to_string();
        let mut same_origin = true;
        for _ in 0..=MAX_REDIRECTS {
            if !url_allowed(&current) {
                return Err(FetchError::NotAllowed(current));
            }
            let mut request = agent.get(&current);
            for (name, value) in headers {
                if same_origin || !is_credential(name) {
                    request = request.set(name, value);
                }
            }
            let response = match request.call() {
                Ok(response) => response,
                Err(ureq::Error::Status(code, _)) => return Err(FetchError::Status(code)),
                Err(e) => return Err(FetchError::Offline(e.kind().to_string())),
            };
            let status = response.status();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                if (300..400).contains(&status) {
                    return Err(FetchError::Status(status));
                }
                return Ok(response);
            }
            let next = response
                .header("location")
                .and_then(|location| redirect_target(&current, location))
                .ok_or_else(|| FetchError::Offline("a redirect to no usable address".into()))?;
            if !hop_allowed(url, &next) {
                return Err(FetchError::NotAllowed(next));
            }
            same_origin = same_origin && origin(&current) == origin(&next);
            current = next;
        }
        Err(FetchError::Offline("too many redirects".into()))
    }

    /// Where a redirect's `Location` points, read against the address that
    /// sent it (it may be relative).
    pub(super) fn redirect_target(from: &str, location: &str) -> Option<String> {
        let base = url::Url::parse(from).ok()?;
        Some(base.join(location.trim()).ok()?.to_string())
    }

    /// Whether a request that began at `start` may be redirected to
    /// `next`. Besides [`url_allowed`], a server elsewhere may not send the
    /// game on to this computer or to a private, link-local or loopback IP
    /// address: it could otherwise make the game send requests to the
    /// player's router or a service on their computer. A request that began
    /// on such an address (a developer's own test server) may stay there.
    pub(super) fn hop_allowed(start: &str, next: &str) -> bool {
        url_allowed(next) && (near(start) || !near(next))
    }

    /// Whether `address`'s host is this computer or an IP address on a private
    /// network. A name other than `localhost` is not looked up.
    fn near(address: &str) -> bool {
        use std::net::{Ipv4Addr, Ipv6Addr};
        fn near_v4(ip: Ipv4Addr) -> bool {
            let [a, b, ..] = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                // Carrier-grade NAT, 100.64.0.0/10.
                || (a == 100 && (b & 0xc0) == 64)
        }
        fn near_v6(ip: Ipv6Addr) -> bool {
            let first = ip.segments()[0];
            ip.is_loopback()
                || ip.is_unspecified()
                // Unique local (fc00::/7) and link-local (fe80::/10).
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || ip.to_ipv4_mapped().is_some_and(near_v4)
        }
        let Ok(parsed) = url::Url::parse(address) else {
            // Unreadable: treat it as near, so it is never a redirect target.
            return true;
        };
        match parsed.host() {
            Some(url::Host::Domain(name)) => {
                let name = name.trim_end_matches('.').to_ascii_lowercase();
                name == "localhost" || name.ends_with(".localhost")
            }
            Some(url::Host::Ipv4(ip)) => near_v4(ip),
            Some(url::Host::Ipv6(ip)) => near_v6(ip),
            None => true,
        }
    }

    /// The scheme, host and port of `url`.
    fn origin(url: &str) -> Option<url::Origin> {
        url::Url::parse(url).ok().map(|u| u.origin())
    }

    /// Headers that are never sent on to another server.
    fn is_credential(name: &str) -> bool {
        ["authorization", "proxy-authorization", "cookie"]
            .iter()
            .any(|c| name.eq_ignore_ascii_case(c))
    }

    impl Fetch for HttpFetch {
        fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            limit: u64,
        ) -> Result<Vec<u8>, FetchError> {
            let response = call(&self.small, url, headers)?;
            let declared = response
                .header("content-length")
                .and_then(|v| v.trim().parse::<u64>().ok());
            if declared.is_some_and(|n| n > limit) {
                return Err(FetchError::TooLarge);
            }
            read_limited(response.into_reader(), limit)
        }

        fn open(&self, url: &str) -> Result<Body, FetchError> {
            let response = call(&self.download, url, &[])?;
            let length = response
                .header("content-length")
                .and_then(|v| v.trim().parse::<u64>().ok());
            Ok(Body {
                length,
                reader: Box::new(response.into_reader()),
            })
        }

        fn supports_ranges(&self) -> bool {
            true
        }

        fn open_range(
            &self,
            url: &str,
            range: std::ops::Range<u64>,
        ) -> Result<Option<RangeBody>, FetchError> {
            if range.is_empty() {
                return Err(FetchError::Offline("an empty range".into()));
            }
            let wanted = format!("bytes={}-{}", range.start, range.end - 1);
            // The bytes as stored: a range of a compressed answer would be
            // a range of other bytes.
            let response = call(
                &self.download,
                url,
                &[("Range", &wanted), ("Accept-Encoding", "identity")],
            )?;
            let length = response
                .header("content-length")
                .and_then(|v| v.trim().parse::<u64>().ok());
            Ok(Some(RangeBody {
                status: response.status(),
                content_range: response.header("content-range").map(str::to_string),
                length,
                reader: Box::new(response.into_reader()),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_this_computer() {
        for url in [
            "https://example.com/u/update-stable-linux-x64.json",
            "https://api.github.com/repos/a/b/releases",
            "http://127.0.0.1:8080/x.json",
            "http://localhost/x.json",
        ] {
            assert!(url_allowed(url), "{url}");
        }
        for url in [
            "http://example.com/x.json",
            "ftp://example.com/x",
            "file:///etc/passwd",
            "https://",
            "https:///x",
            "http://localhost.evil.com/x",
            "https://user@example.com/x",
            "https://example.com/a b",
        ] {
            assert!(!url_allowed(url), "{url}");
        }
    }

    #[cfg(feature = "http")]
    #[test]
    fn redirects_are_read_against_the_address_that_sent_them() {
        use super::http::redirect_target;
        let from = "https://example.com/releases/v1/update.json";
        for (location, to) in [
            (
                "https://cdn.example.net/a.zip",
                "https://cdn.example.net/a.zip",
            ),
            ("/other/a.zip", "https://example.com/other/a.zip"),
            ("a.zip", "https://example.com/releases/v1/a.zip"),
            ("//cdn.example.net/a.zip", "https://cdn.example.net/a.zip"),
        ] {
            assert_eq!(redirect_target(from, location).as_deref(), Some(to));
        }
        // A plain HTTP target is still read; `call` refuses it before
        // anything is sent there.
        let plain = redirect_target(from, "http://192.168.1.1/admin").unwrap();
        assert!(!url_allowed(&plain));
    }

    #[cfg(feature = "http")]
    #[test]
    fn a_server_elsewhere_cannot_redirect_to_this_computer_or_its_network() {
        use super::http::hop_allowed;
        let from = "https://example.com/u/update.json";
        for to in [
            "http://127.0.0.1:8080/x",
            "http://localhost/x",
            "https://localhost/x",
            "https://127.0.0.1/x",
            "https://0x7f.1/x",
            "https://10.0.0.1/x",
            "https://192.168.1.1/admin",
            "https://172.16.5.4/x",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/x",
            "https://[::1]/x",
            "https://[fd00::1]/x",
            "https://[fe80::1]/x",
            "https://[::ffff:192.168.1.1]/x",
            "https://router.localhost/x",
        ] {
            assert!(!hop_allowed(from, to), "{to}");
        }
        for to in [
            "https://cdn.example.net/a.zip",
            "https://8.8.8.8/a.zip",
            "https://objects.githubusercontent.com/a.zip",
        ] {
            assert!(hop_allowed(from, to), "{to}");
        }
        // A developer's own server on this computer may redirect within it.
        assert!(hop_allowed(
            "http://127.0.0.1:8080/a.json",
            "http://127.0.0.1:8080/b.json"
        ));
        assert!(hop_allowed(
            "http://localhost:8080/a.json",
            "https://cdn.example.net/a.zip"
        ));
    }

    #[test]
    fn reading_stops_at_the_limit() {
        assert_eq!(read_limited(&b"hello"[..], 5), Ok(b"hello".to_vec()));
        assert_eq!(read_limited(&b"hello!"[..], 5), Err(FetchError::TooLarge));
    }
}
