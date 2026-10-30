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

/// GET requests.
pub trait Fetch: Send + Sync {
    /// The whole body of `url`, refusing more than `limit` bytes.
    fn get(&self, url: &str, headers: &[(&str, &str)], limit: u64) -> Result<Vec<u8>, FetchError>;

    /// `url`'s body as a stream, for a download too big to hold. The reader
    /// should fail a read that waits too long rather than block for good.
    fn open(&self, url: &str) -> Result<Body, FetchError>;
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

    use super::{read_limited, url_allowed, Body, Fetch, FetchError};

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
    /// before anything is sent to it, so a redirect can never make the game
    /// reach a plain HTTP server, or a computer on the player's network.
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

    #[test]
    fn reading_stops_at_the_limit() {
        assert_eq!(read_limited(&b"hello"[..], 5), Ok(b"hello".to_vec()));
        assert_eq!(read_limited(&b"hello!"[..], 5), Err(FetchError::TooLarge));
    }
}
