//! Version numbers, as Cargo writes them: `1.4.0`, `1.5.0-beta.2`.

use std::cmp::Ordering;

/// A game's version, parsed. A leading `v` is allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version(semver::Version);

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        semver::Version::parse(text).ok().map(Version)
    }

    /// Release order: a pre-release (`1.5.0-beta.2`) comes before the
    /// release it leads to, and build metadata (`+linux`) is ignored, so two
    /// builds of one version are equal.
    pub fn precedence(&self, other: &Version) -> Ordering {
        let (a, b) = (&self.0, &other.0);
        (a.major, a.minor, a.patch)
            .cmp(&(b.major, b.minor, b.patch))
            .then_with(|| a.pre.cmp(&b.pre))
    }

    pub fn is_newer_than(&self, other: &Version) -> bool {
        self.precedence(other) == Ordering::Greater
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A version as a player hears it: `1.5.0-beta.2` is "1.5.0 beta 2", and
/// build metadata is left out.
pub fn spoken_version(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let text = text.split('+').next().unwrap_or(text);
    match text.split_once('-') {
        Some((base, pre)) => format!("{base} {}", pre.replace(['.', '-'], " ")),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn newer(a: &str, b: &str) -> bool {
        Version::parse(a)
            .unwrap()
            .is_newer_than(&Version::parse(b).unwrap())
    }

    #[test]
    fn versions_order_as_releases_do() {
        assert!(newer("1.0.1", "1.0.0"));
        assert!(newer("1.10.0", "1.9.9"));
        assert!(newer("2.0.0", "1.99.99"));
        assert!(newer("1.5.0", "1.5.0-beta.2"));
        assert!(newer("1.5.0-beta.10", "1.5.0-beta.2"));
        assert!(newer("1.5.0-beta.1", "1.5.0-alpha.9"));
        assert!(newer("1.5.0-rc.1", "1.5.0-beta.9"));
        assert!(newer("v1.2.0", "1.1.0"));
        assert!(!newer("1.0.0", "1.0.0"));
        assert!(!newer("1.0.0", "1.0.1"));
        assert!(!newer("1.5.0-beta.2", "1.5.0"));
        // Build metadata is not a newer version.
        assert!(!newer("1.0.0+rebuilt", "1.0.0"));
        assert!(!newer("1.0.0", "1.0.0+rebuilt"));
    }

    #[test]
    fn only_real_versions_parse() {
        assert!(Version::parse("1.2").is_none());
        assert!(Version::parse("latest").is_none());
        assert!(Version::parse("").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(Version::parse(" 1.2.3 ").is_some());
    }

    #[test]
    fn versions_are_spoken_plainly() {
        assert_eq!(spoken_version("1.4.0"), "1.4.0");
        assert_eq!(spoken_version("v1.5.0-beta.2"), "1.5.0 beta 2");
        assert_eq!(spoken_version("1.5.0-rc.1+linux"), "1.5.0 rc 1");
    }
}
