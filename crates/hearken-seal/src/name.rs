//! Which file names a game may seal.

use std::io;

/// The longest name, in bytes.
pub const MAX_LEN: usize = 64;

/// Checks that `name` is a single, ordinary file name: `[A-Za-z0-9._-]`,
/// not starting with `.`, at most 64 bytes, and not one of
/// the names the vault uses beside a file (`.plain-backup`, `.tampered`).
pub fn check(name: &str) -> io::Result<()> {
    let ok = !name.is_empty()
        && name.len() <= MAX_LEN
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !name.ends_with(BACKUP)
        && !name.contains(TAMPERED);
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not a name a data file can be sealed under"),
        ))
    }
}

/// What a plain file is kept as when a sealing build first writes over it.
pub const BACKUP: &str = ".plain-backup";

/// What a file the game set aside is renamed to, with `-2`, `-3`... after
/// it when that is taken.
pub const TAMPERED: &str = ".tampered";

/// The temporary file a write goes through: hidden, so it can never be a
/// game's own name.
pub fn temp(name: &str) -> String {
    format!(".{name}.tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_names_in_the_folder() {
        for good in ["records.json", "save-1.dat", "A_b.c", &"x".repeat(64)] {
            assert!(check(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "../x",
            "a/b",
            "a\\b",
            ".hidden",
            "..",
            "c:x",
            "a b",
            "é.json",
            "records.json.plain-backup",
            "records.json.tampered",
            "records.json.tampered-2",
            &"x".repeat(65),
        ] {
            let err = check(bad).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{bad}");
        }
    }
}
