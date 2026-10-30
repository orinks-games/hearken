//! A released game updating itself: `ctx.updates`.
//!
//! A game names where its updates are published in `Game::updates`: GitHub
//! releases, a manifest URL on any HTTPS host, or a source of its own. The
//! engine checks there off the game loop when the game starts, and from
//! Settings when the player asks, and offers what it finds through the
//! screens in [`crate::screens::update`]. Everything that decides whether
//! an update is safe to install is in `hearken-update`, the same for every
//! source; see `docs/updates.md` for publishing one.
//!
//! Only a copy `hearken release` built for a game with `update = true`
//! updates itself. A developer build, a copy run from a Cargo `target`
//! folder, one on iOS and one AG Now runs say why instead.

use std::io;
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

pub use hearken_update::{
    spoken_version, Blocked, CheckError, Fetch, Found, GitHubReleases, Host, Install, Manifest,
    ManifestUrl, Query, Running, SourceError, Staged, Update, UpdateSource, DEFAULT_CHANNEL,
};

use crate::settings::UpdateSettings;

/// Where a game's updates are published, and what to call the game out
/// loud.
#[derive(Clone)]
pub struct UpdateConfig {
    pub(crate) source: Arc<dyn UpdateSource>,
    /// The game's name as the player hears it in "My Game version 1.4.0".
    pub name: String,
    /// The channels a player can follow, the default first.
    pub channels: Vec<String>,
    /// Where a player can get an update by hand, spoken when one cannot
    /// install itself: "example.com/my-game".
    pub manual: Option<String>,
    /// Whether the game checks each time it starts (the player can still
    /// turn it off).
    pub check_at_start: bool,
    /// Whether an update found at start is announced. Off, the game reads
    /// `ctx.updates.available()` and offers it its own way.
    pub announce: bool,
}

impl UpdateConfig {
    /// Updates from `source`, for the game called `name`.
    pub fn new(name: impl Into<String>, source: impl UpdateSource + 'static) -> UpdateConfig {
        UpdateConfig {
            source: Arc::new(source),
            name: name.into(),
            channels: vec![DEFAULT_CHANNEL.to_string()],
            manual: None,
            check_at_start: true,
            announce: true,
        }
    }

    /// Updates from the releases of `owner/repo` on GitHub. `None` when
    /// either is not a GitHub name.
    pub fn github(name: impl Into<String>, owner: &str, repo: &str) -> Option<UpdateConfig> {
        let source = GitHubReleases::new(owner, repo)?;
        Some(UpdateConfig::new(name, source).manual(format!("github.com/{owner}/{repo}/releases")))
    }

    /// Updates from a manifest URL on any HTTPS host, with `{platform}`,
    /// `{channel}` and `{game}` filled in. `None` when it is not HTTPS.
    pub fn manifest_url(name: impl Into<String>, template: &str) -> Option<UpdateConfig> {
        Some(UpdateConfig::new(name, ManifestUrl::new(template)?))
    }

    /// The channels a player can choose, the default first: `["stable",
    /// "beta"]`. Names that are not lowercase letters, digits and hyphens
    /// are dropped; with none left, `stable` stays.
    pub fn channels(mut self, channels: &[&str]) -> UpdateConfig {
        let mut kept: Vec<String> = Vec::new();
        for c in channels {
            if hearken_update::manifest::is_channel(c) && !kept.iter().any(|k| k == c) {
                kept.push(c.to_string());
            }
        }
        if !kept.is_empty() {
            self.channels = kept;
        }
        self
    }

    /// Where to get an update by hand, as it is spoken.
    pub fn manual(mut self, place: impl Into<String>) -> UpdateConfig {
        self.manual = Some(place.into());
        self
    }

    /// Checks at start without saying anything: the game offers what
    /// `ctx.updates.available()` holds itself.
    pub fn quiet(mut self) -> UpdateConfig {
        self.announce = false;
        self
    }

    /// Never checks at start; only from Settings.
    pub fn no_check_at_start(mut self) -> UpdateConfig {
        self.check_at_start = false;
        self
    }
}

impl std::fmt::Debug for UpdateConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateConfig")
            .field("source", &self.source.describe())
            .field("name", &self.name)
            .field("channels", &self.channels)
            .finish()
    }
}

/// Puts the version before an update back when that update did not finish
/// being copied in (the computer shut down part-way, or the copy failed).
/// Runs as the game starts, before anything opens. Returns the line to
/// tell the player, when there is one.
pub fn recover_interrupted() -> Option<String> {
    let install = Host::current().install();
    match hearken_update::recover_interrupted(&install) {
        Ok(hearken_update::Recovery::Nothing) => None,
        Ok(hearken_update::Recovery::Restored) => Some(
            "The last update did not finish installing, so the version before it was put back."
                .to_string(),
        ),
        Ok(hearken_update::Recovery::NoBackup) => Some(
            "The last update did not finish installing. If the game does not work as it \
             should, download it again."
                .to_string(),
        ),
        Err(e) => {
            log::warn!("could not undo an unfinished update: {e}");
            Some(
                "The last update did not finish installing, and the version before it could \
                 not be put back. If the game does not work as it should, download it again."
                    .to_string(),
            )
        }
    }
}

/// What a check found: an update, nothing newer (`Ok(None)`), or a line for
/// the player saying why it could not tell.
pub type CheckOutcome = Result<Option<Update>, String>;

/// A check running on its own thread.
pub struct Checker {
    rx: Receiver<CheckOutcome>,
    done: Option<CheckOutcome>,
}

impl Checker {
    /// The outcome, once there is one. Keeps answering the same outcome.
    pub fn poll(&mut self) -> Option<&CheckOutcome> {
        if self.done.is_none() {
            match self.rx.try_recv() {
                Ok(outcome) => self.done = Some(outcome),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.done = Some(Err("The update check stopped.".to_string()))
                }
            }
        }
        self.done.as_ref()
    }

    /// A checker that already has `outcome`, for tests.
    pub fn finished(outcome: CheckOutcome) -> Checker {
        let (_tx, rx) = std::sync::mpsc::channel();
        Checker {
            rx,
            done: Some(outcome),
        }
    }
}

/// The swap: writes and starts the apply script. A test puts in one that
/// records the call (`Updates::set_apply`).
pub type ApplyFn = Arc<dyn Fn(&Staged, &Install, &Path, &Host) -> io::Result<()> + Send + Sync>;

/// The game's updates: its source, this copy, and what the check at start
/// found.
pub struct Updates {
    config: Option<UpdateConfig>,
    running: Option<Running>,
    host: Host,
    agnow: bool,
    fetch: Arc<dyn Fetch>,
    apply: ApplyFn,
    background: Option<Checker>,
    found: Option<Update>,
}

/// What the player hears when a check could not tell.
pub fn check_error_line(e: &CheckError) -> String {
    use hearken_update::{FetchError, Refusal};
    match e {
        CheckError::Source(SourceError::NotFound) => {
            "The game's update page was not found.".to_string()
        }
        CheckError::Source(SourceError::Fetch(FetchError::Status(403 | 429))) => {
            "The update server is busy.".to_string()
        }
        CheckError::Source(_) => "Could not reach the update server.".to_string(),
        CheckError::Refused(Refusal::Manifest(hearken_update::ManifestError::BadSignature {
            ..
        })) => "The update server offered an update this game did not sign, so it was \
                not installed."
            .to_string(),
        CheckError::Refused(_) => {
            "The update server offered an update that did not pass its checks, so it was \
             not installed."
                .to_string()
        }
        CheckError::BadRunningVersion(_) => {
            "This copy's version number cannot be compared, so updates are off.".to_string()
        }
    }
}

impl Updates {
    pub fn new(config: Option<UpdateConfig>) -> Updates {
        Updates {
            config,
            running: Running::baked(),
            host: Host::current(),
            agnow: hearken_update::managed_by_agnow(),
            fetch: Arc::new(hearken_update::HttpFetch::default()),
            apply: Arc::new(|staged, install, staging, host| {
                hearken_update::apply_and_restart(staged, install, staging, host).map(|_| ())
            }),
            background: None,
            found: None,
        }
    }

    /// Whether the game offers updates at all.
    pub fn is_offered(&self) -> bool {
        self.config.is_some()
    }

    pub fn config(&self) -> Option<&UpdateConfig> {
        self.config.as_ref()
    }

    /// The game's spoken name, "the game" when it offers no updates.
    pub fn name(&self) -> &str {
        self.config.as_ref().map_or("the game", |c| c.name.as_str())
    }

    /// This copy, as `hearken release` built it; `None` in a developer build.
    pub fn running(&self) -> Option<&Running> {
        self.running.as_ref()
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn fetch(&self) -> Arc<dyn Fetch> {
        self.fetch.clone()
    }

    /// The running version as a player hears it, when this copy has one.
    pub fn spoken_current(&self) -> Option<String> {
        self.running.as_ref().map(|r| spoken_version(&r.version))
    }

    /// "My Game version 1.4.0".
    pub fn title(&self, update: &Update) -> String {
        format!(
            "{} version {}",
            self.name(),
            spoken_version(update.version())
        )
    }

    /// Why this copy does not update itself, or `None` when it can.
    pub fn blocked(&self) -> Option<Blocked> {
        hearken_update::blocked(self.running.as_ref(), &self.host, self.agnow)
    }

    /// What the player hears instead of a check, when this copy cannot
    /// update itself.
    pub fn blocked_line(&self) -> Option<String> {
        if !self.is_offered() {
            return Some("This game does not update itself.".to_string());
        }
        Some(match self.blocked()? {
            Blocked::Store => "On iPhone and iPad, updates come from the App Store.".to_string(),
            Blocked::AgNow => "AG Now keeps this copy up to date.".to_string(),
            Blocked::DevBuild | Blocked::CargoBuild => {
                "This is a developer build, which never updates itself. Released copies do."
                    .to_string()
            }
        })
    }

    /// The channels the player can choose.
    pub fn channels(&self) -> Vec<String> {
        self.config
            .as_ref()
            .map_or_else(|| vec![DEFAULT_CHANNEL.to_string()], |c| c.channels.clone())
    }

    /// The channel followed: the player's choice, else the one this copy
    /// was released on, else the game's default; always one it lists.
    pub fn channel(&self, settings: &UpdateSettings) -> String {
        let channels = self.channels();
        let listed = |c: &str| channels.iter().any(|l| l == c);
        settings
            .channel
            .as_deref()
            .filter(|c| listed(c))
            .or_else(|| {
                self.running
                    .as_ref()
                    .map(|r| r.channel.as_str())
                    .filter(|c| listed(c))
            })
            .unwrap_or(&channels[0])
            .to_string()
    }

    /// Starts a check of `channel` on its own thread. `None` when this
    /// copy cannot update itself.
    pub fn check(&self, channel: &str) -> Option<Checker> {
        let config = self.config.as_ref()?;
        if self.blocked().is_some() {
            return None;
        }
        let running = self.running.clone()?;
        let source = config.source.clone();
        let fetch = self.fetch.clone();
        let host = self.host.clone();
        let wanted = channel.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let outcome =
                    hearken_update::check(source.as_ref(), fetch.as_ref(), &running, &wanted)
                        .map(|found| {
                            // Which files this copy already has, so the
                            // offer gives the smaller download. Hashing
                            // them takes a while; it is on this thread.
                            found.map(|mut update| {
                                update.plan =
                                    hearken_update::plan(&update, fetch.as_ref(), &host.install());
                                update
                            })
                        })
                        .map_err(|e| {
                            log::warn!("update check of {}: {e}", source.describe());
                            check_error_line(&e)
                        });
                let _ = tx.send(outcome);
            });
        if let Err(e) = spawned {
            log::warn!("could not start the update check: {e}");
            return Some(Checker::finished(Err(
                "The update check could not start.".to_string()
            )));
        }
        Some(Checker { rx, done: None })
    }

    /// The check at start, when the game and the player want one.
    pub fn check_at_start(&mut self, settings: &UpdateSettings) {
        let wanted = self.config.as_ref().is_some_and(|c| c.check_at_start);
        if wanted && settings.check_at_start && self.background.is_none() {
            self.background = self.check(&self.channel(settings));
        }
    }

    /// Collects the check at start. Returns the update it found, once,
    /// unless the player skipped that version.
    pub(crate) fn poll_background(&mut self, settings: &UpdateSettings) -> Option<Update> {
        let outcome = self.background.as_mut()?.poll()?.clone();
        self.background = None;
        match outcome {
            Ok(Some(update)) if settings.skipped.as_deref() != Some(update.version()) => {
                self.found = Some(update.clone());
                Some(update)
            }
            _ => None,
        }
    }

    /// The update the check at start found, until the player skips it or
    /// changes channel.
    pub fn available(&self) -> Option<&Update> {
        self.found.as_ref()
    }

    /// Forgets the update the check at start found.
    pub fn forget(&mut self) {
        self.found = None;
    }

    /// Whether the check at start found an update the game should announce.
    pub(crate) fn announces(&self) -> bool {
        self.config.as_ref().is_some_and(|c| c.announce)
    }

    /// Where to get an update by hand, as a sentence, or "".
    pub fn manual_line(&self) -> String {
        match self.config.as_ref().and_then(|c| c.manual.as_deref()) {
            Some(place) => format!("To update by hand, download it from {place}."),
            None => String::new(),
        }
    }

    /// Starts the swap: the script waits for the game to exit.
    pub fn apply(&self, staged: &Staged, install: &Install, staging: &Path) -> io::Result<()> {
        (self.apply)(staged, install, staging, &self.host)
    }

    /// Stands in for the network, for tests.
    pub fn set_fetch(&mut self, fetch: Arc<dyn Fetch>) {
        self.fetch = fetch;
    }

    /// Stands in for this copy's baked identity, for tests.
    pub fn set_running(&mut self, running: Option<Running>) {
        self.running = running;
    }

    /// Stands in for this process, for tests.
    pub fn set_host(&mut self, host: Host) {
        self.host = host;
    }

    /// Whether AG Now runs this copy, for tests.
    pub fn set_agnow(&mut self, agnow: bool) {
        self.agnow = agnow;
    }

    /// Stands in for the swap, for tests.
    pub fn set_apply(&mut self, apply: ApplyFn) {
        self.apply = apply;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> UpdateConfig {
        UpdateConfig::manifest_url("My Game", "https://example.com/{channel}/{platform}.json")
            .unwrap()
            .channels(&["stable", "beta", "Bad Name", "beta"])
    }

    fn running(channel: &str) -> Running {
        Running {
            key: hearken_update::SecretKey::generate().unwrap().public(),
            game: "my-game".into(),
            version: "1.0.0".into(),
            channel: channel.into(),
            platform: "linux-x64".into(),
        }
    }

    #[test]
    fn the_channel_is_the_players_then_the_copys_then_the_default() {
        let mut updates = Updates::new(Some(config()));
        assert_eq!(updates.channels(), ["stable", "beta"]);
        let mut settings = UpdateSettings::default();
        assert_eq!(updates.channel(&settings), "stable");
        updates.set_running(Some(running("beta")));
        assert_eq!(updates.channel(&settings), "beta");
        settings.channel = Some("stable".into());
        assert_eq!(updates.channel(&settings), "stable");
        // A channel the game no longer lists falls back.
        settings.channel = Some("nightly".into());
        assert_eq!(updates.channel(&settings), "beta");
    }

    #[test]
    fn a_developer_build_says_why_and_checks_nothing() {
        let mut updates = Updates::new(Some(config()));
        updates.set_running(None);
        assert!(updates.blocked_line().unwrap().contains("developer build"));
        assert!(updates.check("stable").is_none());
        updates.set_running(Some(running("stable")));
        updates.set_host(Host::fake(
            hearken_update::Os::current(),
            Path::new("/games/g/g"),
        ));
        updates.set_agnow(true);
        assert_eq!(
            updates.blocked_line().as_deref(),
            Some("AG Now keeps this copy up to date.")
        );
        updates.set_agnow(false);
        assert_eq!(updates.blocked_line(), None);
        assert!(Updates::new(None).blocked_line().is_some());
    }

    #[test]
    fn github_configs_say_where_to_update_by_hand() {
        let config = UpdateConfig::github("My Game", "me", "my-game").unwrap();
        let updates = Updates::new(Some(config));
        assert_eq!(
            updates.manual_line(),
            "To update by hand, download it from github.com/me/my-game/releases."
        );
        assert!(UpdateConfig::github("My Game", "me", "../x").is_none());
        assert!(UpdateConfig::manifest_url("My Game", "http://example.com/x.json").is_none());
    }
}
