//! A released game updating itself, from wherever its developer publishes.
//!
//! The steps, each its own module:
//!
//! 1. [`source`]: find the signed manifest for this platform and channel,
//!    on GitHub releases, at any HTTPS URL, or from a source the game
//!    writes itself.
//! 2. [`check`]: refuse it unless it is signed with the key built into this
//!    copy, is for this game, platform and channel, and is newer than this
//!    copy. Every source gets the same checks.
//! 3. [`download`]: fetch the archive it names, no more than its signed
//!    size, and check its signed SHA-256 before anything opens it.
//! 4. [`install`] and [`extract`]: unpack it as untrusted input, and find
//!    what replaces the running install.
//! 5. [`apply`]: a detached script waits for the game to exit, swaps the
//!    update in with a way back if that fails, and starts the game again.
//!
//! Only a copy built by `hearken release` with an update key (see
//! [`Running::baked`]) updates itself. Developer builds never do, nor a copy
//! on iOS (the App Store updates it), nor one run by AG Now, which updates
//! its games itself. See `docs/updates.md` for publishing.
//!
//! Nothing here knows any game. The engine's screens are in `hearken-app`
//! behind its `update` feature.

pub mod apply;
pub mod check;
pub mod download;
pub mod extract;
pub mod fetch;
pub mod install;
pub mod key;
pub mod manifest;
pub mod notes;
pub mod platform;
pub mod source;
pub mod version;

/// What `hearken release` baked into this build (see `build.rs`).
mod baked {
    include!(concat!(env!("OUT_DIR"), "/baked.rs"));
}

pub use apply::{apply_and_restart, apply_and_restart_with};
pub use check::{check, CheckError, Refusal, Running, Update};
pub use download::{download, DownloadError, IDLE_TIMEOUT};
pub use extract::Limits;
#[cfg(feature = "http")]
pub use fetch::HttpFetch;
pub use fetch::{Body, Fetch, FetchError};
pub use install::{
    can_auto_apply, make_staging_dir, stage, stash_for_manual_install, Host, Install, Os, Staged,
};
pub use key::{PublicKey, SecretKey};
pub use manifest::{Manifest, ManifestError};
pub use notes::flatten_markdown;
pub use source::{Found, GitHubReleases, ManifestUrl, Query, SourceError, UpdateSource};
pub use version::{spoken_version, Version};

/// The channel a game's players follow unless they choose another.
pub const DEFAULT_CHANNEL: &str = "stable";

/// Whether this platform lets a program replace itself. On iOS the App
/// Store or TestFlight does that.
pub const SELF_UPDATES: bool = !cfg!(any(target_os = "ios", target_os = "android"));

/// Whether a value of `AGNOW` means the game runs on AG Now, which sets
/// `AGNOW=1` in every session it starts and updates its games itself. It
/// also drops anything a game writes into its own folder when the session
/// ends, so an update there would cost the download and change nothing.
pub fn agnow_value_set(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|v| {
        let v = v.to_string_lossy();
        let v = v.trim();
        !v.is_empty() && v != "0"
    })
}

/// True when AG Now started this copy.
pub fn managed_by_agnow() -> bool {
    agnow_value_set(std::env::var_os("AGNOW").as_deref())
}

/// Why this copy does not update itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// iOS: the App Store or TestFlight updates it.
    Store,
    /// AG Now keeps it up to date.
    AgNow,
    /// Not built by `hearken release` with an update key.
    DevBuild,
    /// Run from a Cargo `target` folder, not an install.
    CargoBuild,
}

/// Why `host`, running as `running`, cannot update itself; `None` when it
/// can. `agnow` is whether AG Now started it.
pub fn blocked(running: Option<&Running>, host: &Host, agnow: bool) -> Option<Blocked> {
    if !SELF_UPDATES {
        return Some(Blocked::Store);
    }
    if agnow {
        return Some(Blocked::AgNow);
    }
    if running.is_none() {
        return Some(Blocked::DevBuild);
    }
    if install::in_cargo_target(&host.exe) {
        return Some(Blocked::CargoBuild);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agnow_is_on_for_any_value_but_nothing_or_zero() {
        use std::ffi::OsStr;
        assert!(agnow_value_set(Some(OsStr::new("1"))));
        assert!(agnow_value_set(Some(OsStr::new("yes"))));
        assert!(!agnow_value_set(Some(OsStr::new("0"))));
        assert!(!agnow_value_set(Some(OsStr::new(" "))));
        assert!(!agnow_value_set(None));
    }

    #[test]
    fn a_build_without_a_key_never_updates() {
        // Tests are built without HEARKEN_UPDATE_KEY.
        assert_eq!(Running::baked(), None);
        let host = Host::fake(Os::Linux, std::path::Path::new("/games/g/g"));
        assert_eq!(blocked(None, &host, false), Some(Blocked::DevBuild));
        assert_eq!(blocked(None, &host, true), Some(Blocked::AgNow));
    }
}
