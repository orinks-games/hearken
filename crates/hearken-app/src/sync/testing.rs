//! An account's synced files in memory, for tests: a [`Transport`] that
//! answers the `saves/` routes as `@hearken/online` does.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::{base64, sha256_hex, unbase64};
use crate::online::{Body, Reply, Transport};

/// One kept revision.
#[derive(Clone, Debug)]
pub struct Kept {
    pub revision: u64,
    pub format: u32,
    pub content: Vec<u8>,
    pub card: Option<Value>,
}

/// Accounts' saves, by account then file name. Tokens map to accounts
/// through [`FakeSaves::token`]; a token never given is signed out, and one
/// given as a guest is refused as `account-needed`.
#[derive(Default)]
pub struct FakeSaves {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tokens: BTreeMap<String, (String, bool)>,
    saves: BTreeMap<(String, String), Vec<Kept>>,
    offline: bool,
    /// Requests seen, as "GET saves/records/head".
    log: Vec<String>,
}

impl FakeSaves {
    pub fn new() -> Arc<FakeSaves> {
        Arc::new(FakeSaves::default())
    }

    /// Lets `token` act as `account` (a guest when `guest`).
    pub fn token(&self, token: &str, account: &str, guest: bool) {
        self.inner
            .lock()
            .unwrap()
            .tokens
            .insert(token.into(), (account.into(), guest));
    }

    /// Whether the server answers at all.
    pub fn set_offline(&self, offline: bool) {
        self.inner.lock().unwrap().offline = offline;
    }

    /// The revisions kept of `name` for `account`, oldest first.
    pub fn kept(&self, account: &str, name: &str) -> Vec<Kept> {
        self.inner
            .lock()
            .unwrap()
            .saves
            .get(&(account.into(), name.into()))
            .cloned()
            .unwrap_or_default()
    }

    /// The requests seen so far.
    pub fn requests(&self) -> Vec<String> {
        self.inner.lock().unwrap().log.clone()
    }

    /// Answers `method` on `url` if it is a saves route, as the server would.
    pub fn handle(
        &self,
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Option<Result<Reply, String>> {
        let at = url.find("/saves/")?;
        let parts: Vec<&str> = url[at + 1..].split('/').collect();
        let mut inner = self.inner.lock().unwrap();
        inner.log.push(format!("{method} {}", parts.join("/")));
        if inner.offline {
            return Some(Err("offline".into()));
        }
        let reply = |status: u16, body: Value| Some(Ok(Reply { status, body }));
        let Some((account, guest)) = token.and_then(|t| inner.tokens.get(t).cloned()) else {
            return reply(401, json!({"error": "signed-out"}));
        };
        if guest {
            return reply(403, json!({"error": "account-needed"}));
        }
        let name = parts.get(1).copied().unwrap_or("").to_string();
        let key = (account.clone(), name);
        let latest = inner.saves.get(&key).and_then(|l| l.last()).cloned();
        match (method, parts.len(), parts.get(2).copied()) {
            ("GET", 3, Some("head")) => reply(
                200,
                json!({
                    "ok": true,
                    "account": account,
                    "save": latest.map(|k| json!({
                        "revision": k.revision,
                        "hash": sha256_hex(&k.content),
                        "bytes": k.content.len(),
                        "format": k.format,
                    })),
                }),
            ),
            ("GET", 2, _) => match latest {
                None => reply(404, json!({"error": "no-save"})),
                Some(k) => reply(
                    200,
                    json!({
                        "ok": true,
                        "revision": k.revision,
                        "hash": sha256_hex(&k.content),
                        "format": k.format,
                        "content": base64(&k.content),
                    }),
                ),
            },
            ("POST", 2, _) => {
                let body = body?;
                let content = body["content"].as_str().and_then(unbase64)?;
                if body["hash"].as_str() != Some(sha256_hex(&content).as_str()) {
                    return reply(422, json!({"error": "bad-save"}));
                }
                if latest
                    .as_ref()
                    .is_some_and(|k| sha256_hex(&k.content) == sha256_hex(&content))
                {
                    return reply(
                        200,
                        json!({"ok": true, "revision": latest.unwrap().revision, "repeat": true}),
                    );
                }
                let current = latest.map_or(0, |k| k.revision);
                if body["base"].as_u64() != Some(current) {
                    return reply(409, json!({"error": "stale", "revision": current}));
                }
                let kept = Kept {
                    revision: current + 1,
                    format: body["format"].as_u64().unwrap_or(0) as u32,
                    content,
                    card: body.get("card").cloned(),
                };
                inner.saves.entry(key).or_default().push(kept);
                reply(200, json!({"ok": true, "revision": current + 1}))
            }
            _ => reply(404, json!({"error": "not-found"})),
        }
    }
}

impl Transport for FakeSaves {
    fn post(&self, url: &str, token: Option<&str>, body: Body) -> Result<Reply, String> {
        let body = match body {
            Body::Json(v) => Some(v),
            Body::Run(_) => None,
        };
        self.handle("POST", url, token, body.as_ref())
            .unwrap_or_else(|| {
                Ok(Reply {
                    status: 404,
                    body: json!({"error": "not-found"}),
                })
            })
    }

    fn get(&self, url: &str, token: Option<&str>) -> Result<Reply, String> {
        self.handle("GET", url, token, None).unwrap_or_else(|| {
            Ok(Reply {
                status: 404,
                body: json!({"error": "not-found"}),
            })
        })
    }
}

impl Transport for Arc<FakeSaves> {
    fn post(&self, url: &str, token: Option<&str>, body: Body) -> Result<Reply, String> {
        (**self).post(url, token, body)
    }

    fn get(&self, url: &str, token: Option<&str>) -> Result<Reply, String> {
        (**self).get(url, token)
    }
}
