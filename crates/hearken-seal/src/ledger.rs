//! The folder's history: how many times each sealed file has been written.
//!
//! Its plaintext is one line per sealed file, `name\tgeneration\n`, sorted
//! by name. It is itself sealed, under the reserved name
//! [`LEDGER`](crate::LEDGER).

use std::collections::BTreeMap;

/// Each sealed file's latest generation.
pub type Entries = BTreeMap<String, u64>;

pub fn to_text(entries: &Entries) -> String {
    entries
        .iter()
        .map(|(name, generation)| format!("{name}\t{generation}\n"))
        .collect()
}

/// The entries in `text`, or `None` when it is not a ledger this crate
/// wrote. Only reached once the seal has authenticated, so a failure here
/// means a bug or a forged key, never an edit.
pub fn from_text(text: &[u8]) -> Option<Entries> {
    let text = std::str::from_utf8(text).ok()?;
    let mut entries = Entries::new();
    for line in text.lines() {
        let (name, generation) = line.split_once('\t')?;
        crate::name::check(name).ok()?;
        entries.insert(name.to_string(), generation.parse().ok()?);
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut entries = Entries::new();
        entries.insert("b.json".into(), 2);
        entries.insert("a.json".into(), 17);
        let text = to_text(&entries);
        assert_eq!(text, "a.json\t17\nb.json\t2\n");
        assert_eq!(from_text(text.as_bytes()), Some(entries));
        assert_eq!(from_text(b""), Some(Entries::new()));
        assert_eq!(from_text(b"a.json 3\n"), None);
        assert_eq!(from_text(b"../a\t3\n"), None);
    }
}
