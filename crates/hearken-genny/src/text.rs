//! Text to phonemes: genny's built-in dictionary, number words, letter
//! names and its letter-to-sound fallback (English).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::phonemes::{id, ph};

/// One phoneme of a word: index into the inventory and its stress (0, 1, 2).
pub type Phone = (usize, u8);

#[derive(Clone, Debug)]
pub enum Word {
    Spoken { ph: Vec<Phone>, letter: bool },
}

#[derive(Clone, Debug)]
pub struct Phrase {
    pub words: Vec<Word>,
    /// The punctuation that ended it, or `'\0'` at the end of the text.
    pub punct: char,
}

static DICT_SRC: &str = include_str!("dict_en.txt");

fn dict() -> &'static HashMap<&'static str, &'static str> {
    static DICT: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut map = HashMap::new();
        for line in DICT_SRC.lines() {
            for entry in line.split('|') {
                let entry = entry.trim();
                if let Some((word, phones)) = entry.split_once(' ') {
                    map.insert(word, phones.trim());
                }
            }
        }
        map
    })
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

fn spoken(ph: Vec<Phone>, letter: bool) -> Word {
    Word::Spoken { ph, letter }
}

fn word_en(tok: &str, single: bool) -> Vec<Word> {
    let low = tok.to_lowercase();
    let chars: Vec<char> = low.chars().collect();
    let d = dict();
    if chars.len() == 1 && chars[0].is_alphabetic() {
        if low == "a" && tok == "a" && !single {
            return vec![spoken(vec![(id("AH").unwrap_or(0), 0)], false)];
        }
        if let Some(l) = letter_en(chars[0]) {
            return vec![spoken(parse_ph(l), true)];
        }
    }
    if let Some(p) = d.get(low.as_str()) {
        return vec![spoken(parse_ph(p), false)];
    }
    if let Some(base) = low.strip_suffix("'s") {
        if let Some(p) = d.get(base) {
            let mut phones = parse_ph(p);
            phones.push((id("Z").unwrap_or(0), 0));
            return vec![spoken(phones, false)];
        }
    }
    let upper = tok.chars().any(|c| c.is_alphabetic()) && !tok.chars().any(|c| c.is_lowercase());
    if upper && (2..=5).contains(&tok.chars().count()) {
        return low
            .chars()
            .filter_map(letter_en)
            .map(|l| spoken(parse_ph(l), true))
            .collect();
    }
    if let Some(base) = low.strip_suffix('s') {
        if let Some(p) = d.get(base) {
            let mut phones = parse_ph(p);
            let last = phones.last().map(|&(p, _)| ph(p).name);
            let suffix = if matches!(last, Some("P" | "T" | "K" | "F" | "TH")) {
                "S"
            } else {
                "Z"
            };
            phones.push((id(suffix).unwrap_or(0), 0));
            return vec![spoken(phones, false)];
        }
    }
    let phones = lts(&low);
    if phones.is_empty() {
        Vec::new()
    } else {
        vec![spoken(phones, false)]
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
                .map(|Word::Spoken { ph: phones, .. }| {
                    phones
                        .iter()
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
