//! Is there an update? The same checks for every source: the manifest is
//! signed by this game's key, for this game, platform and channel, and
//! newer than the running version.

use crate::fetch::Fetch;
use crate::key::PublicKey;
use crate::manifest::{self, Manifest, ManifestError};
use crate::source::{Query, SourceError, UpdateSource};
use crate::version::Version;

/// What the running copy is, as `hearken release` baked it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub key: PublicKey,
    /// The game's package name.
    pub game: String,
    pub version: String,
    /// The channel this copy was released on.
    pub channel: String,
    pub platform: String,
}

impl Running {
    /// This program, when `hearken release` built it with an update key.
    /// `None` for every other build: a developer build never updates.
    pub fn baked() -> Option<Running> {
        let key = PublicKey::from_bytes(crate::baked::KEY?)?;
        Some(Running {
            key,
            game: crate::baked::GAME.to_string(),
            version: crate::baked::VERSION.to_string(),
            channel: crate::baked::CHANNEL.to_string(),
            platform: crate::platform::current(),
        })
    }
}

/// An update to offer: its signed manifest, and where its archive is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub manifest: Manifest,
    pub archive_url: String,
    /// Where the archive's file index is, when the manifest names one.
    pub index_url: Option<String>,
    /// The files this copy already has, so it downloads only the rest: set
    /// by [`crate::partial::plan`] after the check. `None` downloads the
    /// whole archive.
    pub plan: Option<crate::partial::Plan>,
}

impl Update {
    pub fn version(&self) -> &str {
        &self.manifest.version
    }

    /// The archive's size in bytes, as signed.
    pub fn size(&self) -> u64 {
        self.manifest.size
    }

    /// The bytes downloading it takes: less than [`Update::size`] when this
    /// copy has some of its files already.
    pub fn download_size(&self) -> u64 {
        self.plan
            .as_ref()
            .map_or(self.manifest.size, |plan| plan.download_size())
    }
}

/// Why a manifest was refused, though it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Manifest(ManifestError),
    /// Signed with this game's key, but for another game.
    WrongGame(String),
    WrongPlatform(String),
    WrongChannel(String),
    /// The archive the manifest names is nowhere to be found.
    NoArchive(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Manifest(e) => e.fmt(f),
            Refusal::WrongGame(g) => write!(f, "the manifest is for another game ({g})"),
            Refusal::WrongPlatform(p) => write!(f, "the manifest is for another platform ({p})"),
            Refusal::WrongChannel(c) => write!(f, "the manifest is for another channel ({c})"),
            Refusal::NoArchive(a) => write!(f, "the manifest's archive {a} is not published"),
        }
    }
}

/// Why a check found nothing to offer, other than being up to date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// The source could not be reached, or answered oddly.
    Source(SourceError),
    /// What the source offered failed its checks. Never installed.
    Refused(Refusal),
    /// This copy's own version does not parse; nothing can be compared.
    BadRunningVersion(String),
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckError::Source(e) => e.fmt(f),
            CheckError::Refused(r) => write!(f, "refused the update: {r}"),
            CheckError::BadRunningVersion(v) => write!(f, "this copy's version {v} is not valid"),
        }
    }
}

impl std::error::Error for CheckError {}

impl From<SourceError> for CheckError {
    fn from(e: SourceError) -> Self {
        CheckError::Source(e)
    }
}

impl From<Refusal> for CheckError {
    fn from(r: Refusal) -> Self {
        CheckError::Refused(r)
    }
}

/// The update on `channel` newer than `running`, if any. `Ok(None)` means
/// up to date, including when the source offers the running version or an
/// older one: an older one is never installed, so a host or a person in
/// between cannot roll a player back to a version with a known flaw.
pub fn check(
    source: &dyn UpdateSource,
    fetch: &dyn Fetch,
    running: &Running,
    channel: &str,
) -> Result<Option<Update>, CheckError> {
    let current = Version::parse(&running.version)
        .ok_or_else(|| CheckError::BadRunningVersion(running.version.clone()))?;
    let query = Query {
        game: running.game.clone(),
        platform: running.platform.clone(),
        channel: channel.to_string(),
    };
    let Some(found) = source.find(&query, fetch)? else {
        return Ok(None);
    };
    let manifest = manifest::open(&found.manifest, &running.key).map_err(Refusal::Manifest)?;
    if manifest.game != running.game {
        return Err(Refusal::WrongGame(manifest.game).into());
    }
    if manifest.platform != running.platform {
        return Err(Refusal::WrongPlatform(manifest.platform).into());
    }
    if manifest.channel != channel {
        return Err(Refusal::WrongChannel(manifest.channel).into());
    }
    let offered = manifest
        .parsed_version()
        .ok_or_else(|| Refusal::Manifest(ManifestError::Malformed("version".into())))?;
    if !offered.is_newer_than(&current) {
        if offered.precedence(&current).is_lt() {
            log::warn!(
                "{} offers {offered} on {channel}, older than this copy's {current}; not installing it",
                source.describe()
            );
        }
        return Ok(None);
    }
    let archive_url = match &manifest.url {
        Some(url) => url.clone(),
        None => found
            .file_url(&manifest.archive)
            .ok_or_else(|| Refusal::NoArchive(manifest.archive.clone()))?,
    };
    // Beside the manifest, like the archive (on GitHub, an asset of the
    // same release). A missing index only means a whole download.
    let index_url = manifest
        .index
        .as_ref()
        .and_then(|index| found.file_url(&index.file));
    Ok(Some(Update {
        manifest,
        archive_url,
        index_url,
        plan: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::FetchError;
    use crate::key::SecretKey;
    use crate::manifest::tests::sample;
    use crate::source::tests::Table;
    use crate::source::ManifestUrl;

    const URL: &str = "https://example.com/g/update-stable-linux-x64.json";

    fn running(secret: &SecretKey, version: &str) -> Running {
        Running {
            key: secret.public(),
            game: "my-game".into(),
            version: version.into(),
            channel: "stable".into(),
            platform: "linux-x64".into(),
        }
    }

    fn source() -> ManifestUrl {
        ManifestUrl::new("https://example.com/g/update-{channel}-{platform}.json").unwrap()
    }

    fn serving(manifest: &Manifest, secret: &SecretKey) -> Table {
        Table::default().with(URL, manifest::sign(manifest, secret).unwrap())
    }

    #[test]
    fn a_newer_signed_release_is_offered() {
        let secret = SecretKey::generate().unwrap();
        let fetch = serving(&sample(), &secret);
        let update = check(&source(), &fetch, &running(&secret, "1.3.2"), "stable")
            .unwrap()
            .unwrap();
        assert_eq!(update.version(), "1.4.0");
        assert_eq!(
            update.archive_url,
            "https://example.com/g/my-game-1.4.0-linux-x64.tar.gz"
        );
        assert_eq!(update.index_url, None);
        assert_eq!(update.download_size(), update.size());
    }

    #[test]
    fn a_signed_index_is_found_beside_the_manifest() {
        let secret = SecretKey::generate().unwrap();
        let mut manifest = sample();
        manifest.archive = "my-game-1.4.0-windows-x64.zip".into();
        manifest.index = Some(crate::manifest::IndexRef {
            file: "my-game-1.4.0-windows-x64.index.json".into(),
            size: 10,
            sha256: "ab".repeat(32),
        });
        let fetch = serving(&manifest, &secret);
        let update = check(&source(), &fetch, &running(&secret, "1.3.2"), "stable")
            .unwrap()
            .unwrap();
        assert_eq!(
            update.index_url.as_deref(),
            Some("https://example.com/g/my-game-1.4.0-windows-x64.index.json")
        );
        assert_eq!(update.plan, None);
    }

    #[test]
    fn the_same_or_an_older_version_is_up_to_date() {
        let secret = SecretKey::generate().unwrap();
        let fetch = serving(&sample(), &secret);
        for version in ["1.4.0", "1.4.1", "2.0.0", "1.4.0+rebuilt"] {
            assert_eq!(
                check(&source(), &fetch, &running(&secret, version), "stable"),
                Ok(None),
                "{version}"
            );
        }
        // A beta of the release offered is older than it.
        assert!(check(
            &source(),
            &fetch,
            &running(&secret, "1.4.0-beta.3"),
            "stable"
        )
        .unwrap()
        .is_some());
    }

    #[test]
    fn nothing_published_is_up_to_date() {
        let secret = SecretKey::generate().unwrap();
        assert_eq!(
            check(
                &source(),
                &Table::default(),
                &running(&secret, "1.0.0"),
                "stable"
            ),
            Ok(None)
        );
    }

    #[test]
    fn another_keys_manifest_is_refused() {
        let secret = SecretKey::generate().unwrap();
        let attacker = SecretKey::generate().unwrap();
        let fetch = serving(&sample(), &attacker);
        let err = check(&source(), &fetch, &running(&secret, "1.0.0"), "stable").unwrap_err();
        assert!(matches!(
            err,
            CheckError::Refused(Refusal::Manifest(ManifestError::BadSignature { .. }))
        ));
    }

    #[test]
    fn a_signed_manifest_for_elsewhere_is_refused() {
        let secret = SecretKey::generate().unwrap();
        let mut other_game = sample();
        other_game.game = "other-game".into();
        let mut other_platform = sample();
        other_platform.platform = "windows-x64".into();
        let mut other_channel = sample();
        other_channel.channel = "beta".into();
        for (manifest, expected) in [
            (other_game, Refusal::WrongGame("other-game".into())),
            (other_platform, Refusal::WrongPlatform("windows-x64".into())),
            (other_channel, Refusal::WrongChannel("beta".into())),
        ] {
            let fetch = serving(&manifest, &secret);
            assert_eq!(
                check(&source(), &fetch, &running(&secret, "1.0.0"), "stable"),
                Err(CheckError::Refused(expected))
            );
        }
    }

    #[test]
    fn network_trouble_is_an_error_not_up_to_date() {
        let secret = SecretKey::generate().unwrap();
        let fetch = Table::default().failing(URL, FetchError::Offline("reset".into()));
        assert!(matches!(
            check(&source(), &fetch, &running(&secret, "1.0.0"), "stable"),
            Err(CheckError::Source(_))
        ));
    }

    #[test]
    fn an_archive_elsewhere_is_fetched_from_its_signed_url() {
        let secret = SecretKey::generate().unwrap();
        let mut manifest = sample();
        manifest.url = Some("https://downloads.example.net/x/my-game.tar.gz".into());
        let fetch = serving(&manifest, &secret);
        let update = check(&source(), &fetch, &running(&secret, "1.0.0"), "stable")
            .unwrap()
            .unwrap();
        assert_eq!(
            update.archive_url,
            "https://downloads.example.net/x/my-game.tar.gz"
        );
    }

    #[test]
    fn a_running_version_that_does_not_parse_checks_nothing() {
        let secret = SecretKey::generate().unwrap();
        assert!(matches!(
            check(
                &source(),
                &Table::default(),
                &running(&secret, "dev"),
                "stable"
            ),
            Err(CheckError::BadRunningVersion(_))
        ));
    }
}
