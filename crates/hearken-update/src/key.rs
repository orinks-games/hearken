//! The game's update signing key.
//!
//! The secret half signs each release's manifest and stays with the
//! developer: in a key file under their config folder, or a CI secret. The
//! public half is committed to the game's `Cargo.toml` as `update-key` and
//! built into every release, which then trusts only manifests it signed.

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

/// How a public key is written: `ed25519:` and 64 hex digits.
pub const PUBLIC_PREFIX: &str = "ed25519:";

/// A public update key: what a release checks manifests against.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(pub(crate) [u8; 32]);

impl PublicKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Option<PublicKey> {
        // Refuses bytes that are not a point on the curve.
        VerifyingKey::from_bytes(&bytes).ok()?;
        Some(PublicKey(bytes))
    }

    /// `ed25519:<64 hex digits>`; the prefix may be left out.
    pub fn parse(text: &str) -> Option<PublicKey> {
        let text = text.trim();
        let hex = text.strip_prefix(PUBLIC_PREFIX).unwrap_or(text);
        PublicKey::from_bytes(from_hex::<32>(hex)?)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn verifying(&self) -> Option<VerifyingKey> {
        VerifyingKey::from_bytes(&self.0).ok()
    }
}

impl std::fmt::Display for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{PUBLIC_PREFIX}{}", to_hex(&self.0))
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

/// A secret update key. `Debug` never shows it.
#[derive(Clone)]
pub struct SecretKey(SigningKey);

impl SecretKey {
    /// A new key from the operating system's random source.
    pub fn generate() -> std::io::Result<SecretKey> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(SecretKey(SigningKey::from_bytes(&seed)))
    }

    /// The key from its 64 hex digits, surrounding whitespace allowed.
    pub fn from_hex(hex: &str) -> Option<SecretKey> {
        Some(SecretKey(SigningKey::from_bytes(&from_hex::<32>(
            hex.trim(),
        )?)))
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0.to_bytes())
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key().to_bytes())
    }

    pub(crate) fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretKey(hidden, public {})", self.public())
    }
}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bytes() == other.0.to_bytes()
    }
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Exactly `N` bytes from `2 * N` hex digits.
pub(crate) fn from_hex<const N: usize>(hex: &str) -> Option<[u8; N]> {
    if hex.len() != N * 2 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_through_text() {
        let secret = SecretKey::generate().unwrap();
        let again = SecretKey::from_hex(&format!(" {}\n", secret.to_hex())).unwrap();
        assert_eq!(secret, again);
        let public = secret.public();
        let text = public.to_string();
        assert!(text.starts_with("ed25519:") && text.len() == 8 + 64);
        assert_eq!(PublicKey::parse(&text), Some(public));
        assert_eq!(PublicKey::parse(&text[8..]), Some(public));
        assert!(!format!("{secret:?}").contains(&secret.to_hex()));
    }

    #[test]
    fn bad_text_is_no_key() {
        assert!(SecretKey::from_hex("abc").is_none());
        assert!(SecretKey::from_hex(&"g".repeat(64)).is_none());
        assert!(PublicKey::parse("ed25519:").is_none());
        assert!(PublicKey::parse(&format!("ed25519:{}", "0".repeat(63))).is_none());
        assert!(from_hex::<2>("é1").is_none());
    }
}
