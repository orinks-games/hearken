//! Discord Rich Presence: the "Playing ..." status a player's friends see.
//!
//! A game opts in with [`crate::Game::discord`], giving its own Discord
//! application ID, and reports what the player is doing through
//! [`crate::State::presence`] (per screen) or [`crate::Game::presence`]
//! (a fallback). The engine asks each frame while the status is on, and
//! sends changes from a worker thread (see [`hearken_discord`]). When
//! Discord is not running nothing happens and nothing is said: the game
//! plays exactly as it would without it.
//!
//! The player turns it off from Settings ("Discord status"); it is on by
//! default, and Discord's own activity setting still applies.

pub use hearken_discord::Presence;
use hearken_discord::{DiscordPresence, DiscordPresenceOptions};

/// A game's Discord application, from
/// <https://discord.com/developers/applications>. Discord shows the
/// application's name as the game, so name the application after the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscordConfig {
    /// The application ID (Discord's "client ID"), all digits.
    pub app_id: String,
    /// Show how long this session has been running. On by default.
    pub show_elapsed: bool,
}

impl DiscordConfig {
    pub fn new(app_id: impl Into<String>) -> Self {
        DiscordConfig {
            app_id: app_id.into(),
            show_elapsed: true,
        }
    }

    /// No elapsed-time counter.
    pub fn without_elapsed(mut self) -> Self {
        self.show_elapsed = false;
        self
    }
}

/// The game's Discord status, at `ctx.discord`. Dormant (never touching
/// Discord) unless the game gave an application ID and Hearken was built
/// with its `discord` feature, and always dormant in tests and agent
/// sessions.
#[derive(Default)]
pub struct Discord {
    service: Option<DiscordPresence>,
}

impl Discord {
    /// Never reaches Discord.
    pub fn dormant() -> Self {
        Discord::default()
    }

    /// Over `service`, as a test does with a fake client.
    pub fn with_service(service: DiscordPresence) -> Self {
        Discord {
            service: Some(service),
        }
    }

    /// The real service for `config`, started when `enabled`.
    pub(crate) fn start(config: &DiscordConfig, enabled: bool) -> Self {
        let service = DiscordPresence::new(DiscordPresenceOptions {
            enabled,
            show_elapsed: config.show_elapsed,
            ..DiscordPresenceOptions::new(config.app_id.clone())
        });
        if !service.available() {
            if config.app_id.trim().is_empty() {
                log::warn!("discord: the game's application ID is empty; presence is off");
            } else {
                log::warn!(
                    "discord: the game gives an application ID, but Hearken was built \
                     without its discord feature; presence is off"
                );
            }
            return Discord::dormant();
        }
        service.start();
        log::info!("discord: presence {}", if enabled { "on" } else { "off" });
        Discord::with_service(service)
    }

    /// Whether the game offers Discord status, so Settings shows the switch.
    pub fn is_offered(&self) -> bool {
        self.service.as_ref().is_some_and(|s| s.available())
    }

    /// Whether the status is being kept up to date now.
    pub fn is_on(&self) -> bool {
        self.service.as_ref().is_some_and(|s| s.enabled())
    }

    /// Whether the worker is connected to a running Discord.
    pub fn connected(&self) -> bool {
        self.service.as_ref().is_some_and(|s| s.connected())
    }

    pub(crate) fn set_enabled(&self, on: bool) {
        if let Some(service) = &self.service {
            service.set_enabled(on);
        }
    }

    pub(crate) fn update(&self, presence: Option<Presence>) {
        if let Some(service) = &self.service {
            service.update(presence);
        }
    }

    /// Clears the status and stops the worker. Bounded: waits at most two
    /// seconds, and only for a worker holding a live connection.
    pub(crate) fn shutdown(&self) {
        if let Some(service) = &self.service {
            service.shutdown();
        }
    }
}
