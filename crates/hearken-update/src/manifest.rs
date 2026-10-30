//! The signed manifest: one small file per platform and channel that says
//! which archive is the update and what it must hash to.
//!
//! The file is a JSON envelope:
//!
//! ```json
//! {
//!   "format": 1,
//!   "key": "ed25519:…",
//!   "signature": "…128 hex digits…",
//!   "signed": "{\"format\":1,\"game\":\"my-game\",\"version\":\"1.4.0\",…}"
//! }
//! ```
//!
//! `signed` is the manifest itself, as a string, and the signature covers
//! its exact bytes (after a fixed domain prefix). Nothing in it is read
//! until the signature checks out against the key built into the game, so
//! a host, a mirror or anyone between them can change nothing the game acts
//! on. `key` only helps a developer see which key signed a file.

use serde::{Deserialize, Serialize};

use crate::key::{from_hex, to_hex, PublicKey, SecretKey};
use crate::version::Version;

/// The manifest format this crate writes and reads.
pub const FORMAT: u32 = 1;

/// The largest manifest file read.
pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// The longest release notes kept.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;

/// The largest archive a manifest may name.
pub const MAX_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Put before the manifest's bytes when signing, so a signature over a
/// manifest can never be passed off as a signature over anything else.
const DOMAIN: &[u8] = b"hearken-update manifest v1\n";

/// What a release says about itself. Every field is signed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    /// The game's package name, as in its `Cargo.toml`.
    pub game: String,
    pub version: String,
    pub channel: String,
    /// The platform the archive was built for, as `hearken release` names
    /// it: `windows-x64`, `linux-x64`, `macos-arm64`, `macos` (Intel).
    pub platform: String,
    /// The archive's file name.
    pub archive: String,
    /// Where the archive is, when it is not beside the manifest (or, on
    /// GitHub, an asset of the same release).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The archive's size in bytes.
    pub size: u64,
    /// The archive's SHA-256, as 64 lowercase hex digits.
    pub sha256: String,
    /// The release notes, as Markdown.
    #[serde(default)]
    pub notes: String,
    /// When the release was made, as the developer wrote it (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(default)]
    signature: String,
    signed: String,
}

/// Why a manifest file was not accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// Not a manifest file at all, or too big to be one.
    NotAManifest,
    /// A manifest format this game does not know.
    UnknownFormat(u32),
    /// No signature.
    Unsigned,
    /// The signature is not this game's key's. `signer` is the key the file
    /// says signed it, when it says.
    BadSignature { signer: Option<String> },
    /// Signed, but a field is out of shape.
    Malformed(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::NotAManifest => f.write_str("not an update manifest"),
            ManifestError::UnknownFormat(n) => write!(f, "unknown manifest format {n}"),
            ManifestError::Unsigned => f.write_str("the manifest is not signed"),
            ManifestError::BadSignature { signer: Some(k) } => {
                write!(f, "the manifest is signed by another key ({k})")
            }
            ManifestError::BadSignature { signer: None } => {
                f.write_str("the manifest's signature does not match this game's key")
            }
            ManifestError::Malformed(why) => write!(f, "the manifest is malformed: {why}"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Whether `name` is a plain id: 1 to 64 of letters, digits, `-` and `_`.
pub fn is_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether `name` is a channel: 1 to 32 of lowercase letters, digits and `-`.
pub fn is_channel(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether `name` is a file name safe to save and to put in a URL: letters,
/// digits, `-`, `_`, `.` and `+`, not starting with a dot.
pub fn is_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

impl Manifest {
    /// Every field in shape, as `open` requires and `sign` checks first.
    pub fn validate(&self) -> Result<(), ManifestError> {
        let bad = |why: &str| Err(ManifestError::Malformed(why.to_string()));
        if self.format != FORMAT {
            return Err(ManifestError::UnknownFormat(self.format));
        }
        if !is_id(&self.game) {
            return bad("game");
        }
        if Version::parse(&self.version).is_none() {
            return bad("version");
        }
        if !is_channel(&self.channel) {
            return bad("channel");
        }
        if !is_channel(&self.platform) {
            return bad("platform");
        }
        if !is_file_name(&self.archive) || archive_kind(&self.archive).is_none() {
            return bad("archive");
        }
        if let Some(url) = &self.url {
            if !crate::fetch::url_allowed(url) {
                return bad("url");
            }
        }
        if self.size == 0 || self.size > MAX_ARCHIVE_BYTES {
            return bad("size");
        }
        if from_hex::<32>(&self.sha256).is_none() || self.sha256.chars().any(|c| c.is_uppercase()) {
            return bad("sha256");
        }
        if self.notes.len() > MAX_NOTES_BYTES {
            return bad("notes");
        }
        Ok(())
    }

    pub fn parsed_version(&self) -> Option<Version> {
        Version::parse(&self.version)
    }

    /// The notes as plain lines to speak.
    pub fn note_lines(&self) -> Vec<String> {
        crate::notes::flatten_markdown(&self.notes)
    }
}

/// What kind of archive a file name is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
    /// One AppImage file: the update itself, nothing to unpack.
    AppImage,
}

pub fn archive_kind(name: &str) -> Option<ArchiveKind> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        Some(ArchiveKind::Zip)
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveKind::TarGz)
    } else if lower.ends_with(".appimage") {
        Some(ArchiveKind::AppImage)
    } else {
        None
    }
}

fn message(signed: &str) -> Vec<u8> {
    let mut message = DOMAIN.to_vec();
    message.extend_from_slice(signed.as_bytes());
    message
}

/// The manifest file for `manifest`, signed with `secret`.
pub fn sign(manifest: &Manifest, secret: &SecretKey) -> Result<String, ManifestError> {
    manifest.validate()?;
    let signed =
        serde_json::to_string(manifest).map_err(|e| ManifestError::Malformed(e.to_string()))?;
    let envelope = Envelope {
        format: FORMAT,
        key: Some(secret.public().to_string()),
        signature: to_hex(&secret.sign(&message(&signed))),
        signed,
    };
    let mut text = serde_json::to_string_pretty(&envelope)
        .map_err(|e| ManifestError::Malformed(e.to_string()))?;
    text.push('\n');
    Ok(text)
}

/// The manifest in `bytes`, once its signature checks out against `key`.
/// Nothing signed is read before that.
pub fn open(bytes: &[u8], key: &PublicKey) -> Result<Manifest, ManifestError> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ManifestError::NotAManifest);
    }
    let envelope: Envelope =
        serde_json::from_slice(bytes).map_err(|_| ManifestError::NotAManifest)?;
    if envelope.format != FORMAT {
        return Err(ManifestError::UnknownFormat(envelope.format));
    }
    if envelope.signature.trim().is_empty() {
        return Err(ManifestError::Unsigned);
    }
    let bad = || ManifestError::BadSignature {
        signer: envelope
            .key
            .as_deref()
            .and_then(PublicKey::parse)
            .filter(|k| k != key)
            .map(|k| k.to_string()),
    };
    let signature = from_hex::<64>(envelope.signature.trim()).ok_or_else(bad)?;
    let verifying = key.verifying().ok_or_else(bad)?;
    verifying
        .verify_strict(
            &message(&envelope.signed),
            &ed25519_dalek::Signature::from_bytes(&signature),
        )
        .map_err(|_| bad())?;
    let manifest: Manifest = serde_json::from_str(&envelope.signed)
        .map_err(|e| ManifestError::Malformed(e.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn sample() -> Manifest {
        Manifest {
            format: FORMAT,
            game: "my-game".into(),
            version: "1.4.0".into(),
            channel: "stable".into(),
            platform: "linux-x64".into(),
            archive: "my-game-1.4.0-linux-x64.tar.gz".into(),
            url: None,
            size: 1234,
            sha256: "ab".repeat(32),
            notes: "## Fixed\n- The menu no longer skips a row.".into(),
            published: Some("2026-10-30T02:00:00Z".into()),
        }
    }

    #[test]
    fn a_signed_manifest_opens_with_its_key() {
        let secret = SecretKey::generate().unwrap();
        let text = sign(&sample(), &secret).unwrap();
        assert_eq!(open(text.as_bytes(), &secret.public()), Ok(sample()));
        assert!(!text.contains(&secret.to_hex()));
    }

    #[test]
    fn another_key_is_refused_and_named() {
        let secret = SecretKey::generate().unwrap();
        let other = SecretKey::generate().unwrap();
        let text = sign(&sample(), &other).unwrap();
        assert_eq!(
            open(text.as_bytes(), &secret.public()),
            Err(ManifestError::BadSignature {
                signer: Some(other.public().to_string())
            })
        );
    }

    /// Changing any byte of the signed text, or the signature, is caught.
    #[test]
    fn tampering_is_caught() {
        let secret = SecretKey::generate().unwrap();
        let text = sign(&sample(), &secret).unwrap();
        let mut envelope: serde_json::Value = serde_json::from_str(&text).unwrap();
        let signed = envelope["signed"].as_str().unwrap().to_string();
        for (from, to) in [
            ("1.4.0", "9.4.0"),
            ("linux-x64", "linux-arm"),
            ("\"size\":1234", "\"size\":99999"),
            ("abab", "abac"),
            ("stable", "stabld"),
        ] {
            let mut forged = envelope.clone();
            forged["signed"] = signed.replacen(from, to, 1).into();
            let bytes = serde_json::to_vec(&forged).unwrap();
            assert!(
                matches!(
                    open(&bytes, &secret.public()),
                    Err(ManifestError::BadSignature { .. })
                ),
                "{from} -> {to}"
            );
        }
        let sig = envelope["signature"].as_str().unwrap().to_string();
        let flipped = if sig.starts_with('0') { "1" } else { "0" };
        envelope["signature"] = format!("{flipped}{}", &sig[1..]).into();
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(matches!(
            open(&bytes, &secret.public()),
            Err(ManifestError::BadSignature { signer: None })
        ));
    }

    #[test]
    fn a_manifest_without_a_signature_is_refused() {
        let secret = SecretKey::generate().unwrap();
        let signed = serde_json::to_string(&sample()).unwrap();
        let bare = serde_json::json!({ "format": 1, "signed": signed });
        assert_eq!(
            open(&serde_json::to_vec(&bare).unwrap(), &secret.public()),
            Err(ManifestError::Unsigned)
        );
        // The manifest alone, with no envelope, is not a manifest file.
        assert_eq!(
            open(signed.as_bytes(), &secret.public()),
            Err(ManifestError::NotAManifest)
        );
        assert_eq!(
            open(b"<html>not found</html>", &secret.public()),
            Err(ManifestError::NotAManifest)
        );
        let huge = vec![b' '; MAX_MANIFEST_BYTES as usize + 1];
        assert_eq!(
            open(&huge, &secret.public()),
            Err(ManifestError::NotAManifest)
        );
    }

    #[test]
    fn nothing_out_of_shape_is_signed_or_opened() {
        let secret = SecretKey::generate().unwrap();
        let cases: Vec<fn(&mut Manifest)> = vec![
            |m| m.game = "my game".into(),
            |m| m.version = "latest".into(),
            |m| m.channel = "Stable".into(),
            |m| m.platform = "../linux".into(),
            |m| m.archive = "../evil.zip".into(),
            |m| m.archive = "dir/evil.zip".into(),
            |m| m.archive = "game.exe".into(),
            |m| m.archive = ".hidden.zip".into(),
            |m| m.url = Some("http://example.com/game.zip".into()),
            |m| m.url = Some("file:///etc/passwd".into()),
            |m| m.size = 0,
            |m| m.size = MAX_ARCHIVE_BYTES + 1,
            |m| m.sha256 = "AB".repeat(32),
            |m| m.sha256 = "ab".repeat(31),
            |m| m.notes = "x".repeat(MAX_NOTES_BYTES + 1),
            |m| m.format = 2,
        ];
        for (i, change) in cases.into_iter().enumerate() {
            let mut manifest = sample();
            change(&mut manifest);
            assert!(sign(&manifest, &secret).is_err(), "case {i}");
            // Signed by hand, it is still refused once opened.
            let signed = serde_json::to_string(&manifest).unwrap();
            let envelope = serde_json::json!({
                "format": 1,
                "signature": to_hex(&secret.sign(&message(&signed))),
                "signed": signed,
            });
            let opened = open(&serde_json::to_vec(&envelope).unwrap(), &secret.public());
            assert!(
                matches!(
                    opened,
                    Err(ManifestError::Malformed(_) | ManifestError::UnknownFormat(_))
                ),
                "case {i}: {opened:?}"
            );
        }
    }

    #[test]
    fn archives_are_known_by_their_names() {
        assert_eq!(
            archive_kind("a-1.0.0-windows-x64.zip"),
            Some(ArchiveKind::Zip)
        );
        assert_eq!(archive_kind("a.tar.gz"), Some(ArchiveKind::TarGz));
        assert_eq!(archive_kind("a.tgz"), Some(ArchiveKind::TarGz));
        assert_eq!(
            archive_kind("A-x86_64.AppImage"),
            Some(ArchiveKind::AppImage)
        );
        assert_eq!(archive_kind("a.exe"), None);
    }
}
