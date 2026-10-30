//! Where a game's updates are published. A source only finds the signed
//! manifest for a platform and channel; checking it is the same for every
//! source, so a source the game writes itself is as safe as these.
//!
//! - [`GitHubReleases`]: the manifest is an asset of a GitHub release, the
//!   archive another asset of the same release.
//! - [`ManifestUrl`]: the manifest is at a URL on any HTTPS host, the
//!   archive beside it. `{platform}`, `{channel}` and `{game}` in the URL
//!   are filled in.

use serde_json::Value;

use crate::fetch::{url_allowed, Fetch, FetchError};
use crate::manifest::{is_channel, is_id, MAX_MANIFEST_BYTES};

/// What is being looked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub game: String,
    pub platform: String,
    pub channel: String,
}

/// The file name a release's manifest is published under:
/// `update-stable-windows-x64.json`.
pub fn manifest_name(channel: &str, platform: &str) -> String {
    format!("update-{channel}-{platform}.json")
}

/// A manifest a source found, and where to find the archive it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The manifest file's bytes, not yet checked.
    pub manifest: Vec<u8>,
    /// The manifest's own URL: an archive named without a URL is beside it.
    pub location: String,
    /// Files published with the manifest, by name, and their URLs, for a
    /// host (such as a GitHub release) whose files are not at predictable
    /// URLs beside each other.
    pub files: Vec<(String, String)>,
}

impl Found {
    /// The URL of the archive named `name`: one of `files`, else beside the
    /// manifest.
    pub fn file_url(&self, name: &str) -> Option<String> {
        if let Some((_, url)) = self.files.iter().find(|(n, _)| n == name) {
            return Some(url.clone());
        }
        let base = self.location.split(['?', '#']).next()?;
        let dir = &base[..base.rfind('/')? + 1];
        // Past the scheme's `//`, so a bare host is not cut.
        (dir.len() > "https://".len()).then(|| format!("{dir}{name}"))
    }
}

/// Why a source could not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    Fetch(FetchError),
    /// The place updates are published does not exist.
    NotFound,
    /// The host answered with something that is not what it should be.
    Malformed(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Fetch(e) => e.fmt(f),
            SourceError::NotFound => f.write_str("the update source was not found"),
            SourceError::Malformed(why) => write!(f, "the update source answered oddly: {why}"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<FetchError> for SourceError {
    fn from(e: FetchError) -> Self {
        SourceError::Fetch(e)
    }
}

/// Somewhere a game's updates are published.
pub trait UpdateSource: Send + Sync {
    /// The manifest for `query`, or `None` when nothing is published for
    /// that platform and channel.
    fn find(&self, query: &Query, fetch: &dyn Fetch) -> Result<Option<Found>, SourceError>;

    /// A short description for logs: `github.com/owner/repo`.
    fn describe(&self) -> String;
}

/// GitHub releases (or GitHub Enterprise, with [`GitHubReleases::api`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubReleases {
    owner: String,
    repo: String,
    api: String,
}

/// The public GitHub API.
pub const GITHUB_API: &str = "https://api.github.com";

/// The most of a release list read.
const MAX_RELEASE_LIST_BYTES: u64 = 8 * 1024 * 1024;

fn github_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl GitHubReleases {
    /// The releases of `owner/repo` on github.com. `None` when either is not
    /// a GitHub name.
    pub fn new(owner: &str, repo: &str) -> Option<GitHubReleases> {
        (github_name(owner) && github_name(repo)).then(|| GitHubReleases {
            owner: owner.to_string(),
            repo: repo.to_string(),
            api: GITHUB_API.to_string(),
        })
    }

    /// Another API, such as GitHub Enterprise's `https://github.example.com/api/v3`.
    pub fn api(mut self, base: &str) -> GitHubReleases {
        self.api = base.trim_end_matches('/').to_string();
        self
    }

    fn releases_url(&self) -> String {
        format!(
            "{}/repos/{}/{}/releases?per_page=100",
            self.api, self.owner, self.repo
        )
    }
}

impl UpdateSource for GitHubReleases {
    /// The newest release (as GitHub lists them, newest first) that carries
    /// this platform and channel's manifest. Drafts are skipped. Prereleases
    /// count: the channel in the manifest's name, not GitHub's flag, says
    /// which players a release is for.
    fn find(&self, query: &Query, fetch: &dyn Fetch) -> Result<Option<Found>, SourceError> {
        let list = match fetch.get(
            &self.releases_url(),
            &[
                ("Accept", "application/vnd.github+json"),
                ("X-GitHub-Api-Version", "2022-11-28"),
            ],
            MAX_RELEASE_LIST_BYTES,
        ) {
            Ok(list) => list,
            Err(FetchError::Status(404)) => return Err(SourceError::NotFound),
            Err(e) => return Err(e.into()),
        };
        let releases: Value = serde_json::from_slice(&list)
            .map_err(|_| SourceError::Malformed("the release list is not JSON".into()))?;
        let releases = releases
            .as_array()
            .ok_or_else(|| SourceError::Malformed("the release list is not a list".into()))?;
        let wanted = manifest_name(&query.channel, &query.platform);
        for release in releases {
            if release["draft"].as_bool() == Some(true) {
                continue;
            }
            let files: Vec<(String, String)> = release["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|asset| {
                    let name = asset["name"].as_str()?;
                    let url = asset["browser_download_url"].as_str()?;
                    url_allowed(url).then(|| (name.to_string(), url.to_string()))
                })
                .collect();
            let Some((_, url)) = files.iter().find(|(name, _)| *name == wanted) else {
                continue;
            };
            let manifest = fetch.get(url, &[], MAX_MANIFEST_BYTES)?;
            return Ok(Some(Found {
                manifest,
                location: url.clone(),
                files,
            }));
        }
        Ok(None)
    }

    fn describe(&self) -> String {
        let host = self
            .api
            .strip_prefix("https://")
            .unwrap_or(&self.api)
            .replace("api.github.com", "github.com");
        format!("{host}/{}/{}", self.owner, self.repo)
    }
}

/// A manifest at a URL on any HTTPS host: a static site, a bucket, a CDN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestUrl {
    template: String,
}

impl ManifestUrl {
    /// `template` with `{platform}`, `{channel}` and `{game}` filled in for
    /// each check: `https://example.com/my-game/update-{channel}-{platform}.json`.
    /// `None` when it is not an HTTPS URL.
    pub fn new(template: &str) -> Option<ManifestUrl> {
        let probe = template
            .replace("{platform}", "p")
            .replace("{channel}", "c")
            .replace("{game}", "g");
        (url_allowed(&probe) && !probe.contains(['{', '}'])).then(|| ManifestUrl {
            template: template.to_string(),
        })
    }

    /// The URL for `query`. The values are ids, so they need no escaping.
    pub fn url_for(&self, query: &Query) -> Option<String> {
        (is_channel(&query.platform) && is_channel(&query.channel) && is_id(&query.game)).then(
            || {
                self.template
                    .replace("{platform}", &query.platform)
                    .replace("{channel}", &query.channel)
                    .replace("{game}", &query.game)
            },
        )
    }
}

impl UpdateSource for ManifestUrl {
    fn find(&self, query: &Query, fetch: &dyn Fetch) -> Result<Option<Found>, SourceError> {
        let url = self
            .url_for(query)
            .ok_or_else(|| SourceError::Malformed("the query is not plain names".into()))?;
        match fetch.get(&url, &[], MAX_MANIFEST_BYTES) {
            Ok(manifest) => Ok(Some(Found {
                manifest,
                location: url,
                files: Vec::new(),
            })),
            // Nothing published for this platform or channel yet.
            Err(FetchError::Status(404 | 410)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn describe(&self) -> String {
        self.template.clone()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;
    use crate::fetch::Body;

    /// Answers from a table of URLs, and remembers what was asked.
    #[derive(Default)]
    pub struct Table {
        pub answers: HashMap<String, Result<Vec<u8>, FetchError>>,
        pub asked: Mutex<Vec<String>>,
    }

    impl Table {
        pub fn with(mut self, url: &str, body: impl Into<Vec<u8>>) -> Self {
            self.answers.insert(url.to_string(), Ok(body.into()));
            self
        }

        pub fn failing(mut self, url: &str, e: FetchError) -> Self {
            self.answers.insert(url.to_string(), Err(e));
            self
        }
    }

    impl Fetch for Table {
        fn get(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
            limit: u64,
        ) -> Result<Vec<u8>, FetchError> {
            self.asked.lock().unwrap().push(url.to_string());
            let body = self
                .answers
                .get(url)
                .cloned()
                .unwrap_or(Err(FetchError::Status(404)))?;
            if body.len() as u64 > limit {
                return Err(FetchError::TooLarge);
            }
            Ok(body)
        }

        fn open(&self, url: &str) -> Result<Body, FetchError> {
            let body = self.get(url, &[], u64::MAX)?;
            Ok(Body {
                length: Some(body.len() as u64),
                reader: Box::new(std::io::Cursor::new(body)),
            })
        }
    }

    pub fn query(channel: &str) -> Query {
        Query {
            game: "my-game".into(),
            platform: "linux-x64".into(),
            channel: channel.into(),
        }
    }

    fn release(tag: &str, draft: bool, assets: &[&str]) -> Value {
        serde_json::json!({
            "tag_name": tag,
            "draft": draft,
            "assets": assets.iter().map(|name| serde_json::json!({
                "name": name,
                "browser_download_url": format!("https://github.com/o/r/releases/download/{tag}/{name}"),
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn github_finds_the_newest_release_with_the_channels_manifest() {
        let list = serde_json::json!([
            release("v2.0.0", true, &["update-stable-linux-x64.json"]),
            release(
                "v1.5.0-beta.1",
                false,
                &["update-beta-linux-x64.json", "a.tar.gz"]
            ),
            release(
                "v1.4.0",
                false,
                &[
                    "update-stable-linux-x64.json",
                    "my-game-1.4.0-linux-x64.tar.gz"
                ]
            ),
            release("v1.3.0", false, &["update-stable-linux-x64.json"]),
        ]);
        let fetch = Table::default()
            .with(
                "https://api.github.com/repos/o/r/releases?per_page=100",
                list.to_string(),
            )
            .with(
                "https://github.com/o/r/releases/download/v1.4.0/update-stable-linux-x64.json",
                "stable manifest",
            )
            .with(
                "https://github.com/o/r/releases/download/v1.5.0-beta.1/update-beta-linux-x64.json",
                "beta manifest",
            );
        let source = GitHubReleases::new("o", "r").unwrap();
        let found = source.find(&query("stable"), &fetch).unwrap().unwrap();
        assert_eq!(found.manifest, b"stable manifest");
        assert_eq!(
            found.file_url("my-game-1.4.0-linux-x64.tar.gz").as_deref(),
            Some("https://github.com/o/r/releases/download/v1.4.0/my-game-1.4.0-linux-x64.tar.gz")
        );
        let beta = source.find(&query("beta"), &fetch).unwrap().unwrap();
        assert_eq!(beta.manifest, b"beta manifest");
        assert_eq!(source.find(&query("nightly"), &fetch), Ok(None));
        assert_eq!(source.describe(), "github.com/o/r");
    }

    #[test]
    fn github_enterprise_has_its_own_api() {
        let fetch = Table::default().with(
            "https://git.example.com/api/v3/repos/team/game/releases?per_page=100",
            "[]",
        );
        let source = GitHubReleases::new("team", "game")
            .unwrap()
            .api("https://git.example.com/api/v3/");
        assert_eq!(source.find(&query("stable"), &fetch), Ok(None));
        assert_eq!(source.describe(), "git.example.com/api/v3/team/game");
    }

    #[test]
    fn github_problems_are_not_up_to_date() {
        let url = "https://api.github.com/repos/o/r/releases?per_page=100";
        let source = GitHubReleases::new("o", "r").unwrap();
        let missing = Table::default();
        assert_eq!(
            source.find(&query("stable"), &missing),
            Err(SourceError::NotFound)
        );
        let limited = Table::default().failing(url, FetchError::Status(403));
        assert_eq!(
            source.find(&query("stable"), &limited),
            Err(SourceError::Fetch(FetchError::Status(403)))
        );
        let junk = Table::default().with(url, "<html>");
        assert!(matches!(
            source.find(&query("stable"), &junk),
            Err(SourceError::Malformed(_))
        ));
        assert!(GitHubReleases::new("o", "../r").is_none());
        assert!(GitHubReleases::new("", "r").is_none());
    }

    #[test]
    fn a_manifest_url_is_filled_in_for_each_check() {
        let source =
            ManifestUrl::new("https://cdn.example.com/{game}/update-{channel}-{platform}.json")
                .unwrap();
        let fetch = Table::default().with(
            "https://cdn.example.com/my-game/update-beta-linux-x64.json",
            "beta",
        );
        let found = source.find(&query("beta"), &fetch).unwrap().unwrap();
        assert_eq!(found.manifest, b"beta");
        assert_eq!(
            found.file_url("my-game-1.5.0-linux-x64.tar.gz").as_deref(),
            Some("https://cdn.example.com/my-game/my-game-1.5.0-linux-x64.tar.gz")
        );
        // Nothing published for a channel is not an error.
        assert_eq!(source.find(&query("stable"), &fetch), Ok(None));
        let down = Table::default().failing(
            "https://cdn.example.com/my-game/update-stable-linux-x64.json",
            FetchError::Status(500),
        );
        assert!(source.find(&query("stable"), &down).is_err());
    }

    #[test]
    fn only_https_templates_are_sources() {
        assert!(ManifestUrl::new("http://example.com/{platform}.json").is_none());
        assert!(ManifestUrl::new("https://example.com/{unknown}.json").is_none());
        assert!(ManifestUrl::new("https://example.com/u/{platform}.json?v=1").is_some());
        let source = ManifestUrl::new("https://example.com/{channel}/{platform}.json").unwrap();
        let mut q = query("stable");
        q.platform = "../../x".into();
        assert_eq!(source.url_for(&q), None);
    }

    #[test]
    fn archives_beside_a_manifest_with_a_query() {
        let found = Found {
            manifest: Vec::new(),
            location: "https://example.com/g/update-stable-linux-x64.json?token=1".into(),
            files: Vec::new(),
        };
        assert_eq!(
            found.file_url("g.tar.gz").as_deref(),
            Some("https://example.com/g/g.tar.gz")
        );
    }
}
