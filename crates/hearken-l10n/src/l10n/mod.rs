//! Translating a Hearken game's spoken text.
//!
//! A game keeps its source lines in English. Each finished line is
//! translated where it leaves for speech (`Ctx::say` and `say_event` when
//! the game has loaded a catalog). A catalog line is a template:
//! `It is {tod}.` matches "It is 3:15 PM." and hands `3:15 PM` to the
//! translation.
//!
//! A line no template fills is split into sentences, and each sentence is
//! tried on its own. A sentence is translated whole or not at all: it may
//! be split into clauses only when every clause that is not a name
//! translates, and a template whose holes would keep more English words
//! than the template itself supplies is passed over. So a line never comes
//! out mixed word by word; what is not translated stays English. English
//! itself is the identity: no catalog, no lookups.
//!
//! A menu row's position cue ("4 of 8.") is translated apart from the row
//! (see [`POSITION_CUE`]), so the cue never depends on the row matching.
//!
//! Catalogs are gettext PO files. A game ships them under `lang/` (see
//! `docs/l10n.md`). [`crate::load`] finds override files beside the
//! program or in the data folder.

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;

use aho_corasick::{AhoCorasick, MatchKind};
use std::sync::Mutex;

pub mod matcher;
pub mod plural;
pub mod po;

use matcher::{hole_names, parse_source, parse_translation, Part, Seg, Template, TemplateSet};

/// The catalog entry for a menu row's position cue, as `menu.rs` builds it.
pub const POSITION_CUE: &str = "{position} of {count}.";

/// The setting value that follows the operating system's language.
pub const AUTO: &str = "auto";
/// English: the language the game is written in.
pub const ENGLISH: &str = "en";

/// A language the game can speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    /// The catalog's file stem and the settings value: `es`, `pt_BR`.
    pub code: &'static str,
    /// The language's name in English.
    pub name: &'static str,
    /// The language's name in that language.
    pub autonym: &'static str,
}

impl Language {
    /// The settings row's name: the English name with the language's own
    /// name after it, "Spanish (Español)", or one name when they are the same.
    pub fn label(&self) -> String {
        let Language { name, autonym, .. } = *self;
        if name == autonym {
            autonym.to_string()
        } else {
            format!("{name} ({autonym})")
        }
    }
}

/// Languages Hearken knows by code for Settings labels and locale matching.
/// A game still has to ship (or drop in) a catalog for each one it offers;
/// [`crate::load`] discovers extra languages from PO headers through
/// [`add_languages`].
pub const LANGUAGES: &[Language] = &[
    Language {
        code: "en",
        name: "English",
        autonym: "English",
    },
    Language {
        code: "es",
        name: "Spanish",
        autonym: "Español",
    },
    Language {
        code: "fr",
        name: "French",
        autonym: "Français",
    },
    Language {
        code: "de",
        name: "German",
        autonym: "Deutsch",
    },
    Language {
        code: "pt_BR",
        name: "Brazilian Portuguese",
        autonym: "Português do Brasil",
    },
    Language {
        code: "it",
        name: "Italian",
        autonym: "Italiano",
    },
    Language {
        code: "ru",
        name: "Russian",
        autonym: "Русский",
    },
    Language {
        code: "pl",
        name: "Polish",
        autonym: "Polski",
    },
    Language {
        code: "zh_CN",
        name: "Simplified Chinese",
        autonym: "简体中文",
    },
    Language {
        code: "ja",
        name: "Japanese",
        autonym: "日本語",
    },
    Language {
        code: "tr",
        name: "Turkish",
        autonym: "Türkçe",
    },
    Language {
        code: "vi",
        name: "Vietnamese",
        autonym: "Tiếng Việt",
    },
];

/// Languages a translator added as a file alone, beyond [`LANGUAGES`].
static ADDED: std::sync::OnceLock<Vec<Language>> = std::sync::OnceLock::new();

/// Offer languages that came as a catalog file with no entry in
/// [`LANGUAGES`]: `(code, name, autonym)`, read from the files' headers.
/// The game calls this once, before the language is first resolved; a
/// later call changes nothing. Codes already known are skipped.
pub fn add_languages(found: impl IntoIterator<Item = (String, String, String)>) {
    let mut added: Vec<Language> = Vec::new();
    for (code, name, autonym) in found {
        if language(&code).is_some() || added.iter().any(|lang| lang.code == code) {
            continue;
        }
        // Leaked once per language at launch: the settings row and the
        // resolved code hand these out as `&'static str` like the built-ins.
        added.push(Language {
            code: Box::leak(code.into_boxed_str()),
            name: Box::leak(name.into_boxed_str()),
            autonym: Box::leak(autonym.into_boxed_str()),
        });
    }
    let _ = ADDED.set(added);
}

/// Every language the game has: [`LANGUAGES`], then any added from a file.
pub fn languages() -> impl Iterator<Item = &'static Language> {
    LANGUAGES.iter().chain(ADDED.get().into_iter().flatten())
}

/// The language a code names, if the game has it.
pub fn language(code: &str) -> Option<&'static Language> {
    languages().find(|lang| lang.code == code)
}

/// The catalog for an operating-system locale tag ("es-MX", "pt_BR.UTF-8",
/// "zh-Hans-CN"), or `None` when the game has no catalog for it.
///
/// Any Portuguese gets the Brazilian catalog and any Chinese the simplified
/// one: closer than English for the player, and a voice set to either
/// reads them.
pub fn match_locale(tag: &str) -> Option<&'static str> {
    let tag = tag.split(['.', '@']).next().unwrap_or(tag);
    let base = tag
        .split(['-', '_'])
        .next()
        .unwrap_or(tag)
        .to_ascii_lowercase();
    let code = match base.as_str() {
        "pt" => "pt_BR",
        "zh" => "zh_CN",
        other => other,
    };
    // An added catalog may be regional ("pt_PT"); the whole tag picks it
    // before the base language does.
    let whole = tag.replace('-', "_");
    language(&whole)
        .or_else(|| language(code))
        .map(|lang| lang.code)
}

/// The catalog a language setting asks for: `auto` follows `system` (the
/// operating system's locale tag), an unknown value falls back to English.
pub fn resolve(setting: &str, system: Option<&str>) -> &'static str {
    if setting == AUTO || setting.is_empty() {
        return system.and_then(match_locale).unwrap_or(ENGLISH);
    }
    language(setting).map_or(ENGLISH, |lang| lang.code)
}

/// Placeholder words whose value is a place or a proper name: never sent
/// through the catalog, so "Mobile" stays a city in Alabama and never
/// becomes a phone. A hole is a name when one of its words (split at `_`,
/// trailing digits dropped) starts with one of these: `city2`, `the_city`,
/// `destination`, `driver_name`. Matched by word, not anywhere in the name,
/// so `replace` is not a place and `open_road_target` is not a road.
const PROPER_NAME_HOLES: &[&str] = &[
    "city", "town", "village", "county", "state", "road", "street", "highway", "route", "exit",
    "where", "dest", "origin", "home", "station", "place", "artist", "song", "handle", "driver",
    "brand", "chain", "studio", "company", "landmark", "rcity",
];

/// A last word that makes a hole a value about a place, not its name:
/// `place_callouts`, `exit_speed`, `road_class`.
const NOT_NAME_HEADS: &[&str] = &[
    "callouts",
    "direction",
    "class",
    "step",
    "summary",
    "done",
    "next",
    "word",
    "speed",
    "hint",
    "note",
    "clause",
    "target",
    "level",
    "mph",
    "s",
    "min",
];

/// Holes that read like a place but hold a condition ("charged").
const NOT_NAME_HOLES: &[&str] = &["air_state", "brake_state"];

/// The help's control names (`{{engine}}` in the source, `{control_engine}`
/// in the catalog): a key or button name, kept as the keyboard reads it.
const CONTROL_HOLE_PREFIXES: &[&str] = &["control_", "key_", "pad_"];

fn is_proper_name_hole(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if CONTROL_HOLE_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return true;
    }
    if NOT_NAME_HOLES.contains(&name.as_str()) {
        return false;
    }
    let words: Vec<&str> = name
        .split('_')
        .map(|word| word.trim_end_matches(|c: char| c.is_ascii_digit()))
        .filter(|word| !word.is_empty())
        .collect();
    if words
        .last()
        .is_none_or(|last| NOT_NAME_HEADS.contains(last))
    {
        return false;
    }
    words
        .iter()
        .any(|word| PROPER_NAME_HOLES.iter().any(|part| word.starts_with(part)))
}

/// How deep a line is split and its captures translated in turn.
// A drive-start line nests that deep: the line, its objective, the trip
// progress inside it, and the distance inside that.
const MAX_DEPTH: usize = 6;

/// How many lighter templates are tried for one that keeps no English
/// once a template that keeps some has matched.
/// Units a hole named after them counts: "{seconds} seconds".
const COUNTED_UNITS: &[&str] = &[
    "seconds",
    "minutes",
    "hours",
    "days",
    "weeks",
    "metres",
    "meters",
    "miles",
    "kilometres",
    "kilometers",
    "feet",
    "points",
];

/// Words an amount before a unit may hold: "1 minute 20 seconds",
/// "half a" metre.
const AMOUNT_WORDS: &[&str] = &[
    "minute", "minutes", "hour", "hours", "and", "a", "half", "an",
];

/// Holes that always hold a number.
const COUNT_HOLES: &[&str] = &["n", "count", "number", "num", "total", "index"];

/// The most sentences a catalog entry holds whole.
const MAX_SENTENCE_GROUP: usize = 4;

const MAX_LOOSER_TRIES: usize = 8;

/// A rendered template match: its literal weight, the translated line, and
/// the English pieces its holes kept as they were.
struct Hit {
    weight: usize,
    out: String,
    kept: Vec<String>,
    /// A hole took a clause (", "): a looser fit than one whose holes
    /// hold single values.
    loose: bool,
    /// The lowercase hole values and literal pieces of the template: a
    /// clause none of them holds whole was cut between a hole and words.
    pieces: Vec<String>,
}

impl Hit {
    fn exact(out: String) -> Self {
        Self {
            weight: usize::MAX,
            out,
            kept: Vec::new(),
            loose: false,
            pieces: Vec::new(),
        }
    }

    /// A match that keeps no English beats one that does; then the heavier.
    fn rank(&self) -> (bool, usize) {
        (self.kept.is_empty(), self.weight)
    }

    /// As `rank`, but a match whose holes took no clause beats one whose
    /// hole did.
    fn rank_tight(&self) -> (bool, bool, usize) {
        (self.kept.is_empty(), !self.loose, self.weight)
    }
}
/// Lines whose translation is remembered (the drive repeats itself).
const CACHE_LIMIT: usize = 8192;

/// One language's compiled catalog.
pub struct Catalog {
    language: String,
    exact: HashMap<String, String>,
    /// The multi-sentence exact entries, found inside longer lines.
    embedded: Option<AhoCorasick>,
    embedded_keys: Vec<String>,
    templates: TemplateSet,
    /// The translation of [`POSITION_CUE`], when the catalog has one.
    position_cue: Option<String>,
    /// Entries that were rejected while compiling, for the checker.
    pub problems: Vec<String>,
}

impl Catalog {
    /// Compile a PO file's text. Untranslated (empty) and fuzzy entries are
    /// left out; a broken entry is recorded in `problems` and skipped, so
    /// one typo never costs the whole language.
    pub fn from_po(language: &str, text: &str) -> Result<Self, po::PoError> {
        let entries = po::parse(text)?;
        let mut exact = HashMap::new();
        let mut templates = Vec::new();
        let mut problems = Vec::new();
        let mut position_cue = None;
        let mut tails = Vec::new();
        for entry in entries {
            // An entry with a msgctxt tells a translator apart a second use
            // of the same English (the controller's Back button beside the
            // Back row); the game matches finished lines, so the plain entry
            // is the one it speaks.
            if entry.msgid.is_empty()
                || entry.msgstr.is_empty()
                || entry.is_fuzzy()
                || entry.msgctxt.is_some()
            {
                continue;
            }
            if entry.msgid == POSITION_CUE {
                if entry.msgstr.contains("{position}") && entry.msgstr.contains("{count}") {
                    position_cue = Some(entry.msgstr.clone());
                } else {
                    problems.push(format!(
                        "line {}: the position cue needs both {{position}} and {{count}}",
                        entry.line
                    ));
                }
                continue;
            }
            let segs = parse_source(&entry.msgid);
            let names = hole_names(&segs);
            if names.is_empty() {
                exact.insert(entry.msgid, entry.msgstr);
                continue;
            }
            // A template is matched against a trimmed piece, so a piece of a
            // longer line (" Mission brief: {terminal_text}") is compiled
            // without the spaces that joined it.
            let segs = parse_source(entry.msgid.trim());
            let translation = parse_translation(entry.msgstr.trim());
            let unknown: Vec<&str> = translation
                .iter()
                .filter_map(|part| match part {
                    Part::Hole(name) | Part::Plural(name, _) => Some(name.as_str()),
                    Part::Lit(_) => None,
                })
                .filter(|name| !names.contains(name))
                .collect();
            if !unknown.is_empty() {
                problems.push(format!(
                    "line {}: the translation names {{{}}}, which the English line does not have",
                    entry.line,
                    unknown.join("}, {")
                ));
                continue;
            }
            // "{flavor} The match starts on a short practice run." is
            // also the sentence after the lead-in: a line split into
            // sentences reaches it without its hole.
            if let ([Seg::Hole(a), Seg::Lit(tail)], [Part::Hole(b), Part::Lit(out)]) =
                (segs.as_slice(), translation.as_slice())
            {
                if a == b
                    && tail.starts_with(' ')
                    && out.starts_with(' ')
                    && tail.trim_start().starts_with(char::is_uppercase)
                {
                    tails.push((tail.trim().to_string(), out.trim().to_string()));
                }
            }
            match Template::new(segs, translation) {
                Ok(template) => templates.push(template),
                // A joiner like "{a}, {b}" is not an error; the splitter
                // already handles what it would.
                Err(_) => continue,
            }
        }
        for (key, value) in tails {
            exact.entry(key).or_insert(value);
        }
        // A fragment like "Notice: " is also found by its bare words,
        // the way a piece reaches it after trimming.
        let trimmed: Vec<(String, String)> = exact
            .iter()
            .filter(|(key, _)| key.trim() != key.as_str() && !key.trim().is_empty())
            .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
            .collect();
        for (key, value) in trimmed {
            exact.entry(key).or_insert(value);
        }
        let mut embedded_keys: Vec<String> = exact
            .keys()
            .filter(|key| has_sentence_break(key))
            .cloned()
            .collect();
        embedded_keys.sort();
        let embedded = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&embedded_keys)
            .map_err(|err| log::error!("translations could not be indexed: {err}"))
            .ok();
        Ok(Self {
            language: language.to_string(),
            embedded,
            embedded_keys,
            exact,
            templates: TemplateSet::new(templates),
            position_cue,
            problems,
        })
    }

    /// A short word or phrase the catalog has as an entry of its own
    /// ("spades", "jack"): safe to fill a generic template's hole.
    fn knows_word(&self, value: &str) -> bool {
        let value = value.trim();
        if value.is_empty() || value.contains(['.', ':', ';', ',']) {
            return false;
        }
        // A word kept as it is ("Enter", a key name) is no word of the
        // language: "Enter to start" is not "{from} to {to}".
        let changes = |key: &str| self.exact.get(key).is_some_and(|out| out != key);
        if changes(value) {
            return true;
        }
        let mut chars = value.chars();
        chars.next().is_some_and(|first| {
            let flipped: String = if first.is_uppercase() {
                first.to_lowercase().chain(chars).collect()
            } else {
                first.to_uppercase().chain(chars).collect()
            };
            changes(&flipped)
        })
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// "4 of 8." in the catalog's language, or as English has it.
    fn position_cue(&self, position: &str, count: &str) -> String {
        let Some(cue) = &self.position_cue else {
            return format!("{position} of {count}.");
        };
        let number = |raw: &str| {
            plural::localize_number(&self.language, raw).unwrap_or_else(|| raw.to_string())
        };
        cue.replace("{position}", &number(position))
            .replace("{count}", &number(count))
    }

    /// How many lines the catalog translates.
    pub fn len(&self) -> usize {
        self.exact.len() + self.templates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Translates finished English lines into the player's language.
pub struct Translator {
    catalog: Option<Catalog>,
    cache: Mutex<HashMap<String, String>>,
    misses: Option<Mutex<MissLog>>,
}

/// Where untranslated lines are written, once each, for translators.
struct MissLog {
    path: PathBuf,
    seen: HashSet<String>,
}

impl Default for Translator {
    fn default() -> Self {
        Self::english()
    }
}

impl Translator {
    /// English: every line passes through untouched.
    pub fn english() -> Self {
        Self {
            catalog: None,
            cache: Mutex::new(HashMap::new()),
            misses: None,
        }
    }

    pub fn new(catalog: Catalog) -> Self {
        Self {
            catalog: Some(catalog),
            cache: Mutex::new(HashMap::new()),
            misses: None,
        }
    }

    /// Append every line that is not fully translated to `path`, once per
    /// session: the list a translator works from.
    pub fn log_misses_to(mut self, path: PathBuf) -> Self {
        self.misses = Some(Mutex::new(MissLog {
            path,
            seen: HashSet::new(),
        }));
        self
    }

    /// The catalog's language code, `en` for English.
    pub fn language(&self) -> &str {
        self.catalog.as_ref().map_or(ENGLISH, Catalog::language)
    }

    pub fn is_english(&self) -> bool {
        self.catalog.is_none()
    }

    /// The line in the player's language, as much of it as the catalog
    /// covers.
    pub fn translate<'a>(&self, line: &'a str) -> Cow<'a, str> {
        let Some(catalog) = &self.catalog else {
            return Cow::Borrowed(line);
        };
        if let Some(hit) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(line)
        {
            return Cow::Owned(hit.clone());
        }
        let mut missed = Vec::new();
        let mut memo = Memo::new();
        let whole_line = |missed: &mut Vec<String>, memo: &mut Memo| {
            Pass {
                catalog,
                missed,
                memo,
            }
            .line(line, 0)
            .map(|out| {
                if starts_lowercase(line) {
                    out
                } else {
                    capitalize_first_letter(&out)
                }
            })
        };
        let cue = split_position_cue(line);
        // "Discoveries tonight: 0 of 8." reads like a menu row and its
        // position, but the catalog has it whole: that wins.
        let whole = cue.and_then(|_| {
            let mut whole_missed = Vec::new();
            let out = whole_line(&mut whole_missed, &mut memo)?;
            whole_missed.is_empty().then_some(out)
        });
        let out = match (whole, cue) {
            (Some(out), _) => out,
            (None, Some((row, position, count))) => {
                let row = Pass {
                    catalog,
                    missed: &mut missed,
                    memo: &mut memo,
                }
                .line(row, 0)
                .unwrap_or_else(|| row.to_string());
                let cue = catalog.position_cue(position, count);
                let sep = if ends_full_width(&row) { "" } else { " " };
                format!("{row}{sep}{cue}")
            }
            (None, None) => whole_line(&mut missed, &mut memo).unwrap_or_else(|| line.to_string()),
        };
        let out = contract_articles(&catalog.language, out);
        if let Some(log) = &self.misses {
            log.lock()
                .unwrap_or_else(|e| e.into_inner())
                .record(&missed);
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(line.to_string(), out.clone());
        Cow::Owned(out)
    }
}

impl MissLog {
    fn record(&mut self, missed: &[String]) {
        let fresh: Vec<&String> = missed
            .iter()
            .filter(|line| self.seen.insert((*line).clone()))
            .collect();
        if fresh.is_empty() {
            return;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path);
        match file {
            Ok(mut file) => {
                for line in fresh {
                    let _ = writeln!(file, "{}", line.replace('\n', "\\n"));
                }
            }
            Err(err) => log::warn!(
                "Could not write untranslated lines to {}: {err}",
                self.path.display()
            ),
        }
    }
}

/// One translation of one line: the catalog, and the pieces nothing
/// translated.
struct Pass<'c, 'm> {
    catalog: &'c Catalog,
    missed: &'m mut Vec<String>,
    memo: &'m mut Memo,
}

/// Every piece already worked out in one line's translation, by its text,
/// depth and whether it may split, with the misses it recorded. The
/// matcher tries the same capture under many templates and clause splits,
/// so a long line (the driving keyboard help) asked for the same pieces
/// millions of times: minutes on the game thread before this.
type Memo = HashMap<(String, usize, bool), (Option<String>, Vec<String>)>;

impl Pass<'_, '_> {
    /// A whole line or one of its pieces. `None` when nothing in it was
    /// translated; the piece is then recorded as missed.
    fn line(&mut self, text: &str, depth: usize) -> Option<String> {
        let out = self.piece(text, depth, true);
        if out.is_none() && has_words(text) {
            self.missed.push(text.trim().to_string());
        }
        out
    }

    fn piece(&mut self, text: &str, depth: usize, split: bool) -> Option<String> {
        let key = (text.to_string(), depth, split);
        if let Some((out, missed)) = self.memo.get(&key) {
            self.missed.extend(missed.iter().cloned());
            return out.clone();
        }
        let start = self.missed.len();
        let out = self.piece_uncached(text, depth, split);
        let missed = self.missed.get(start..).unwrap_or_default().to_vec();
        self.memo.insert(key, (out.clone(), missed));
        out
    }

    fn piece_uncached(&mut self, text: &str, depth: usize, split: bool) -> Option<String> {
        let core = text.trim();
        if core.is_empty() {
            return None;
        }
        let lead = &text[..text.len() - text.trim_start().len()];
        let trail = &text[text.trim_end().len()..];
        let wrap = |inner: String| format!("{lead}{inner}{trail}");
        if let Some(hit) = self.catalog.exact.get(core) {
            return Some(wrap(hit.clone()));
        }
        // A language's Settings label, "Spanish (Español)": in another
        // language it is the language's own name, which needs no catalog.
        if let Some(autonym) = languages()
            .find(|lang| lang.name != lang.autonym && lang.label() == core.trim_end_matches('.'))
            .map(|lang| lang.autonym)
        {
            let out = if core.ends_with('.') {
                self.with_full_stop(autonym.to_string())
            } else {
                autonym.to_string()
            };
            return Some(wrap(out));
        }
        // "Status ready to continue" inside "Status: {summary}" is the
        // catalog's whole sentence without its full stop. A whole sentence
        // beats any template a loose fragment of it fills ("{driven} driving").
        if !core.ends_with(['.', '!', '?', ':', ';', ',']) {
            if let Some(hit) = self.catalog.exact.get(&format!("{core}.")) {
                let out = hit
                    .strip_suffix(['.', '\u{3002}'])
                    .map(str::to_string)
                    .unwrap_or_else(|| hit.clone());
                return Some(wrap(out));
            }
        }
        // "In the right lane." is the catalog's exact "In the right lane"
        // with a full stop: that beats any template the stopped line also
        // fills ("{at} the {noun}.").
        if let Some(hit) = core
            .strip_suffix('.')
            .filter(|bare| !bare.ends_with(['.', ':', ',', ';']))
            .and_then(|bare| self.catalog.exact.get(bare))
        {
            let out = self.with_full_stop(hit.clone());
            return Some(wrap(out));
        }
        // Sentences that each translate cleanly beat a template for the
        // whole line: "4 of 4 lanterns gathered. The song is whole." is two
        // sentences the catalog has, not "{gathered} of {lanterns}.{joins}
        // The song is whole." with "4 lanterns gathered" as the count.
        if split && depth < MAX_DEPTH && has_sentence_break(core) {
            let start = self.missed.len();
            if let Some(out) = self.split_by(core, depth, false) {
                if self.missed.len() == start {
                    return Some(wrap(out));
                }
            }
            self.missed.truncate(start);
        }
        let direct = self.lookup(core, depth);
        // "Score board." is the catalog's "Score board" with the full
        // stop a menu adds after an item.
        // A lead-in like "Notice:" keeps its colon the same way.
        // The heavier of the two wins: "Deliveries: 1, 0 percent on time."
        // is the catalog's "Deliveries: {deliveries}, {f} percent on time"
        // with a stop, not a looser template that ends in one.
        let stopped = core
            .chars()
            .next_back()
            .filter(|c| matches!(c, '.' | ':' | ',' | ';'))
            .and_then(|mark| {
                let bare = &core[..core.len() - mark.len_utf8()];
                if bare.ends_with(['.', ':', ',', ';']) {
                    return None;
                }
                let mut hit = self.lookup(bare, depth)?;
                if mark == '.' {
                    hit.out = self.with_full_stop(hit.out);
                } else {
                    hit.out.push(mark);
                }
                Some(hit)
            });
        let best = match (direct, stopped) {
            (Some(direct), Some(stopped)) if stopped.rank_tight() > direct.rank_tight() => {
                Some(stopped)
            }
            (Some(direct), _) => Some(direct),
            (None, stopped) => stopped,
        };
        // A template loses to the clauses translated each when it cuts a
        // clause the catalog has whole between a hole and its own words:
        // "Thin ice, far left." is "Thin ice" and the catalog's "far left",
        // not "{time} left." with "Thin ice, far" as the time.
        if let Some(hit) = best.as_ref().filter(|hit| hit.kept.is_empty()) {
            if self.cuts_a_whole_clause(core, &hit.pieces) {
                let start = self.missed.len();
                if let Some(out) = self.split_by(core, depth, true) {
                    if self.missed.len() == start {
                        return Some(wrap(out));
                    }
                }
                self.missed.truncate(start);
            }
        }
        match best {
            Some(hit) if hit.kept.is_empty() => Some(wrap(hit.out)),
            // A match that keeps English pieces loses to splitting the
            // line, when every piece of that translates.
            Some(hit) => {
                let start = self.missed.len();
                let split_out = self.split_piece(core, depth, split);
                if split_out.is_some() && self.missed.len() == start {
                    return split_out.map(wrap);
                }
                self.missed.truncate(start);
                self.missed.extend(hit.kept);
                Some(wrap(hit.out))
            }
            None => self.split_piece(core, depth, split).map(wrap),
        }
    }

    /// Whether a clause of `core` is a catalog entry whole that no hole
    /// value or literal piece of the template holds whole.
    fn cuts_a_whole_clause(&self, core: &str, pieces: &[String]) -> bool {
        let clauses = split_clauses(core);
        clauses.len() >= 2
            && clauses.iter().any(|(clause, _)| {
                let clause = clause.trim();
                let bare = clause.trim_end_matches(['.', '!', '?']);
                if bare.is_empty() || !has_words(bare) {
                    return false;
                }
                let known = [clause.to_string(), bare.to_string(), format!("{bare}.")]
                    .iter()
                    .any(|form| {
                        self.catalog.exact.contains_key(form.as_str())
                            || self
                                .catalog
                                .exact
                                .contains_key(capitalize_first_letter(form).as_str())
                    });
                let lower = bare.to_lowercase();
                known && !pieces.iter().any(|piece| piece.contains(&lower))
            })
    }

    /// A piece no exact entry or template has whole: its sentences,
    /// clauses or lead-in, translated each.
    fn split_piece(&mut self, core: &str, depth: usize, split: bool) -> Option<String> {
        if depth >= MAX_DEPTH {
            return None;
        }
        // "Status ready to continue" after "Status: " is the catalog's
        // sentence without its full stop.
        if !core.ends_with(['.', '!', '?', ':', ';', ',']) {
            if let Some(Hit { out, .. }) = self.lookup(&format!("{core}."), depth) {
                let out = out
                    .strip_suffix(['.', '\u{3002}'])
                    .map(str::to_string)
                    .unwrap_or(out);
                return Some(out);
            }
        }
        if split {
            // Sentences that each translate beat a sentence the catalog has
            // found inside the line: "4 of 4 lanterns gathered. The song is
            // whole." is two sentences, not "The song is whole." with words
            // around it.
            let start = self.missed.len();
            if let Some(out) = self.split_by(core, depth, false) {
                if self.missed.len() == start {
                    return Some(out);
                }
            }
            self.missed.truncate(start);
            if let Some(out) = self.embedded(core, depth) {
                return Some(out);
            }
            if let Some(out) = self.split_by(core, depth, false) {
                return Some(out);
            }
        }
        if let Some(out) = self.split_by(core, depth, true) {
            return Some(out);
        }
        self.lead_in(core, depth)
    }

    /// The piece split into sentences, or into clauses, each translated.
    /// Clauses all translate or the piece stays English.
    fn split_by(&mut self, core: &str, depth: usize, clauses: bool) -> Option<String> {
        let pieces = if clauses {
            split_clauses(core)
        } else {
            split_sentences(core)
        };
        if pieces.len() < 2 {
            return None;
        }
        if !clauses {
            return self.split_sentence_groups(core, &pieces, depth);
        }
        let cjk = matches!(
            self.catalog.language.split('_').next().unwrap_or_default(),
            "zh" | "ja"
        );
        {
            let mut any = false;
            let mut whole = true;
            let mut out = String::new();
            let mut missed = Vec::new();
            for (piece, sep) in &pieces {
                let translated = {
                    let mut inner = Pass {
                        catalog: self.catalog,
                        missed: &mut missed,
                        memo: &mut *self.memo,
                    };
                    // A sentence is a line of its own; only clauses go deeper.
                    let deeper = if clauses { depth + 1 } else { depth };
                    let out = inner.line(piece, deeper);
                    // A sentence opens with a capital in the translation as
                    // in English: "451 miles remaining" is "Quedan 451 millas".
                    if clauses || starts_lowercase(piece) {
                        out
                    } else {
                        out.map(|out| capitalize_first_letter(&out))
                    }
                };
                any |= translated.is_some();
                whole &= translated.is_some() || !has_words(piece) || is_name_like(piece);
                let piece_out = translated.unwrap_or_else(|| (*piece).to_string());
                let sep = if sep.trim().is_empty() && ends_full_width(&piece_out) {
                    ""
                } else if clauses && cjk {
                    // Chinese and Japanese mark a clause with their own
                    // comma and colon: "カエル、コオロギ", not "カエル, コオロギ".
                    match *sep {
                        ", " => "\u{3001}",
                        ": " => "\u{ff1a}",
                        "; " => "\u{ff1b}",
                        other => other,
                    }
                } else {
                    sep
                };
                out.push_str(&piece_out);
                out.push_str(sep);
            }
            // Clauses are words of one sentence: translated all together,
            // or the sentence stays English rather than come out mixed.
            if any && (whole || !clauses) {
                self.missed.extend(missed);
                return Some(out);
            }
        }
        None
    }

    /// A run of sentences the catalog has whole, with or without the
    /// full stop the line puts after it: "... over the garden" said as
    /// "... over the garden. Up says ...".
    fn whole_group(&mut self, group: &str, depth: usize) -> Option<String> {
        if let Some(hit) = self.lookup(group, depth).filter(|hit| hit.kept.is_empty()) {
            return Some(hit.out);
        }
        let bare = group.strip_suffix('.')?;
        let hit = self.lookup(bare, depth).filter(|hit| hit.kept.is_empty())?;
        Some(self.with_full_stop(hit.out))
    }

    /// Sentences, each translated, where a run of sentences the catalog
    /// has whole is kept together: "Next: Belfry. 1 of 3. Escape for the
    /// list." is the catalog's "Next: {scene}. {number} of {count}." and
    /// then "Escape for the list.", not "1 of 3." left English.
    fn split_sentence_groups(
        &mut self,
        core: &str,
        pieces: &[(&str, &str)],
        depth: usize,
    ) -> Option<String> {
        let n = pieces.len();
        // Byte offset where each piece starts, and where it ends before its gap.
        let base = core.as_ptr() as usize;
        let start_of = |i: usize| pieces[i].0.as_ptr() as usize - base;
        let end_of = |i: usize| start_of(i) + pieces[i].0.len();
        // best[i]: (pieces left English, groups) for the first i pieces,
        // and cut[i] the group that ends them: where it starts, its
        // translation and the pieces it missed.
        let mut best: Vec<Option<(usize, usize)>> = vec![None; n + 1];
        let mut cut: HashMap<usize, (usize, Option<String>, Vec<String>)> = HashMap::new();
        best[0] = Some((0, 0));
        for i in 1..=n {
            for j in (0..i).rev() {
                if i - j > MAX_SENTENCE_GROUP || (j == 0 && i == n) {
                    continue;
                }
                let Some((left, groups)) = best[j] else {
                    continue;
                };
                let group = &core[start_of(j)..end_of(i - 1)];
                let mut missed = Vec::new();
                let out = if i - j == 1 {
                    let mut inner = Pass {
                        catalog: self.catalog,
                        missed: &mut missed,
                        memo: &mut *self.memo,
                    };
                    let out = inner.line(group, depth);
                    if starts_lowercase(group) {
                        out
                    } else {
                        out.map(|out| capitalize_first_letter(&out))
                    }
                } else {
                    self.whole_group(group, depth)
                };
                if i - j > 1 && out.is_none() {
                    continue;
                }
                let english = out.is_none() && has_words(group) && !is_name_like(group);
                // Fewest pieces left English, then the most groups: a run
                // is kept whole only when its sentences do not translate
                // one by one.
                let score = (left + english as usize, groups + 1);
                if best[i].is_none_or(|b| (score.0, Reverse(score.1)) < (b.0, Reverse(b.1))) {
                    best[i] = Some(score);
                    cut.insert(i, (j, out, missed));
                }
            }
        }
        let mut cuts = Vec::new();
        let mut i = n;
        while i > 0 {
            let (j, out, missed) = cut.remove(&i)?;
            cuts.push((j, i, out, missed));
            i = j;
        }
        cuts.reverse();
        if cuts.iter().all(|c| c.2.is_none()) {
            return None;
        }
        let mut out = String::new();
        for (j, i, translated, missed) in cuts {
            self.missed.extend(missed);
            let piece_out =
                translated.unwrap_or_else(|| core[start_of(j)..end_of(i - 1)].to_string());
            let sep = pieces[i - 1].1;
            let sep = if sep.trim().is_empty() && ends_full_width(&piece_out) {
                ""
            } else {
                sep
            };
            out.push_str(&piece_out);
            out.push_str(sep);
        }
        Some(out)
    }

    /// "Level 2 of 7: 19 crates of supplies from ... in Riverton,
    /// Oregon to ..." is a lead-in the catalog has ("Level {index} of
    /// {total}: ") before a sentence it has. The clause split breaks that
    /// sentence at the commas inside its place names, so the lead-in and
    /// the rest are each tried whole: both translate, or neither does.
    fn lead_in(&mut self, core: &str, depth: usize) -> Option<String> {
        for (at, _) in core.match_indices(": ") {
            let (head, rest) = (&core[..at + 2], &core[at + 2..]);
            if rest.trim().is_empty() || !has_words(rest) {
                continue;
            }
            let mut missed = Vec::new();
            let mut inner = Pass {
                catalog: self.catalog,
                missed: &mut missed,
                memo: &mut *self.memo,
            };
            let Some(head_out) = inner.piece(head, depth + 1, false) else {
                continue;
            };
            let Some(rest_out) = inner.piece(rest, depth + 1, has_sentence_break(rest)) else {
                continue;
            };
            self.missed.extend(missed);
            let rest_out = if starts_lowercase(rest) {
                rest_out
            } else {
                capitalize_first_letter(&rest_out)
            };
            let sep = if head_out.ends_with(char::is_whitespace) || ends_full_width(&head_out) {
                ""
            } else {
                " "
            };
            return Some(format!("{head_out}{sep}{rest_out}"));
        }
        None
    }

    /// A line with a whole multi-sentence catalog entry inside it, such as a
    /// notice after its lead-in: the entry is translated in place and
    /// the text around it piece by piece. Splitting into sentences first
    /// would break the entry into sentences the catalog does not have.
    fn embedded(&mut self, core: &str, depth: usize) -> Option<String> {
        let finder = self.catalog.embedded.as_ref()?;
        let mut out = String::new();
        let mut last = 0;
        let mut any = false;
        for found in finder.find_iter(core) {
            let (start, end) = (found.start(), found.end());
            let opens = start == 0 || core[..start].ends_with(char::is_whitespace);
            let closes = end == core.len()
                || core[end..].starts_with(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
            if !opens || !closes {
                continue;
            }
            let gap = &core[last..start];
            if !gap.trim().is_empty() {
                let translated = self.line(gap, depth);
                out.push_str(translated.as_deref().unwrap_or(gap));
            } else {
                out.push_str(gap);
            }
            let key = &self.catalog.embedded_keys[found.pattern().as_usize()];
            out.push_str(&self.catalog.exact[key]);
            last = end;
            any = true;
        }
        if !any {
            return None;
        }
        let gap = &core[last..];
        if !gap.trim().is_empty() {
            let translated = self.line(gap, depth);
            out.push_str(translated.as_deref().unwrap_or(gap));
        } else {
            out.push_str(gap);
        }
        Some(out)
    }

    /// A translated piece with its sentence ended the way the language ends
    /// one, unless the translation already did.
    /// A whole-piece match: exact, then a template, then the same with the
    /// first letter's case flipped, since a fragment that opens a sentence
    /// ("Confirmed spare seat.") is the catalog's lowercase fragment.
    /// With the match's weight: a template's literal letters, or
    /// `usize::MAX` for an exact entry.
    fn lookup(&mut self, core: &str, depth: usize) -> Option<Hit> {
        if let Some(hit) = self.catalog.exact.get(core) {
            return Some(Hit::exact(hit.clone()));
        }
        let mut chars = core.chars();
        let first = chars.next()?;
        let upper = first.is_uppercase();
        let flipped: Option<String> = if upper {
            Some(first.to_lowercase().chain(chars).collect())
        } else if first.is_lowercase() {
            Some(first.to_uppercase().chain(chars).collect())
        } else {
            None
        };
        // "None left to turn" is the catalog's whole "none left to turn"
        // capitalised to open a sentence: that beats "{n} left to turn".
        if let Some(hit) = flipped.as_ref().and_then(|f| self.catalog.exact.get(f)) {
            return Some(Hit::exact(if upper {
                capitalize(hit)
            } else {
                hit.clone()
            }));
        }
        let direct = self.template(core, depth);
        // A fragment the game capitalised to open a sentence ("Divided, two
        // lanes your side") fills its lowercase template, and that template
        // wins over a looser one that would swallow "Divided" into a hole.
        if let Some(lower) = flipped.as_deref().filter(|_| upper) {
            if !self.catalog.exact.contains_key(lower) {
                if let Some(hit) = self.template(lower, depth) {
                    if direct.as_ref().is_none_or(|best| hit.rank() > best.rank()) {
                        return Some(Hit {
                            out: capitalize(&hit.out),
                            ..hit
                        });
                    }
                }
            }
        }
        if direct.as_ref().is_some_and(|hit| hit.kept.is_empty()) {
            return direct;
        }
        // "bell 4, middle low" in "Place 2: bell 4, middle low." is the
        // catalog's "Bell {bell}, {name}" lowercased to go mid-sentence.
        if let Some(upper_form) = flipped.as_deref().filter(|_| !upper) {
            if !self.catalog.exact.contains_key(upper_form) {
                if let Some(hit) = self.template(upper_form, depth) {
                    if direct.as_ref().is_none_or(|best| hit.rank() > best.rank()) {
                        // German capitalizes its nouns wherever they stand.
                        let german = self.catalog.language.split('_').next() == Some("de");
                        let out = if german {
                            hit.out
                        } else {
                            lowercase_first_letter(&hit.out)
                        };
                        return Some(Hit { out, ..hit });
                    }
                }
            }
        }
        direct
    }

    fn with_full_stop(&self, mut out: String) -> String {
        if !out.trim_end().ends_with(['.', '!', '?', '。', '！', '？']) {
            let base = self.catalog.language.split('_').next().unwrap_or_default();
            out.push(if matches!(base, "zh" | "ja") {
                '。'
            } else {
                '.'
            });
        }
        out
    }

    /// The best template the line fills, rendered with its captures, and
    /// that template's literal weight.
    /// A name with a catalog frame of its own whose every hole is a name
    /// ("station Central Depot in Midland"): the frame translated,
    /// the names kept.
    fn place_phrase(&self, raw: &str) -> Option<String> {
        if !raw.contains(' ') {
            return None;
        }
        let (template, captures) = self.catalog.templates.best_match(raw, |template, caps| {
            // "{place_name} in {city}" has few letters of its own, but here
            // every hole is a name, so it is a frame around names.
            template.segs.iter().any(|seg| {
                    matches!(seg, Seg::Lit(text) if text.ends_with(" in ") || text.contains(" near "))
                })
                && caps.iter().all(|(name, value)| {
                    is_proper_name_hole(name) && reads_as_name(value) && !value.contains(": ")
                })
        })?;
        let mut out = String::new();
        for part in &template.translation {
            match part {
                Part::Lit(text) => out.push_str(text),
                Part::Hole(name) => {
                    let raw = captures.get(name.as_str()).copied().unwrap_or_default();
                    let bare = name
                        .starts_with("the_")
                        .then(|| raw.strip_prefix("the ").or(raw.strip_prefix("The ")))
                        .flatten();
                    out.push_str(bare.unwrap_or(raw));
                }
                Part::Plural(..) => return None,
            }
        }
        Some(out)
    }

    /// The best template the line fills, rendered: its literal weight,
    /// the line, and the English pieces its holes kept. A template that
    /// keeps no English beats a heavier one that does: "8 of hearts" is
    /// "{rank_words} of {name}", not "{n} hearts" holding "8 of".
    fn template(&mut self, core: &str, depth: usize) -> Option<Hit> {
        let catalog = self.catalog;
        let mut kept_some: Option<Hit> = None;
        let mut tried_after = 0;
        // A clean hit and its hole count: of two templates as heavy, the
        // one with more holes wins, as "Next night: night {next}, {name}"
        // beats "Next night: night {next}" with "2, every side" as {next}.
        // A clean hit, its hole count, and whether a hole took a clause
        // (", "). A hit whose hole took a clause gives way to a lighter
        // template with more holes and none: "Against the clock, {name}"
        // with "Downtown, on the road bike" as the name loses to
        // "{clock_name}, {district}, on the {bike}".
        let mut clean: Option<(Hit, usize, bool)> = None;
        // What followed the comma in the clean hit's loose holes: a lighter
        // template must read its own words there to take over.
        let mut best_tails: Vec<String> = Vec::new();
        let mut tried_lighter = 0;
        for (template, captures) in catalog
            .templates
            .matches(core, move |template, caps| accepts(catalog, template, caps))
        {
            let holes = captures.len();
            let loose = captures.values().any(|value| value.contains(", "));
            if let Some((best, best_holes, best_loose)) = &clean {
                if template.weight < best.weight {
                    if !*best_loose {
                        break;
                    }
                    tried_lighter += 1;
                    if tried_lighter > MAX_LOOSER_TRIES {
                        break;
                    }
                    if loose || holes <= *best_holes || !reads_into(template, &best_tails) {
                        continue;
                    }
                } else if holes <= *best_holes {
                    continue;
                }
                if let Some(hit) = self.render(template, &captures, depth) {
                    if hit.kept.is_empty() {
                        best_tails = clause_tails(&captures);
                        clean = Some((hit, holes, loose));
                    }
                }
                continue;
            }
            if kept_some.is_some() {
                tried_after += 1;
                if tried_after > MAX_LOOSER_TRIES {
                    break;
                }
            }
            let Some(hit) = self.render(template, &captures, depth) else {
                continue;
            };
            if hit.kept.is_empty() {
                best_tails = clause_tails(&captures);
                clean = Some((hit, holes, loose));
                continue;
            }
            kept_some.get_or_insert(hit);
        }
        clean.map(|(hit, _, _)| hit).or(kept_some)
    }

    /// "1, 2, 3, 4" in Chinese or Japanese, with their own comma.
    fn cjk_number_list(&self, raw: &str) -> Option<String> {
        let cjk = matches!(
            self.catalog.language.split('_').next().unwrap_or_default(),
            "zh" | "ja"
        );
        (cjk && raw.contains(", ")
            && raw
                .chars()
                .all(|c| c.is_ascii_digit() || c == ',' || c == ' '))
        .then(|| raw.replace(", ", "\u{3001}"))
    }

    fn render(
        &mut self,
        template: &Template,
        captures: &HashMap<&str, &str>,
        depth: usize,
    ) -> Option<Hit> {
        let mut out = String::new();
        // English words the holes would keep, untranslated, inside the
        // translated frame.
        let mut kept_english = 0;
        let mut kept = Vec::new();
        for part in &template.translation {
            match part {
                Part::Lit(text) => out.push_str(text),
                Part::Hole(name) => {
                    let raw = captures.get(name.as_str()).copied().unwrap_or_default();
                    if is_proper_name_hole(name) && depth < MAX_DEPTH {
                        // A name hole can hold a place phrase around names:
                        // "retail distribution hub {place_name} in {city}"
                        // says its kind and "in" in the language, the names
                        // as they are.
                        if let Some(phrase) = self.place_phrase(raw) {
                            out.push_str(&phrase);
                            continue;
                        }
                    }
                    if is_proper_name_hole(name) || depth >= MAX_DEPTH {
                        // "{the_city}" holds "the Chicago" in English; the
                        // translation brings its own article, if any.
                        let bare = name
                            .starts_with("the_")
                            .then(|| raw.strip_prefix("the ").or(raw.strip_prefix("The ")))
                            .flatten();
                        out.push_str(bare.unwrap_or(raw));
                    } else if let Some(list) = self.cjk_number_list(raw) {
                        out.push_str(&list);
                    } else if let Some(number) =
                        plural::localize_number(&self.catalog.language, raw)
                    {
                        out.push_str(&number);
                    } else {
                        // A capture is often a phrase the catalog knows
                        // ("heavy traffic"); an unknown one (a number, a
                        // name) is simply kept, and not a miss.
                        let mut ignored = Vec::new();
                        let translated = Pass {
                            catalog: self.catalog,
                            missed: &mut ignored,
                            memo: &mut *self.memo,
                        }
                        .piece(raw, depth + 1, has_sentence_break(raw));
                        // A measure ("2 miles") or a name kept as is is
                        // not a word left behind.
                        if translated.is_none() && !is_name_like(raw) {
                            if !is_measure(raw) {
                                kept_english += raw.chars().filter(|c| c.is_alphabetic()).count();
                            }
                            // A measure left as it is still counts against
                            // the match: "9 days" is words of the language too.
                            if has_words(raw) {
                                kept.push(raw.trim().to_string());
                            }
                        }
                        let translated = match translated {
                            // A capture that opens a sentence of the frame
                            // opens with a capital: ". {progress}" is
                            // ". Quedan 451 millas", not ". quedan".
                            Some(text)
                                if !starts_lowercase(raw)
                                    && out.trim_end().ends_with(['.', '!', '?']) =>
                            {
                                Some(capitalize_first_letter(&text))
                            }
                            other => other,
                        };
                        out.push_str(translated.as_deref().unwrap_or(raw));
                    }
                }
                Part::Plural(name, forms) => {
                    let raw = captures.get(name.as_str()).copied().unwrap_or_default();
                    out.push_str(plural::choose(&self.catalog.language, forms, raw));
                }
            }
        }
        // "Lifetime earnings" in "{what} en total" is a word-by-word
        // translation, not a translated line: as much English as frame.
        if kept_english >= template.weight {
            return None;
        }
        // Every source hole must have been captured for the line to match;
        // a template whose segments carry a hole the translation drops is
        // still a faithful match (the translator chose to drop it).
        debug_assert!(template
            .segs
            .iter()
            .all(|seg| !matches!(seg, Seg::Hole(name) if !captures.contains_key(name.as_str()))));
        Some(Hit {
            weight: template.weight,
            out,
            kept,
            loose: captures.values().any(|value| value.contains(", ")),
            pieces: captures
                .values()
                .map(|value| value.to_lowercase())
                .chain(template.segs.iter().filter_map(|seg| match seg {
                    Seg::Lit(text) => Some(text.to_lowercase()),
                    Seg::Hole(_) => None,
                }))
                .collect(),
        })
    }
}

/// The lowercase text after the first ", " in each capture that has one.
fn clause_tails(captures: &HashMap<&str, &str>) -> Vec<String> {
    captures
        .values()
        .filter_map(|value| value.split_once(", ").map(|(_, tail)| tail.to_lowercase()))
        .collect()
}

/// Whether a literal word of `template` stands in one of `tails`.
fn reads_into(template: &Template, tails: &[String]) -> bool {
    template.segs.iter().any(|seg| match seg {
        Seg::Lit(text) => text
            .split(|c: char| !c.is_alphabetic())
            .filter(|word| word.len() >= 2)
            .any(|word| {
                let word = word.to_lowercase();
                tails
                    .iter()
                    .any(|tail| tail.split(|c: char| !c.is_alphabetic()).any(|w| w == word))
            }),
        Seg::Hole(_) => false,
    })
}

/// Whether a template the line fills may take these captures.
fn accepts(catalog: &Catalog, template: &Template, caps: &HashMap<&str, &str>) -> bool {
    // A one-sentence template never swallows several sentences into a
    // hole: "It is {tod}." must not match "It is 9:00. Fuel is low.",
    // which the sentence splitter translates whole.
    // "{place}{state} on {highway}" has few letters of its own, but
    // its holes are names: it matches when what fills them reads as
    // names ("Groton", ", South Dakota", "US-12"), never words a loose
    // match would swallow ("Status ready to continue"). Nor words the
    // catalog has whole: "{rank} of {suit}" is "6 of spades" when
    // "spades" is a word of its own, as a game builds a card name.
    // Numbers alone never fill one either: "2 of 2." after a menu
    // row is its position, not "{rank_words} of {name}".
    // A hole named for a count holds a number: "Night {n}" is never "Night
    // 1, over the chimney" with the words in {n}.
    if caps.iter().any(|(name, value)| {
        (COUNT_HOLES.contains(name) || counts_its_unit(template, name)) && !is_number(value)
    }) {
        return false;
    }
    // A hole that ends the template never takes the stop of a short
    // phrase: "{name} on the {name2}" is not {name2} = "road bike.", which
    // the line's stop after "{name} on the {name2}" reads right.
    let after_word = template.segs.len() >= 2
        && matches!(&template.segs[template.segs.len() - 2],
            Seg::Lit(text) if text.ends_with(' ') && text.trim_end().ends_with(char::is_alphabetic));
    if let (true, Some(Seg::Hole(last))) = (after_word, template.segs.last()) {
        if caps.get(last.as_str()).is_some_and(|value| {
            let value = value.trim_end();
            value.ends_with('.')
                && !has_sentence_break(value)
                && value.split_whitespace().count() <= 3
        }) {
            return false;
        }
    }
    // A hole never ends on the comma or colon of the frame around it:
    // "{name} skipped." is not {name} = "Fireside Patience, learn to play:".
    if caps
        .values()
        .any(|value| value.trim_end().ends_with([',', ':']))
    {
        return false;
    }
    // A hole a word follows, as in "{got_away} got away", is a count of
    // that word: never a list of counts, "0 perfect, 0 missed, 10".
    if caps.iter().any(|(name, value)| {
        before_word(template, name)
            && value.contains(", ")
            && value.trim_start().starts_with(|c: char| c.is_ascii_digit())
    }) {
        return false;
    }
    // Any hole before a unit is a short amount: "{rest} seconds" is not
    // "Ramp to your right, in 2" seconds.
    if caps.iter().any(|(name, value)| {
        before_unit(template, name)
            && (value.contains(", ")
                || value
                    .split(|c: char| !c.is_alphabetic())
                    .filter(|w| !w.is_empty())
                    .any(|w| !AMOUNT_WORDS.contains(&w)))
    }) {
        return false;
    }
    // Nor does any hole take a number and the clause after it: "Play,
    // night {next}" is not "Play, night 1, over the chimney".
    if caps.values().any(|value| {
        value
            .split_once(", ")
            .is_some_and(|(head, rest)| is_number(head) && !is_number_list(rest))
    }) {
        return false;
    }
    // A number never takes the full stop that ends its sentence: "Night
    // 1." is "Night {n}" with a stop after it, not {n} = "1.".
    if caps
        .values()
        .any(|value| is_number(value) && value.trim_end().ends_with('.'))
    {
        return false;
    }
    if template.numbers_only()
        && caps
            .iter()
            .all(|(_, value)| is_number(value) || value.trim().is_empty())
    {
        return false;
    }
    if template.numbers_only()
        && !caps.iter().all(|(name, value)| {
            is_number(value)
                || (is_proper_name_hole(name) && reads_as_name(value))
                || catalog.knows_word(value)
        })
    {
        return false;
    }
    // A name never holds a lead-in: "{studio} is looking for ..."
    // must not take "Mission brief: Pinecone Lantern Works" as
    // the studio and leave the label English; nor does a hole that
    // opens a template: "{name} Company Yard" must not take
    // "company yard: Chicago" and say the label twice.
    if caps.iter().any(|(name, value)| {
        value.contains(": ")
            && (is_proper_name_hole(name)
                || (matches!(template.segs.first(), Some(Seg::Hole(first)) if first == name)
                    && !template.filled.contains(':')))
    }) {
        return false;
    }
    // A name that opens a template reads as one: "{the_city} metro
    // local market" must not take "12 crates of general goods
    // from Harbor yard in Harbor to the Capital" as its city.
    if let Some(Seg::Hole(first)) = template.segs.first() {
        if is_proper_name_hole(first)
            && caps
                .get(first.as_str())
                .is_some_and(|value| !reads_as_place(value))
        {
            return false;
        }
    }
    // Only a hole that ends the template or opens a sentence of it
    // may hold sentences of its own ("Status: {summary}", "How to play.
    // {controls} {page_title}"): "{pct} percent {direction}
    // ahead{length}. {advice}" must not take a whole help text as
    // its percent and leave it English.
    let multi = has_sentence_break(&template.filled);
    !caps.iter().any(|(name, value)| {
        has_sentence_break(value) && !(multi && holds_sentences(&template.segs, name))
    })
}

/// `text` with its first letter lowercased, for a piece that goes
/// mid-sentence.
fn lowercase_first_letter(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `text` with its first letter capitalized.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The first letter is lowercase ("the Down arrow ..."): English did not
/// open a sentence with a capital, so the translation need not either.
fn starts_lowercase(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphanumeric())
        .is_some_and(char::is_lowercase)
}

/// `text` with its first letter (after any digits or marks) capitalized:
/// "451 millas" stays, "quedan 451 millas" becomes "Quedan 451 millas".
fn capitalize_first_letter(text: &str) -> String {
    let Some((at, first)) = text.char_indices().find(|(_, c)| c.is_alphanumeric()) else {
        return text.to_string();
    };
    if !first.is_lowercase() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..at]);
    out.extend(first.to_uppercase());
    out.push_str(&text[at + first.len_utf8()..]);
    out
}

/// A number as the game writes one: "3", "1,250", "2.5", "3:15".
/// The hole is followed by a word: "{got_away} got away".
fn before_word(template: &Template, name: &str) -> bool {
    template.segs.windows(2).any(|pair| match pair {
        [Seg::Hole(hole), Seg::Lit(text)] if hole == name => text
            .strip_prefix(' ')
            .is_some_and(|text| text.starts_with(char::is_alphabetic)),
        _ => false,
    })
}

/// The hole is followed by a unit word: "{rest} seconds".
fn before_unit(template: &Template, name: &str) -> bool {
    template.segs.windows(2).any(|pair| match pair {
        [Seg::Hole(hole), Seg::Lit(text)] if hole == name => text
            .strip_prefix(' ')
            .and_then(|text| text.split(|c: char| !c.is_alphabetic()).next())
            .is_some_and(|word| COUNTED_UNITS.contains(&word)),
        _ => false,
    })
}

/// "{seconds} seconds" and "{hours} hours" count: the hole is followed
/// by its own name as a word, so it holds a number, never "Car door in 13".
fn counts_its_unit(template: &Template, name: &str) -> bool {
    template.segs.windows(2).any(|pair| match pair {
        [Seg::Hole(hole), Seg::Lit(text)] if hole == name => text
            .trim_start()
            .split(|c: char| !c.is_alphabetic())
            .next()
            .is_some_and(|word| word == name && COUNTED_UNITS.contains(&word)),
        _ => false,
    })
}

/// Numbers joined as a list: "6, 7 and 10" is pins, not a clause.
fn is_number_list(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .all(|word| word.chars().all(|c| c.is_ascii_digit()) || matches!(word, "and" | "or"))
}

fn is_number(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ':' | '-' | ' '))
}

/// A trailing menu position cue: "Settings. 4 of 8." is the row
/// "Settings." with position 4 of 8. The row must end a sentence, so
/// "Results 26 to 50 of 52." is not one.
fn split_position_cue(line: &str) -> Option<(&str, &str, &str)> {
    let body = line.strip_suffix('.')?;
    let (rest, count) = body.rsplit_once(' ')?;
    let rest = rest.strip_suffix(" of")?;
    let (row, position) = rest.rsplit_once(' ')?;
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !digits(count) || !digits(position) {
        return None;
    }
    let row = row.trim_end();
    row.ends_with(['.', '!', '?', ':'])
        .then_some((row, position, count))
}

/// A piece that reads as a name or a number, not words to translate:
/// ".977 Smooth Jazz", "Bemidji Company Yard", "US-12". Every word opens
/// with a capital letter, a digit or a mark, and there is more than one
/// word or a digit: a lone capitalized word ("Deliveries") is as likely
/// a label the catalog lacks as a name.
fn is_name_like(text: &str) -> bool {
    let mut words = text.split_whitespace().peekable();
    if words.peek().is_none() {
        return false;
    }
    let capitalized = text.split_whitespace().all(|word| {
        !word
            .chars()
            .find(|c| c.is_alphanumeric())
            .is_some_and(char::is_lowercase)
    });
    capitalized && (text.split_whitespace().count() > 1 || text.chars().any(|c| c.is_ascii_digit()))
}

/// What fills a name hole reads as a name: empty, or every word opens with a
/// capital, a digit or a mark ("Groton", ", South Dakota", "US-12").
fn reads_as_name(text: &str) -> bool {
    text.split_whitespace().all(|word| {
        !word
            .chars()
            .find(|c| c.is_alphanumeric())
            .is_some_and(char::is_lowercase)
    })
}

/// Reads as a place's name with its small words: "Central Depot
/// North in Midland", "the Capital", never "12 crates of general goods".
fn reads_as_place(text: &str) -> bool {
    const SMALL: &[&str] = &[
        "the", "of", "in", "near", "and", "at", "on", "de", "la", "du",
    ];
    text.split_whitespace().all(|word| {
        SMALL.contains(&word)
            || !word
                .chars()
                .find(|c| c.is_alphanumeric())
                .is_some_and(char::is_lowercase)
    })
}

/// A number and its unit, as a readout says one: "2 miles", "3:15 PM".
fn is_measure(text: &str) -> bool {
    text.trim_start().starts_with(|c: char| c.is_ascii_digit())
        && text.split_whitespace().count() <= 2
}

fn has_words(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphabetic()).take(2).count() >= 2
}

/// Spanish joins a preposition to the article after it, which
/// two catalog pieces put side by side do not: "por debajo de" + "el
/// límite" is "por debajo del límite". A capital "El" is a name (El Paso)
/// and stays apart.
fn contract_articles(language: &str, out: String) -> String {
    let pairs: &[(&str, &str)] = match language.split('_').next().unwrap_or_default() {
        "es" => &[(" de el ", " del "), (" a el ", " al ")],
        _ => return out,
    };
    let mut out = out;
    for (apart, joined) in pairs {
        if out.contains(apart) {
            out = out.replace(apart, joined);
        }
    }
    out
}

/// The hole may hold whole sentences: it is the template's last part, or
/// a sentence of the template ends before it or opens after it.
fn holds_sentences(segs: &[Seg], hole: &str) -> bool {
    let Some(at) = segs
        .iter()
        .position(|seg| matches!(seg, Seg::Hole(name) if name == hole))
    else {
        return false;
    };
    if at + 1 == segs.len() {
        return true;
    }
    // "{flavor} The match starts on ...": a sentence of the template
    // opens right after it.
    if let Some(Seg::Lit(text)) = segs.get(at + 1) {
        if text.starts_with(char::is_whitespace)
            && text.trim_start().starts_with(char::is_uppercase)
        {
            return true;
        }
    }
    match at.checked_sub(1).map(|before| &segs[before]) {
        Some(Seg::Lit(text)) => text
            .trim_end()
            .ends_with(['.', '!', '?', '\u{3002}', '\u{FF01}', '\u{FF1F}']),
        _ => false,
    }
}

fn has_sentence_break(text: &str) -> bool {
    split_sentences(text).len() > 1
}

/// Ends in a full-width sentence mark (Chinese, Japanese), after which no
/// space is written.
fn ends_full_width(text: &str) -> bool {
    text.trim_end()
        .ends_with(['。', '！', '？', '：', '；', '，', '、'])
}

/// Sentences, each with the whitespace that followed it: a break is a
/// `.`, `!` or `?` followed by a space and then a capital letter, a digit
/// or a quote.
fn split_sentences(text: &str) -> Vec<(&str, &str)> {
    split_at_marks(text, |text, at| {
        let before = text[..at].chars().next_back();
        if !matches!(before, Some('.' | '!' | '?')) {
            return None;
        }
        let rest = &text[at..];
        let gap = rest.len() - rest.trim_start().len();
        if gap == 0 {
            return None;
        }
        let next = rest[gap..].chars().next()?;
        (next.is_uppercase() || next.is_ascii_digit() || next == '"').then_some(gap)
    })
}

/// Clauses: split at ", ", ": ", "; " and " -- ", keeping the separator.
fn split_clauses(text: &str) -> Vec<(&str, &str)> {
    split_at_marks(text, |text, at| {
        let rest = &text[at..];
        for sep in [", ", ": ", "; ", " -- "] {
            if rest.starts_with(sep) {
                return Some(sep.len());
            }
        }
        None
    })
}

/// Walk `text`; `mark(text, at)` returns the separator length starting at
/// byte `at` when a piece ends there.
fn split_at_marks(text: &str, mark: impl Fn(&str, usize) -> Option<usize>) -> Vec<(&str, &str)> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < text.len() {
        if at > start {
            if let Some(len) = mark(text, at) {
                pieces.push((&text[start..at], &text[at..at + len]));
                at += len;
                start = at;
                continue;
            }
        }
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    if start < text.len() {
        pieces.push((&text[start..], ""));
    }
    pieces
}

#[cfg(test)]
mod tests;
