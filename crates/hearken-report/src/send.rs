//! Posting a report to a relay, and what its answer means.

use std::fmt::{self, Display, Formatter};

use crate::report::{Invalid, Report};

/// A relay's answer to a posted report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    /// The body, which says why a report was refused.
    pub body: String,
}

/// Posts a report's JSON. The real one is HTTPS through ureq; tests answer
/// from a table.
pub trait Sender: Send + Sync {
    /// Posts `json` to `endpoint`, gzipped or not as the sender sees fit.
    /// `Err` only when no answer came at all: no network, a timeout.
    fn post(&self, endpoint: &str, json: &[u8]) -> Result<Answer, String>;
}

/// Why sending a report failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// The report breaks a limit, so it was not sent.
    Invalid(Invalid),
    /// The reply address's domain does not exist or takes no mail, which
    /// is usually a typo the player can fix.
    ReplyAddressTakesNoMail,
    /// The relay has taken as many reports as it will for now.
    RateLimited,
    /// The relay refused the report, for the reason given.
    Rejected(String),
    /// The relay could not be reached, or could not send the email.
    Failed(String),
}

impl Display for SendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Invalid(invalid) => write!(f, "the report is not valid: {invalid}"),
            SendError::ReplyAddressTakesNoMail => {
                f.write_str("the reply address's domain does not take email")
            }
            SendError::RateLimited => {
                f.write_str("too many reports have been sent; try again tomorrow")
            }
            SendError::Rejected(reason) => write!(f, "the relay refused the report: {reason}"),
            SendError::Failed(reason) => write!(f, "the report could not be sent: {reason}"),
        }
    }
}

impl std::error::Error for SendError {}

/// A report opened on the relay's web form: the page the player writes
/// it on, with the logs already there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub url: String,
}

/// `scheme://host[:port]` of `url`, or `None` when it has no scheme.
fn origin(url: &str) -> Option<&str> {
    let after_scheme = url.find("://")? + 3;
    let end = url[after_scheme..]
        .find('/')
        .map_or(url.len(), |i| after_scheme + i);
    Some(&url[..end])
}

impl Report {
    /// Sends the report to the relay at `endpoint` through `sender`.
    ///
    /// Returns why the relay did not take the report, or why it could not
    /// be reached; offer to save [`Report::to_text`] instead.
    pub fn send_with(&self, endpoint: &str, sender: &dyn Sender) -> Result<(), SendError> {
        self.validate().map_err(SendError::Invalid)?;
        let json = serde_json::to_vec(self).map_err(|e| SendError::Failed(e.to_string()))?;
        let answer = sender.post(endpoint, &json).map_err(SendError::Failed)?;
        match answer.status {
            200..=299 => Ok(()),
            422 => Err(SendError::ReplyAddressTakesNoMail),
            429 => Err(SendError::RateLimited),
            400..=499 => Err(SendError::Rejected(answer.body)),
            status => Err(SendError::Failed(format!("the relay answered {status}"))),
        }
    }

    /// Sends the report to the relay at `endpoint`, such as
    /// `https://example.com/api/problem-reports`, over HTTPS.
    #[cfg(feature = "http")]
    pub fn send(&self, endpoint: &str) -> Result<(), SendError> {
        self.send_with(endpoint, &HttpSender::new())
    }

    /// Sends the report's logs ahead to `{endpoint}/open` through `sender`
    /// and returns the page the player writes the rest on. The description
    /// may be empty; the page takes it.
    ///
    /// The relay answers `201` with `{ "url": ... }`; the page must be on
    /// the relay's own origin, so a relay cannot send the browser
    /// elsewhere. Refusals are the ones [`Report::send_with`] gives.
    pub fn open_with(&self, endpoint: &str, sender: &dyn Sender) -> Result<Opened, SendError> {
        self.validate_sent_ahead().map_err(SendError::Invalid)?;
        let json = serde_json::to_vec(self).map_err(|e| SendError::Failed(e.to_string()))?;
        let open = format!("{}/open", endpoint.trim_end_matches('/'));
        let answer = sender.post(&open, &json).map_err(SendError::Failed)?;
        match answer.status {
            200..=299 => {}
            422 => return Err(SendError::ReplyAddressTakesNoMail),
            429 => return Err(SendError::RateLimited),
            400..=499 => return Err(SendError::Rejected(answer.body)),
            status => return Err(SendError::Failed(format!("the relay answered {status}"))),
        }
        #[derive(serde::Deserialize)]
        struct Link {
            url: String,
        }
        let link: Link = serde_json::from_str(&answer.body)
            .map_err(|_| SendError::Failed("the relay gave no page to open".into()))?;
        let same_site = origin(endpoint).is_some_and(|site| {
            link.url
                .strip_prefix(site)
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if !same_site {
            return Err(SendError::Failed(
                "the relay gave a page on another site".into(),
            ));
        }
        Ok(Opened { url: link.url })
    }

    /// Sends the report's logs ahead to the relay at `endpoint` over HTTPS
    /// and returns the page to open; see [`Report::open_with`].
    #[cfg(feature = "http")]
    pub fn open(&self, endpoint: &str) -> Result<Opened, SendError> {
        self.open_with(endpoint, &HttpSender::new())
    }
}

/// HTTPS through ureq: the report's JSON, gzipped, with a minute to get
/// through. Redirects are not followed.
#[cfg(feature = "http")]
pub struct HttpSender {
    agent: ureq::Agent,
}

#[cfg(feature = "http")]
impl Default for HttpSender {
    fn default() -> Self {
        HttpSender::new()
    }
}

#[cfg(feature = "http")]
impl HttpSender {
    pub fn new() -> Self {
        HttpSender {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(60))
                .redirects(0)
                .build(),
        }
    }
}

#[cfg(feature = "http")]
impl Sender for HttpSender {
    fn post(&self, endpoint: &str, json: &[u8]) -> Result<Answer, String> {
        use std::io::Write as _;

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(json).map_err(|e| e.to_string())?;
        let body = encoder.finish().map_err(|e| e.to_string())?;
        let response = match self
            .agent
            .post(endpoint)
            .set("Content-Type", "application/json")
            .set("Content-Encoding", "gzip")
            .send_bytes(&body)
        {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(e)) => return Err(e.to_string()),
        };
        let status = response.status();
        // A refusal's reason is short; anything longer is not one.
        let body = response
            .into_string()
            .map(|text| text.chars().take(1_000).collect())
            .unwrap_or_default();
        Ok(Answer { status, body })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct Table {
        answer: Result<Answer, String>,
        posted: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl Table {
        fn answering(status: u16, body: &str) -> Table {
            Table {
                answer: Ok(Answer {
                    status,
                    body: body.to_string(),
                }),
                posted: Mutex::new(Vec::new()),
            }
        }
    }

    impl Sender for Table {
        fn post(&self, endpoint: &str, json: &[u8]) -> Result<Answer, String> {
            self.posted
                .lock()
                .unwrap()
                .push((endpoint.to_string(), json.to_vec()));
            self.answer.clone()
        }
    }

    fn report() -> Report {
        Report::new("Freight Fate", "1.9.6", "The dock menu is empty.")
    }

    #[test]
    fn an_accepted_report_posts_its_json_to_the_endpoint() {
        let relay = Table::answering(202, "");
        assert_eq!(
            report().send_with("https://example.test/report", &relay),
            Ok(())
        );
        let posted = relay.posted.lock().unwrap();
        assert_eq!(posted[0].0, "https://example.test/report");
        let json: Report = serde_json::from_slice(&posted[0].1).unwrap();
        assert_eq!(json, report());
    }

    #[test]
    fn each_refusal_has_its_own_error() {
        let endpoint = "https://example.test/report";
        assert_eq!(
            report().send_with(endpoint, &Table::answering(422, "")),
            Err(SendError::ReplyAddressTakesNoMail)
        );
        assert_eq!(
            report().send_with(endpoint, &Table::answering(429, "")),
            Err(SendError::RateLimited)
        );
        assert_eq!(
            report().send_with(endpoint, &Table::answering(403, "unknown app")),
            Err(SendError::Rejected("unknown app".into()))
        );
        assert_eq!(
            report().send_with(endpoint, &Table::answering(502, "")),
            Err(SendError::Failed("the relay answered 502".into()))
        );
    }

    #[test]
    fn an_invalid_report_is_never_posted() {
        let relay = Table::answering(202, "");
        let result = report()
            .with_reply_to("nope")
            .send_with("https://example.test/report", &relay);
        assert_eq!(result, Err(SendError::Invalid(Invalid::NotAnAddress)));
        assert!(relay.posted.lock().unwrap().is_empty());
    }

    #[test]
    fn opening_posts_to_the_open_route_and_takes_the_page_from_the_answer() {
        let relay = Table::answering(
            201,
            r#"{"code":"abc","url":"https://example.test/report/abc"}"#,
        );
        let ahead = Report::new("Freight Fate", "1.9.6", "").with_log("session.log", "x\n");
        assert_eq!(
            ahead.open_with("https://example.test/api/problem-reports/", &relay),
            Ok(Opened {
                url: "https://example.test/report/abc".into()
            })
        );
        let posted = relay.posted.lock().unwrap();
        assert_eq!(posted[0].0, "https://example.test/api/problem-reports/open");
        let json: Report = serde_json::from_slice(&posted[0].1).unwrap();
        assert_eq!(json.description, "");
        assert_eq!(json.attachments.len(), 1);
    }

    #[test]
    fn a_page_off_the_relays_site_or_a_refusal_is_not_opened() {
        let endpoint = "https://example.test/api/problem-reports";
        let ahead = Report::new("Freight Fate", "1.9.6", "");
        let elsewhere = Table::answering(201, r#"{"url":"https://example.test.evil/report/abc"}"#);
        assert_eq!(
            ahead.open_with(endpoint, &elsewhere),
            Err(SendError::Failed(
                "the relay gave a page on another site".into()
            ))
        );
        let nothing = Table::answering(201, "");
        assert_eq!(
            ahead.open_with(endpoint, &nothing),
            Err(SendError::Failed("the relay gave no page to open".into()))
        );
        assert_eq!(
            ahead.open_with(endpoint, &Table::answering(429, "")),
            Err(SendError::RateLimited)
        );
        assert_eq!(
            ahead.open_with(endpoint, &Table::answering(503, "")),
            Err(SendError::Failed("the relay answered 503".into()))
        );
        assert_eq!(
            ahead
                .with_reply_to("nope")
                .open_with(endpoint, &Table::answering(201, "")),
            Err(SendError::Invalid(Invalid::NotAnAddress))
        );
    }

    #[test]
    fn no_answer_is_a_failure_with_the_reason() {
        let relay = Table {
            answer: Err("connection refused".into()),
            posted: Mutex::new(Vec::new()),
        };
        assert_eq!(
            report().send_with("https://example.test/report", &relay),
            Err(SendError::Failed("connection refused".into()))
        );
    }
}
