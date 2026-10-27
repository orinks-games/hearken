//! The folder's history: its id, and how many times each sealed file has
//! been written.
//!
//! Its plaintext starts with `folder\t<32 hex digits>\n`, the folder's
//! random id, then has one line per sealed file, sorted by name:
//! `name\tgeneration\n` for a file that should be there, or
//! `name\tgeneration\tgone\n` for one the game removed or set aside. A gone
//! file keeps its generation, so an old copy put back later still reads as
//! rolled back, and a plain file in its place still reads as unsealed. It is
//! itself sealed, under the reserved name [`LEDGER`](crate::LEDGER).

use std::collections::BTreeMap;

use crate::format::FolderId;

/// One sealed file's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// How many times it has been written in this folder.
    pub generation: u64,
    /// The game removed it or set it aside on purpose.
    pub gone: bool,
}

/// Each sealed file's history.
pub type Entries = BTreeMap<String, Entry>;

const FOLDER: &str = "folder";
const GONE: &str = "gone";

pub fn to_text(folder: &FolderId, entries: &Entries) -> String {
    let mut text = format!("{FOLDER}\t{}\n", hex(folder));
    for (name, entry) in entries {
        text.push_str(&format!("{name}\t{}", entry.generation));
        if entry.gone {
            text.push('\t');
            text.push_str(GONE);
        }
        text.push('\n');
    }
    text
}

/// The folder id and entries in `text`, or `None` when it is not a ledger
/// this crate wrote. Only reached once the seal has authenticated, so a
/// failure here means a bug or a forged key, never an edit.
pub fn from_text(text: &[u8]) -> Option<(FolderId, Entries)> {
    let text = std::str::from_utf8(text).ok()?;
    let mut lines = text.lines();
    let folder = lines
        .next()?
        .strip_prefix(FOLDER)?
        .strip_prefix('\t')
        .and_then(unhex)?;
    let mut entries = Entries::new();
    for line in lines {
        let mut fields = line.split('\t');
        let name = fields.next()?;
        crate::name::check(name).ok()?;
        let generation = fields.next()?.parse().ok()?;
        let gone = match fields.next() {
            None => false,
            Some(GONE) => true,
            Some(_) => return None,
        };
        if fields.next().is_some() {
            return None;
        }
        entries.insert(name.to_string(), Entry { generation, gone });
    }
    Some((folder, entries))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(hex: &str) -> Option<FolderId> {
    if hex.len() != 32 || !hex.is_ascii() {
        return None;
    }
    let mut id = [0u8; 16];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let folder = [0xab; 16];
        let mut entries = Entries::new();
        entries.insert(
            "b.json".into(),
            Entry {
                generation: 2,
                gone: true,
            },
        );
        entries.insert(
            "a.json".into(),
            Entry {
                generation: 17,
                gone: false,
            },
        );
        let text = to_text(&folder, &entries);
        let id = "ab".repeat(16);
        assert_eq!(text, format!("folder\t{id}\na.json\t17\nb.json\t2\tgone\n"));
        assert_eq!(from_text(text.as_bytes()), Some((folder, entries)));
        let empty = format!("folder\t{id}\n");
        assert_eq!(from_text(empty.as_bytes()), Some((folder, Entries::new())));
        for bad in [
            String::new(),
            "a.json\t3\n".to_string(),
            format!("folder\t{id}\na.json 3\n"),
            format!("folder\t{id}\n../a\t3\n"),
            format!("folder\t{id}\na.json\t3\tlost\n"),
            format!("folder\t{id}\na.json\t3\tgone\tx\n"),
            "folder\tabc\n".to_string(),
        ] {
            assert_eq!(from_text(bad.as_bytes()), None, "{bad:?}");
        }
    }
}
