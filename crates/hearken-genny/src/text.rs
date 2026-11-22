//! Text to phonemes, as genny's English front end does it: the built-in
//! dictionary, then the CMU Pronouncing Dictionary (with the `cmudict`
//! feature), then the same with a regular suffix stripped, then
//! letter-to-sound rules; number words and letter names; weak forms of
//! function words and the American flap across a phrase.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::phonemes::{id, ph};

/// One phoneme of a word: index into the inventory and its stress (0, 1, 2).
pub type Phone = (usize, u8);

#[derive(Clone, Debug)]
pub struct Word {
    pub ph: Vec<Phone>,
    /// A spelled letter, which gets its own short pause and final lengthening.
    pub letter: bool,
    /// A function word in its weak form: unstressed and shorter.
    pub func: bool,
    /// The full form a function word takes when it ends a phrase.
    strong: Option<Vec<Phone>>,
}

#[derive(Clone, Debug)]
pub struct Phrase {
    pub words: Vec<Word>,
    /// The punctuation that ended it, or `'\0'` at the end of the text.
    pub punct: char,
}

static DICT_SRC: &str = include_str!("dict_en.txt");

/// "word PH ON ES|word PH ON ES" lines into a map.
fn parse_dict(src: &'static str) -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    for line in src.lines() {
        for entry in line.split('|') {
            let entry = entry.trim();
            if let Some((word, phones)) = entry.split_once(' ') {
                map.insert(word, phones.trim());
            }
        }
    }
    map
}

/// genny's own dictionary: numbers and the words games say most.
fn dict() -> &'static HashMap<&'static str, &'static str> {
    static DICT: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    DICT.get_or_init(|| parse_dict(DICT_SRC))
}

fn letter_en(c: char) -> Option<&'static str> {
    Some(match c {
        'a' => "EY1",
        'b' => "B IY1",
        'c' => "S IY1",
        'd' => "D IY1",
        'e' => "IY1",
        'f' => "EH1 F",
        'g' => "JH IY1",
        'h' => "EY1 CH",
        'i' => "AY1",
        'j' => "JH EY1",
        'k' => "K EY1",
        'l' => "EH1 L",
        'm' => "EH1 M",
        'n' => "EH1 N",
        'o' => "OW1",
        'p' => "P IY1",
        'q' => "K Y UW1",
        'r' => "AA1 R",
        's' => "EH1 S",
        't' => "T IY1",
        'u' => "Y UW1",
        'v' => "V IY1",
        'w' => "D AH1 B AH0 L Y UW0",
        'x' => "EH1 K S",
        'y' => "W AY1",
        'z' => "Z IY1",
        _ => return None,
    })
}

/// "F Y UW1 AH0 L" -> phones. Unknown names are skipped.
fn parse_ph(s: &str) -> Vec<Phone> {
    s.split_whitespace()
        .filter_map(|t| {
            let (name, stress) = match t.chars().last() {
                Some(d @ '0'..='2') => (&t[..t.len() - 1], d as u8 - b'0'),
                _ => (t, 0),
            };
            id(name).map(|i| (i, stress))
        })
        .collect()
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

pub fn number_words(n: u64) -> Vec<&'static str> {
    let mut out = Vec::new();
    if n < 20 {
        out.push(ONES[n as usize]);
    } else if n < 100 {
        out.push(TENS[(n / 10) as usize]);
        if !n.is_multiple_of(10) {
            out.push(ONES[(n % 10) as usize]);
        }
    } else if n < 1000 {
        out.push(ONES[(n / 100) as usize]);
        out.push("hundred");
        if !n.is_multiple_of(100) {
            out.extend(number_words(n % 100));
        }
    } else if n < 1_000_000 {
        out.extend(number_words(n / 1000));
        out.push("thousand");
        if !n.is_multiple_of(1000) {
            out.extend(number_words(n % 1000));
        }
    } else if n < 1_000_000_000 {
        out.extend(number_words(n / 1_000_000));
        out.push("million");
        if !n.is_multiple_of(1_000_000) {
            out.extend(number_words(n % 1_000_000));
        }
    } else {
        for c in n.to_string().chars() {
            out.push(ONES[c.to_digit(10).unwrap_or(0) as usize]);
        }
    }
    out
}

fn is_v(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

/// genny's small English letter-to-sound rules, for words missing from the
/// dictionary. Approximate.
pub fn lts(word: &str) -> Vec<Phone> {
    let w: Vec<char> = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase())
        .collect();
    let n = w.len();
    let at = |j: isize| -> Option<char> {
        if j >= 0 && (j as usize) < n {
            Some(w[j as usize])
        } else {
            None
        }
    };
    let starts = |i: usize, pat: &str| -> bool {
        let p: Vec<char> = pat.chars().collect();
        i + p.len() <= n && w[i..i + p.len()] == p[..]
    };
    let magic_e = |j: usize| -> bool {
        j + 2 == n.wrapping_sub(1)
            && at(j as isize + 2) == Some('e')
            && at(j as isize + 1).is_some_and(|c| !is_v(c) && !matches!(c, 'w' | 'x' | 'y'))
    };
    let vowelish = |name: &str| id(name).is_some_and(|i| ph(i).kind.vowelish());

    let mut out: Vec<&'static str> = Vec::new();
    let mut stress_mark: Option<isize> = None;
    let mut i = 0usize;
    const PATTERNS: [(&str, &[&str]); 35] = [
        ("augh", &["AO"]),
        ("eigh", &["EY"]),
        ("ough", &["AO"]),
        ("igh", &["AY"]),
        ("tch", &["CH"]),
        ("dge", &["JH"]),
        ("sch", &["S", "K"]),
        ("ck", &["K"]),
        ("ch", &["CH"]),
        ("sh", &["SH"]),
        ("th", &["TH"]),
        ("ph", &["F"]),
        ("wh", &["W"]),
        ("qu", &["K", "W"]),
        ("nk", &["NG", "K"]),
        ("ng", &["NG"]),
        ("ee", &["IY"]),
        ("ea", &["IY"]),
        ("oo", &["UW"]),
        ("ou", &["AW"]),
        ("oi", &["OY"]),
        ("oy", &["OY"]),
        ("ai", &["EY"]),
        ("ay", &["EY"]),
        ("oa", &["OW"]),
        ("au", &["AO"]),
        ("aw", &["AO"]),
        ("ew", &["UW"]),
        ("ie", &["IY"]),
        ("ue", &["UW"]),
        ("ar", &["AA", "R"]),
        ("or", &["AO", "R"]),
        ("er", &["ER"]),
        ("ir", &["ER"]),
        ("ur", &["ER"]),
    ];
    'outer: while i < n {
        let c = w[i];
        if starts(i, "tion") || starts(i, "sion") {
            stress_mark = Some(out.iter().filter(|p| vowelish(p)).count() as isize - 1);
            out.push(if c == 't' { "SH" } else { "ZH" });
            out.push("AH");
            out.push("N");
            i += 4;
            continue;
        }
        if starts(i, "ture") {
            out.push("CH");
            out.push("ER");
            i += 4;
            continue;
        }
        for (pat, phones) in PATTERNS {
            if starts(i, pat) {
                if matches!(pat, "ar" | "or" | "er" | "ir" | "ur")
                    && at(i as isize + 2).is_some_and(is_v)
                {
                    continue; // r starts the next syllable
                }
                out.extend_from_slice(phones);
                i += pat.len();
                continue 'outer;
            }
        }
        if i == 0 && (starts(0, "kn") || starts(0, "gn") || starts(0, "wr")) {
            i += 1;
            continue;
        }
        if starts(i, "ow") {
            out.push(if i + 2 == n { "OW" } else { "AW" });
            i += 2;
            continue;
        }
        if starts(i, "ey") && i + 2 == n {
            out.push("IY");
            i += 2;
            continue;
        }
        if is_v(c) {
            let before_has_v = w[..i].iter().any(|&ch| is_v(ch) || ch == 'y');
            if c == 'e' && i == n - 1 && n > 2 && before_has_v {
                i += 1;
                continue; // silent final e
            }
            if magic_e(i) {
                out.push(match c {
                    'a' => "EY",
                    'e' => "IY",
                    'i' => "AY",
                    'o' => "OW",
                    _ => "UW",
                });
            } else if c == 'o' && i == n - 1 {
                out.push("OW");
            } else if c == 'e' && n <= 2 && i == n - 1 {
                out.push("IY");
            } else if c == 'a' && i == n - 1 {
                out.push("AH");
            } else {
                out.push(match c {
                    'a' => "AE",
                    'e' => "EH",
                    'i' => "IH",
                    'o' => "AA",
                    _ => "AH",
                });
            }
        } else if c == 'y' {
            if i == 0 {
                out.push("Y");
            } else if i == n - 1 {
                let nv = w.iter().filter(|&&ch| is_v(ch)).count();
                out.push(if nv == 0 { "AY" } else { "IY" });
            } else {
                out.push("IH");
            }
        } else if c == 'c' {
            out.push(if matches!(at(i as isize + 1), Some('e' | 'i' | 'y')) {
                "S"
            } else {
                "K"
            });
        } else if c == 'g' {
            let soft = matches!(at(i as isize + 1), Some('e' | 'i' | 'y')) && i + 1 < n - 1;
            out.push(if soft { "JH" } else { "G" });
        } else if c == 'x' {
            out.push("K");
            out.push("S");
        } else if c == 's' {
            let prev = at(i as isize - 1);
            let next = at(i as isize + 1);
            let between_vowels =
                0 < i && i < n - 1 && prev.is_some_and(is_v) && next.is_some_and(is_v);
            let final_voiced = i == n - 1 && n > 2 && prev.is_some_and(|p| "bdglmnrv".contains(p));
            out.push(if between_vowels || final_voiced {
                "Z"
            } else {
                "S"
            });
        } else if c == 'j' {
            out.push("JH");
        } else if c == 'q' {
            out.push("K");
        } else {
            let m = match c {
                'b' => Some("B"),
                'd' => Some("D"),
                'f' => Some("F"),
                'h' => Some("HH"),
                'k' => Some("K"),
                'l' => Some("L"),
                'm' => Some("M"),
                'n' => Some("N"),
                'p' => Some("P"),
                'r' => Some("R"),
                't' => Some("T"),
                'v' => Some("V"),
                'w' => Some("W"),
                'z' => Some("Z"),
                _ => None,
            };
            if let Some(m) = m {
                let doubled = out.last() == Some(&m) && at(i as isize - 1) == Some(c);
                if !doubled {
                    out.push(m);
                }
            }
        }
        i += 1;
    }
    if out.is_empty() {
        return Vec::new();
    }
    let ids: Vec<usize> = out.iter().filter_map(|p| id(p)).collect();
    let vidx: Vec<usize> = ids
        .iter()
        .enumerate()
        .filter(|(_, &p)| ph(p).kind.vowelish())
        .map(|(j, _)| j)
        .collect();
    let target = match stress_mark {
        Some(m) if m >= 0 && (m as usize) < vidx.len() => vidx[m as usize] as isize,
        _ => vidx.first().map_or(-1, |&v| v as isize),
    };
    let reduce = ["AE", "EH", "AA", "AH", "UH"];
    let ah = id("AH").unwrap_or(0);
    ids.iter()
        .enumerate()
        .map(|(j, &p)| {
            if ph(p).kind.vowelish() {
                if j as isize == target {
                    (p, 1)
                } else if reduce.contains(&ph(p).name) {
                    (ah, 0)
                } else {
                    (p, 0)
                }
            } else {
                (p, 0)
            }
        })
        .collect()
}

/// The phones of a word in the built-in dictionary or, with the `cmudict`
/// feature, the CMU Pronouncing Dictionary.
fn known(word: &str) -> Option<Vec<Phone>> {
    if let Some(p) = dict().get(word) {
        return Some(parse_ph(p));
    }
    crate::cmudict::lookup(word).map(parse_ph)
}

const VOICELESS: [&str; 8] = ["P", "T", "K", "F", "TH", "S", "SH", "CH"];
const SIBILANT: [&str; 6] = ["S", "Z", "SH", "ZH", "CH", "JH"];

fn phone(name: &str) -> Phone {
    (id(name).expect("an English phoneme"), 0)
}

fn last_name(phones: &[Phone]) -> &'static str {
    phones.last().map_or("", |&(p, _)| ph(p).name)
}

/// The built-in dictionary, then CMUdict, then the same with a regular
/// suffix stripped (-'s, -ing, -ed, -s, -ly, -er, -ness, -less).
fn lookup_en(low: &str) -> Option<Vec<Phone>> {
    if let Some(p) = known(low) {
        return Some(p);
    }
    let chars: Vec<char> = low.chars().collect();
    let n = chars.len();
    // The first stem of at least two letters that either dictionary knows.
    let base = |cands: &[String]| -> Option<Vec<Phone>> {
        cands
            .iter()
            .filter(|b| b.chars().count() >= 2)
            .find_map(|b| known(b))
            .filter(|p| !p.is_empty())
    };
    let cut = |k: usize| -> String { chars[..n.saturating_sub(k)].iter().collect() };
    let plural = |mut b: Vec<Phone>| -> Vec<Phone> {
        let last = last_name(&b);
        if SIBILANT.contains(&last) {
            b.extend([phone("IH"), phone("Z")]);
        } else if VOICELESS.contains(&last) {
            b.push(phone("S"));
        } else {
            b.push(phone("Z"));
        }
        b
    };
    if low.ends_with("'s") {
        if let Some(b) = base(&[cut(2)]) {
            return Some(plural(b));
        }
    }
    if low.ends_with("ing") {
        // running -> run
        let doubled = if n > 5 && chars[n - 4] == chars[n - 5] {
            cut(4)
        } else {
            String::new()
        };
        if let Some(mut b) = base(&[cut(3), cut(3) + "e", doubled]) {
            b.extend([phone("IH"), phone("NG")]);
            return Some(b);
        }
    }
    if low.ends_with("ed") {
        // stopped -> stop
        let doubled = if n > 4 && chars[n - 3] == chars[n - 4] {
            cut(3)
        } else {
            String::new()
        };
        if let Some(mut b) = base(&[cut(2), cut(1), doubled]) {
            let last = last_name(&b);
            if matches!(last, "T" | "D") {
                b.extend([phone("IH"), phone("D")]);
            } else if VOICELESS.contains(&last) {
                b.push(phone("T"));
            } else {
                b.push(phone("D"));
            }
            return Some(b);
        }
    }
    if low.ends_with('s') {
        let es = if low.ends_with("es") {
            base(&[cut(2)])
        } else {
            None
        };
        if let Some(b) = es.or_else(|| base(&[cut(1)])) {
            return Some(plural(b));
        }
    }
    let suffixes: [(&str, &[&str]); 4] = [
        ("ly", &["L", "IY"]),
        ("er", &["ER"]),
        ("ness", &["N", "AH", "S"]),
        ("less", &["L", "AH", "S"]),
    ];
    for (suf, add) in suffixes {
        if low.ends_with(suf) {
            let stem = cut(suf.len());
            if let Some(mut b) = base(&[stem.clone(), stem + "e"]) {
                b.extend(add.iter().map(|&a| phone(a)));
                return Some(b);
            }
        }
    }
    None
}

static FUNC_SRC: &str = "\
a AH0|an AH0 N|the DH AH0|and AH0 N D|of AH0 V|to T AH0|for F ER0|from F R AH0 M|at AH0 T|in IH0 N|on AA0 N
by B AY0|with W IH0 DH|as AH0 Z|or ER0|nor N ER0|but B AH0 T|than DH AH0 N|that DH AH0 T|if IH0 F|into IH0 N T UW0
onto AA0 N T UW0|upon AH0 P AA0 N|is IH0 Z|are ER0|was W AH0 Z|were W ER0|be B IY0|been B IH0 N|am AH0 M
can K AH0 N|could K UH0 D|would W UH0 D|should SH UH0 D|will W IH0 L|shall SH AH0 L|must M AH0 S T
do D UW0|does D AH0 Z|has HH AH0 Z|had HH AH0 D|it IH0 T|its IH0 T S|he HH IY0|she SH IY0|we W IY0|you Y UW0
they DH EY0|me M IY0|him IH0 M|her HH ER0|them DH AH0 M|us AH0 S|my M AY0|your Y ER0|his HH IH0 Z|our AW0 ER0
their DH EH0 R|there DH ER0|some S AH0 M|so S OW0|just JH AH0 S T";

/// Function words in running speech, with their weak (reduced) vowel. One
/// said on its own, or stranded at the end of a phrase ("what is it for?"),
/// keeps its full dictionary form.
fn func_words() -> &'static HashMap<&'static str, &'static str> {
    static FUNC: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    FUNC.get_or_init(|| parse_dict(FUNC_SRC))
}

fn spoken(ph: Vec<Phone>, letter: bool) -> Word {
    Word {
        ph,
        letter,
        func: false,
        strong: None,
    }
}

fn function_word(weak: Vec<Phone>, strong: Vec<Phone>) -> Word {
    Word {
        ph: weak,
        letter: false,
        func: true,
        strong: Some(strong),
    }
}

/// Python's `str.isupper`: some cased letter and no lowercase one.
fn is_upper(tok: &str) -> bool {
    tok.chars().any(char::is_uppercase) && !tok.chars().any(char::is_lowercase)
}

fn word_en(tok: &str, single: bool) -> Vec<Word> {
    let low = tok.to_lowercase();
    let chars: Vec<char> = low.chars().collect();
    if chars.len() == 1 && chars[0].is_alphabetic() {
        if low == "a" && tok == "a" && !single {
            return vec![function_word(vec![phone("AH")], vec![(phone("EY").0, 1)])];
        }
        if let Some(l) = letter_en(chars[0]) {
            return vec![spoken(parse_ph(l), true)];
        }
    }
    let upper = is_upper(tok);
    if let Some(weak) = func_words().get(low.as_str()) {
        if !single && !upper {
            let strong = lookup_en(&low).unwrap_or_else(|| parse_ph(weak));
            return vec![function_word(parse_ph(weak), strong)];
        }
    }
    if upper && (2..=5).contains(&tok.chars().count()) && !dict().contains_key(low.as_str()) {
        // An initialism is spelled out unless CMUdict says it reads as a word.
        if let Some(c) = crate::cmudict::lookup(&low) {
            if chars.len() >= 3 && chars.iter().any(|c| "aeiouy".contains(*c)) {
                return vec![spoken(parse_ph(c), false)];
            }
        }
        return low
            .chars()
            .filter_map(letter_en)
            .map(|l| spoken(parse_ph(l), true))
            .collect();
    }
    let phones = lookup_en(&low).unwrap_or_else(|| lts(&low));
    if phones.is_empty() {
        Vec::new()
    } else {
        vec![spoken(phones, false)]
    }
}

/// The American flap: T or D between a vowel (or R) and an unstressed vowel
/// inside a word (water, better, ready), but not before a syllabic n
/// (button, sudden).
fn flap_en(phones: &mut [Phone]) {
    let dx = phone("DX");
    let n = phones.len();
    for j in 1..n.saturating_sub(1) {
        let (p, (nx, nst)) = (ph(phones[j].0).name, phones[j + 1]);
        if !matches!(p, "T" | "D") || !ph(nx).kind.vowelish() || nst != 0 {
            continue;
        }
        let pv = ph(phones[j - 1].0);
        if !(pv.kind.vowelish() || pv.name == "R") {
            continue;
        }
        if matches!(ph(nx).name, "AH" | "IH") && j + 3 == n && ph(phones[j + 2].0).name == "N" {
            continue;
        }
        phones[j] = dx;
    }
}

/// Running-speech English: flaps, weak forms, "the" and "to" before a
/// vowel, a stranded function word strong.
fn connect_en(words: &mut [Word]) {
    let real: Vec<usize> = (0..words.len())
        .filter(|&i| !words[i].ph.is_empty())
        .collect();
    let (dh, ah, t) = (phone("DH"), phone("AH"), phone("T"));
    for (k, &i) in real.iter().enumerate() {
        if !words[i].letter {
            flap_en(&mut words[i].ph);
        }
        if !words[i].func {
            continue;
        }
        let Some(&next) = real.get(k + 1) else {
            let w = &mut words[i];
            if let Some(strong) = w.strong.take() {
                w.ph = strong;
            }
            w.func = false;
            continue;
        };
        if ph(words[next].ph[0].0).kind.vowelish() {
            let w = &mut words[i];
            if w.ph == [dh, ah] {
                w.ph = vec![dh, phone("IY")];
            } else if w.ph == [t, ah] {
                w.ph = vec![t, phone("UW")];
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Token {
    Digits(String),
    Word(String),
    Punct(char),
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '\'' || ('\u{C0}'..='\u{FF}').contains(&c)
}

fn tokens(text: &str) -> Vec<Token> {
    let text = text.replace('-', " ");
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Token::Digits(chars[start..i].iter().collect()));
        } else if is_word_char(c) {
            let start = i;
            while i < chars.len() && is_word_char(chars[i]) {
                i += 1;
            }
            out.push(Token::Word(chars[start..i].iter().collect()));
        } else {
            if matches!(c, ',' | '.' | ';' | ':' | '!' | '?') {
                out.push(Token::Punct(c));
            }
            i += 1;
        }
    }
    out
}

/// Split text into phrases at punctuation, each a list of words.
pub fn text_to_phrases(text: &str) -> Vec<Phrase> {
    let toks = tokens(text);
    let single = toks
        .iter()
        .filter(|t| !matches!(t, Token::Punct(_)))
        .count()
        == 1;
    let mut phrases = Vec::new();
    let mut cur: Vec<Word> = Vec::new();
    for tok in toks {
        match tok {
            Token::Punct(p) => {
                if !cur.is_empty() {
                    phrases.push(Phrase {
                        words: std::mem::take(&mut cur),
                        punct: p,
                    });
                }
            }
            Token::Digits(d) => match d.parse::<u64>() {
                Ok(n) => {
                    for w in number_words(n) {
                        cur.extend(word_en(w, false));
                    }
                }
                Err(_) => {
                    for c in d.chars() {
                        cur.extend(word_en(ONES[c.to_digit(10).unwrap_or(0) as usize], false));
                    }
                }
            },
            Token::Word(w) => cur.extend(word_en(&w, single)),
        }
    }
    if !cur.is_empty() {
        phrases.push(Phrase {
            words: cur,
            punct: '\0',
        });
    }
    for phrase in &mut phrases {
        connect_en(&mut phrase.words);
    }
    phrases
}

/// The ARPAbet transcription, for tests and debugging: words split by `|`.
pub fn to_phonemes(text: &str) -> String {
    text_to_phrases(text)
        .iter()
        .map(|phrase| {
            let words: Vec<String> = phrase
                .words
                .iter()
                .map(|w| {
                    w.ph.iter()
                        .map(|&(p, s)| {
                            let info = ph(p);
                            if info.kind.vowelish() {
                                format!("{}{}", info.name, s)
                            } else {
                                info.name.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect();
            let mut s = words.join(" | ");
            if phrase.punct != '\0' {
                s.push(' ');
                s.push(phrase.punct);
            }
            s
        })
        .collect::<Vec<_>>()
        .join("  ")
}
