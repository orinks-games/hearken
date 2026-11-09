//! How requests reach the server: HTTPS through ureq in the engine, or a
//! scripted transport in a test.

use serde_json::Value;

/// What a server answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// The JSON body, or `Null` when there was none.
    pub body: Value,
}

/// Sends one request at a time, from a worker thread. `Err` only when no
/// answer came at all (no network, a timeout); an HTTP error is a `Reply`.
pub trait Transport: Send + Sync + 'static {
    /// POSTs `body` as JSON to `url`, with the token as `Authorization: Bearer`.
    fn post(&self, url: &str, token: Option<&str>, body: &Value) -> Result<Reply, String>;
    /// GETs `url`, with the token when there is one.
    fn get(&self, url: &str, token: Option<&str>) -> Result<Reply, String>;
}

/// The longest answer read from a server.
pub const MAX_REPLY_BYTES: u64 = 1024 * 1024;

/// HTTPS through ureq. Redirects are not followed, so a token is only ever
/// sent to the URL the game names.
#[cfg(feature = "http")]
pub struct HttpTransport {
    agent: ureq::Agent,
}

#[cfg(feature = "http")]
impl Default for HttpTransport {
    fn default() -> Self {
        use std::time::Duration;
        HttpTransport {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirects(0)
                .user_agent(concat!("hearken-rooms/", env!("CARGO_PKG_VERSION")))
                .build(),
        }
    }
}

#[cfg(feature = "http")]
impl Transport for HttpTransport {
    fn post(&self, url: &str, token: Option<&str>, body: &Value) -> Result<Reply, String> {
        let mut request = self
            .agent
            .post(url)
            .set("Accept", "application/json")
            .set("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        match request.send_string(&body.to_string()) {
            Ok(response) | Err(ureq::Error::Status(_, response)) => Ok(read_reply(response)),
            Err(e) => Err(e.kind().to_string()),
        }
    }

    fn get(&self, url: &str, token: Option<&str>) -> Result<Reply, String> {
        let mut request = self.agent.get(url).set("Accept", "application/json");
        if let Some(token) = token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        match request.call() {
            Ok(response) | Err(ureq::Error::Status(_, response)) => Ok(read_reply(response)),
            Err(e) => Err(e.kind().to_string()),
        }
    }
}

#[cfg(feature = "http")]
fn read_reply(response: ureq::Response) -> Reply {
    use std::io::Read;
    let status = response.status();
    let mut text = String::new();
    let _ = response
        .into_reader()
        .take(MAX_REPLY_BYTES)
        .read_to_string(&mut text);
    Reply {
        status,
        body: serde_json::from_str(&text).unwrap_or(Value::Null),
    }
}

type Replies = std::collections::VecDeque<Result<Reply, String>>;

/// A transport that answers from a script, for tests: each request takes
/// the next reply, and every request made is kept for the test to read.
pub struct Scripted {
    replies: std::sync::Mutex<Replies>,
    /// `(method, url, body)` of every request, in order.
    pub requests: std::sync::Mutex<Vec<(String, String, Value)>>,
    /// Answers requests whose URL (query included) ends with the key
    /// first, so a test can script polls apart from posts.
    by_path: std::sync::Mutex<Vec<(String, Replies)>>,
}

impl Scripted {
    pub fn new() -> std::sync::Arc<Scripted> {
        std::sync::Arc::new(Scripted {
            replies: std::sync::Mutex::new(Default::default()),
            requests: std::sync::Mutex::new(Vec::new()),
            by_path: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Queues `reply` for the next request of any path.
    pub fn push(&self, reply: Result<Reply, String>) {
        self.replies.lock().unwrap().push_back(reply);
    }

    /// Queues `reply` for the next request whose URL ends with `path_end`
    /// (`"/events"`, `"?after=3"`).
    pub fn push_for(&self, path_end: &str, reply: Result<Reply, String>) {
        let mut map = self.by_path.lock().unwrap();
        if let Some((_, queue)) = map.iter_mut().find(|(p, _)| p == path_end) {
            queue.push_back(reply);
        } else {
            map.push((path_end.to_string(), std::iter::once(reply).collect()));
        }
    }

    fn answer(&self, method: &str, url: &str, body: Value) -> Result<Reply, String> {
        self.requests
            .lock()
            .unwrap()
            .push((method.to_string(), url.to_string(), body));
        {
            let mut map = self.by_path.lock().unwrap();
            if let Some((_, queue)) = map.iter_mut().find(|(p, _)| url.ends_with(p.as_str())) {
                if let Some(reply) = queue.pop_front() {
                    return reply;
                }
            }
        }
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err("nothing scripted".into()))
    }

    /// The URLs requested so far, without the base.
    pub fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(m, u, _)| {
                format!(
                    "{m} {}",
                    u.rsplit_once("/api/").map_or(u.as_str(), |(_, r)| r)
                )
            })
            .collect()
    }
}

impl Transport for Scripted {
    fn post(&self, url: &str, _token: Option<&str>, body: &Value) -> Result<Reply, String> {
        self.answer("POST", url, body.clone())
    }

    fn get(&self, url: &str, _token: Option<&str>) -> Result<Reply, String> {
        self.answer("GET", url, Value::Null)
    }
}

/// A reply with `status` and `body`, for scripts.
pub fn reply(status: u16, body: Value) -> Result<Reply, String> {
    Ok(Reply { status, body })
}
