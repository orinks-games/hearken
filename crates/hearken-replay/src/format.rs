//! The run file: a compact, versioned binary record of one run.
//!
//! ```text
//! "HKRN"                magic
//! u8                    format version (1)
//! [u8; 32]              build: the SHA-256 of the rules module that made it
//! varint + UTF-8        rules: the game's name for the rules it ran
//! u64, little-endian    seed
//! varint + UTF-8        context: canonical JSON
//! varint                ticks: how many steps the run took
//! varint                how many inputs follow
//! per input:
//!   varint              ticks since the previous input (the first: since 0)
//!   varint              code
//!   varint (zigzag)     value
//! ```
//!
//! The build sits at a fixed offset (byte 5) so a server can pick the
//! rules module before it parses anything else. Every field has exactly
//! one encoding: varints may not carry extra bytes, the context must be in
//! canonical form and nothing may follow the last input. So two run files
//! differ only when the runs do, and [`Run::id`] can name a run.

use sha2::{Digest, Sha256};

/// The first four bytes of every run file.
pub const MAGIC: [u8; 4] = *b"HKRN";

/// The run file's format version.
pub const FORMAT_VERSION: u8 = 1;

/// Where the build's 32 bytes start in a run file.
pub const BUILD_OFFSET: usize = 5;

/// The largest run file read: 4 MiB, far more than hours of input.
pub const MAX_RUN_BYTES: usize = 4 << 20;

/// The longest rules name, in bytes.
pub const MAX_RULES_LEN: usize = 64;

/// The longest context, in bytes of JSON.
pub const MAX_CONTEXT_LEN: usize = 16 * 1024;

/// The most inputs one run may hold.
pub const MAX_INPUTS: usize = 1 << 20;

/// Which rules module made a run: the SHA-256 of that `.wasm` file, which
/// `hearken release` builds and bakes into the game. All zero for a build
/// with none, which cannot go online.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct BuildId(pub [u8; 32]);

impl BuildId {
    /// No rules module: a developer build, or a game that is not online.
    pub const NONE: BuildId = BuildId([0; 32]);

    /// Whether this is [`BuildId::NONE`].
    pub fn is_none(&self) -> bool {
        *self == BuildId::NONE
    }

    /// The SHA-256 of `module`, the rules module's bytes.
    pub fn of_module(module: &[u8]) -> BuildId {
        BuildId(Sha256::digest(module).into())
    }

    /// 64 lowercase hex digits.
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }

    /// Reads 64 hex digits; `None` for anything else.
    pub fn from_hex(text: &str) -> Option<BuildId> {
        let text = text.trim();
        if text.len() != 64 || !text.is_ascii() {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(BuildId(out))
    }

    /// The rules ID `hearken release` baked into this program; `None` for
    /// any other build.
    pub fn baked() -> Option<BuildId> {
        option_env!("HEARKEN_RULES_ID")
            .and_then(BuildId::from_hex)
            .filter(|id| !id.is_none())
    }

    /// The rules module this program was released with: the ID `hearken
    /// release` baked in through `HEARKEN_RULES_ID`. A build without one
    /// reads `HEARKEN_RULES_ID` when it runs instead, so a developer can try
    /// a local server; a build with neither gets [`BuildId::NONE`].
    pub fn this_build() -> BuildId {
        if let Some(id) = BuildId::baked() {
            return id;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(id) = std::env::var("HEARKEN_RULES_ID")
            .ok()
            .and_then(|v| BuildId::from_hex(&v))
        {
            return id;
        }
        BuildId::NONE
    }
}

/// One input: at step `tick`, the game's input `code` with `value` (1 for
/// a press and 0 for a release, by convention, or any number the game
/// gives it: a menu choice, an axis).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Input {
    pub tick: u64,
    pub code: u16,
    pub value: i32,
}

/// One recorded run: everything needed to play it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The rules module that recorded it.
    pub build: BuildId,
    /// The game's name for the rules that ran (`Replayable::RULES`).
    pub rules: String,
    /// The seed the rules started from.
    pub seed: u64,
    /// The rules' starting context, as canonical JSON (see [`canonical_json`]).
    pub context: String,
    /// How many steps the run took.
    pub ticks: u64,
    /// Every input, in order of tick. Inputs on one tick keep their order.
    pub inputs: Vec<Input>,
}

/// Why a run file could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    /// Not a run file, or one of a format version this build does not read.
    NotARun,
    /// Larger than [`MAX_RUN_BYTES`].
    TooLarge,
    /// It ends partway through a field.
    Truncated,
    /// A varint with extra bytes, or too large for its field.
    BadNumber,
    /// A rules name that is empty, too long, or not `[a-z0-9_-]`.
    BadRules,
    /// A context that is not canonical JSON, or is too long.
    BadContext,
    /// An input on or after the run's last tick, or more than
    /// [`MAX_INPUTS`].
    BadInputs,
    /// Bytes after the last input.
    TrailingBytes,
}

impl FormatError {
    /// A short stable name, for logs and a server's reply.
    pub fn code(&self) -> &'static str {
        match self {
            FormatError::NotARun => "not-a-run",
            FormatError::TooLarge => "too-large",
            FormatError::Truncated => "truncated",
            FormatError::BadNumber => "bad-number",
            FormatError::BadRules => "bad-rules-name",
            FormatError::BadContext => "bad-context",
            FormatError::BadInputs => "bad-inputs",
            FormatError::TrailingBytes => "trailing-bytes",
        }
    }
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for FormatError {}

/// Whether `name` can name rules: 1 to 64 of `a-z`, `0-9`, `-` and `_`.
pub fn valid_rules_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_RULES_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `value` as canonical JSON: no spaces, object keys sorted by their
/// bytes, numbers as serde_json writes them. The same value gives the same
/// text on every platform and with any serde_json features.
pub fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                // A string's JSON form cannot fail.
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                write_canonical(&map[key.as_str()], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_default()),
    }
}

/// Whether `text` is JSON already in canonical form.
pub fn is_canonical_json(text: &str) -> bool {
    text.len() <= MAX_CONTEXT_LEN
        && serde_json::from_str::<serde_json::Value>(text)
            .is_ok_and(|value| canonical_json(&value) == text)
}

impl Run {
    /// The run file. Fails, writing nothing, for a run that could not be
    /// read back: a bad rules name or context, inputs out of order or past
    /// the last tick, or a file over [`MAX_RUN_BYTES`].
    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        if !valid_rules_name(&self.rules) {
            return Err(FormatError::BadRules);
        }
        if !is_canonical_json(&self.context) {
            return Err(FormatError::BadContext);
        }
        if self.inputs.len() > MAX_INPUTS {
            return Err(FormatError::BadInputs);
        }
        let mut out = Vec::with_capacity(64 + self.context.len() + self.inputs.len() * 4);
        out.extend_from_slice(&MAGIC);
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&self.build.0);
        put_varint(&mut out, self.rules.len() as u64);
        out.extend_from_slice(self.rules.as_bytes());
        out.extend_from_slice(&self.seed.to_le_bytes());
        put_varint(&mut out, self.context.len() as u64);
        out.extend_from_slice(self.context.as_bytes());
        put_varint(&mut out, self.ticks);
        put_varint(&mut out, self.inputs.len() as u64);
        let mut last = 0u64;
        for input in &self.inputs {
            if input.tick < last || input.tick >= self.ticks {
                return Err(FormatError::BadInputs);
            }
            put_varint(&mut out, input.tick - last);
            put_varint(&mut out, u64::from(input.code));
            put_varint(&mut out, zigzag(input.value));
            last = input.tick;
        }
        if out.len() > MAX_RUN_BYTES {
            return Err(FormatError::TooLarge);
        }
        Ok(out)
    }

    /// Reads a run file, refusing anything [`Run::encode`] would not have
    /// written byte for byte.
    pub fn decode(bytes: &[u8]) -> Result<Run, FormatError> {
        if bytes.len() > MAX_RUN_BYTES {
            return Err(FormatError::TooLarge);
        }
        let mut r = Reader { bytes, at: 0 };
        if r.take(4)? != MAGIC || r.byte()? != FORMAT_VERSION {
            return Err(FormatError::NotARun);
        }
        let mut build = [0u8; 32];
        build.copy_from_slice(r.take(32)?);
        let rules = r.string(MAX_RULES_LEN, FormatError::BadRules)?;
        if !valid_rules_name(&rules) {
            return Err(FormatError::BadRules);
        }
        let mut seed = [0u8; 8];
        seed.copy_from_slice(r.take(8)?);
        let context = r.string(MAX_CONTEXT_LEN, FormatError::BadContext)?;
        if !is_canonical_json(&context) {
            return Err(FormatError::BadContext);
        }
        let ticks = r.varint()?;
        let count = r.varint()?;
        // Each input takes at least three bytes, so a count the rest of the
        // file cannot hold is refused before anything is allocated for it.
        if count > MAX_INPUTS as u64 || count > (r.left() / 3) as u64 {
            return Err(FormatError::BadInputs);
        }
        let mut inputs = Vec::with_capacity(count as usize);
        let mut tick = 0u64;
        for _ in 0..count {
            tick = tick
                .checked_add(r.varint()?)
                .ok_or(FormatError::BadInputs)?;
            if tick >= ticks {
                return Err(FormatError::BadInputs);
            }
            let code = u16::try_from(r.varint()?).map_err(|_| FormatError::BadNumber)?;
            let value = unzigzag(r.varint()?)?;
            inputs.push(Input { tick, code, value });
        }
        if r.left() != 0 {
            return Err(FormatError::TrailingBytes);
        }
        Ok(Run {
            build: BuildId(build),
            rules,
            seed: u64::from_le_bytes(seed),
            context,
            ticks,
            inputs,
        })
    }

    /// The run's ID: the SHA-256 of its run file, as 64 hex digits. Two
    /// runs share an ID only when they are the same run.
    pub fn id(&self) -> Result<String, FormatError> {
        Ok(run_id(&self.encode()?))
    }
}

/// The ID of the run in `file`, without reading it: the SHA-256 of the
/// bytes, as 64 hex digits. Matches [`Run::id`] for a file that decodes.
pub fn run_id(file: &[u8]) -> String {
    hex(&Sha256::digest(file))
}

/// The build a run file names, read from its fixed place without parsing
/// the rest; `None` when `file` is not a run file of this format.
pub fn build_of(file: &[u8]) -> Option<BuildId> {
    if file.len() < BUILD_OFFSET + 32 || file[..4] != MAGIC || file[4] != FORMAT_VERSION {
        return None;
    }
    let mut build = [0u8; 32];
    build.copy_from_slice(&file[BUILD_OFFSET..BUILD_OFFSET + 32]);
    Some(BuildId(build))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 15)] as char);
    }
    out
}

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn zigzag(v: i32) -> u64 {
    u64::from(((v << 1) ^ (v >> 31)) as u32)
}

fn unzigzag(n: u64) -> Result<i32, FormatError> {
    let n = u32::try_from(n).map_err(|_| FormatError::BadNumber)?;
    Ok(((n >> 1) as i32) ^ -((n & 1) as i32))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        if self.left() < n {
            return Err(FormatError::Truncated);
        }
        let out = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }

    fn byte(&mut self) -> Result<u8, FormatError> {
        Ok(self.take(1)?[0])
    }

    /// A LEB128 varint in its one shortest form.
    fn varint(&mut self) -> Result<u64, FormatError> {
        let mut n = 0u64;
        for i in 0..10 {
            let byte = self.byte()?;
            let bits = u64::from(byte & 0x7f);
            // The tenth byte holds the top bit alone.
            if i == 9 && bits > 1 {
                return Err(FormatError::BadNumber);
            }
            n |= bits << (7 * i);
            if byte & 0x80 == 0 {
                // A last byte of zero (after the first) means extra bytes.
                if i > 0 && byte == 0 {
                    return Err(FormatError::BadNumber);
                }
                return Ok(n);
            }
        }
        Err(FormatError::BadNumber)
    }

    fn string(&mut self, max: usize, too_long: FormatError) -> Result<String, FormatError> {
        let len = self.varint()?;
        if len > max as u64 {
            return Err(too_long);
        }
        let bytes = self.take(len as usize)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| too_long)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Run {
        Run {
            build: BuildId([7; 32]),
            rules: "coin-dash".into(),
            seed: 0x0123_4567_89ab_cdef,
            context: r#"{"laps":3,"mode":"daily"}"#.into(),
            ticks: 1000,
            inputs: vec![
                Input {
                    tick: 0,
                    code: 1,
                    value: 1,
                },
                Input {
                    tick: 0,
                    code: 2,
                    value: -5,
                },
                Input {
                    tick: 999,
                    code: 65535,
                    value: i32::MIN,
                },
            ],
        }
    }

    #[test]
    fn a_run_reads_back_as_written() {
        let run = sample();
        let file = run.encode().unwrap();
        assert_eq!(Run::decode(&file).unwrap(), run);
        assert_eq!(build_of(&file), Some(BuildId([7; 32])));
        assert_eq!(run.id().unwrap(), run_id(&file));
    }

    #[test]
    fn every_cut_short_file_is_refused() {
        let file = sample().encode().unwrap();
        for len in 0..file.len() {
            assert!(Run::decode(&file[..len]).is_err(), "length {len}");
        }
        let mut longer = file.clone();
        longer.push(0);
        assert_eq!(Run::decode(&longer), Err(FormatError::TrailingBytes));
    }

    #[test]
    fn a_number_has_one_encoding() {
        let mut r = Reader {
            bytes: &[0x80, 0x00],
            at: 0,
        };
        assert_eq!(r.varint(), Err(FormatError::BadNumber));
        let mut r = Reader {
            bytes: &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01],
            at: 0,
        };
        assert_eq!(r.varint(), Ok(u64::MAX));
        let mut r = Reader {
            bytes: &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02],
            at: 0,
        };
        assert_eq!(r.varint(), Err(FormatError::BadNumber));
        for v in [0, 1, -1, i32::MAX, i32::MIN] {
            assert_eq!(unzigzag(zigzag(v)), Ok(v));
        }
    }

    #[test]
    fn the_context_must_be_canonical() {
        let mut run = sample();
        run.context = r#"{"mode":"daily","laps":3}"#.into();
        assert_eq!(run.encode(), Err(FormatError::BadContext));
        run.context = r#"{"laps": 3}"#.into();
        assert_eq!(run.encode(), Err(FormatError::BadContext));
        let value: serde_json::Value =
            serde_json::from_str(r#"{"b":[1,{"z":1,"a":2}],"a":null}"#).unwrap();
        assert_eq!(
            canonical_json(&value),
            r#"{"a":null,"b":[1,{"a":2,"z":1}]}"#
        );
    }

    #[test]
    fn inputs_stay_inside_the_run() {
        let mut run = sample();
        run.inputs[2].tick = 1000;
        assert_eq!(run.encode(), Err(FormatError::BadInputs));
        let mut run = sample();
        run.inputs.swap(0, 2);
        assert_eq!(run.encode(), Err(FormatError::BadInputs));
    }

    #[test]
    fn a_huge_count_allocates_nothing() {
        let mut run = sample();
        run.inputs.clear();
        let mut file = run.encode().unwrap();
        file.pop();
        put_varint(&mut file, u64::MAX >> 1);
        assert_eq!(Run::decode(&file), Err(FormatError::BadInputs));
    }

    #[test]
    fn rules_names_are_plain() {
        assert!(valid_rules_name("sonar-dive_2"));
        for bad in ["", "Belfry", "a b", "é", &"x".repeat(65)] {
            assert!(!valid_rules_name(bad), "{bad}");
        }
    }

    #[test]
    fn build_ids_read_as_hex() {
        let id = BuildId([0xab; 32]);
        assert_eq!(BuildId::from_hex(&id.to_hex()), Some(id));
        assert_eq!(BuildId::from_hex("ab"), None);
        assert_eq!(BuildId::from_hex(&"g".repeat(64)), None);
        assert!(BuildId::NONE.is_none());
    }
}
