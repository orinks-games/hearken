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
                small: ureq::AgentBuilder::new()
                    .timeout_connect(CONNECT_TIMEOUT)
                    .timeout(REQUEST_TIMEOUT)
                    .user_agent(user_agent)
                    .build(),
                // No total deadline: a big archive on a slow line takes as
                // long as it takes. Each read gives up after the idle bound,
                // so a connection that goes quiet still ends.
                download: ureq::AgentBuilder::new()
                    .timeout_connect(CONNECT_TIMEOUT)
                    .timeout_read(crate::download::IDLE_TIMEOUT)
                    .user_agent(user_agent)
                    .build(),
            }
        }
    }

    fn call(request: ureq::Request) -> Result<ureq::Response, FetchError> {
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, _)) => return Err(FetchError::Status(code)),
            Err(e) => return Err(FetchError::Offline(e.kind().to_string())),
        };
        // A redirect may land anywhere; it must still be HTTPS.
        if !url_allowed(response.get_url()) {
            return Err(FetchError::NotAllowed(response.get_url().to_string()));
        }
        Ok(response)
    }

    impl Fetch for HttpFetch {
        fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            limit: u64,
        ) -> Result<Vec<u8>, FetchError> {
            if !url_allowed(url) {
                return Err(FetchError::NotAllowed(url.to_string()));
            }
            let mut request = self.small.get(url);
            for (name, value) in headers {
                request = request.set(name, value);
            }
            let response = call(request)?;
            let declared = response
                .header("content-length")
                .and_then(|v| v.trim().parse::<u64>().ok());
            if declared.is_some_and(|n| n > limit) {
                return Err(FetchError::TooLarge);
            }
            read_limited(response.into_reader(), limit)
        }

        fn open(&self, url: &str) -> Result<Body, FetchError> {
            if !url_allowed(url) {
                return Err(FetchError::NotAllowed(url.to_string()));
            }
            let response = call(self.download.get(url))?;
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

    #[test]
    fn reading_stops_at_the_limit() {
        assert_eq!(read_limited(&b"hello"[..], 5), Ok(b"hello".to_vec()));
        assert_eq!(read_limited(&b"hello!"[..], 5), Err(FetchError::TooLarge));
    }
}
