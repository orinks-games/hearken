//! A light British accent over genny's American phonemes: no r after a
//! vowel, a rounded LOT vowel, NURSE without r-colouring and a central
//! start to GOAT. Not genny's; added for Hearken.

use crate::phonemes::{id, ph};
use crate::text::{Phone, Phrase, Word};

fn swap(name: &str) -> usize {
    id(name).expect("the British vowels are in the inventory")
}

/// Rewrite one word's phonemes into the British accent.
pub fn british_word(phones: &[Phone]) -> Vec<Phone> {
    let r = id("R");
    let mut out: Vec<Phone> = Vec::with_capacity(phones.len());
    for (i, &(p, st)) in phones.iter().enumerate() {
        let name = ph(p).name;
        let next_is_vowel = phones
            .get(i + 1)
            .is_some_and(|&(q, _)| ph(q).kind.vowelish());
        if Some(p) == r && !next_is_vowel {
            // Non-rhotic: an r before a consonant or at the end of the word
            // goes, leaving a schwa glide after the front and close vowels.
            if let Some(&(prev, pst)) = out.last() {
                match ph(prev).name {
                    "IH" | "IY" | "EH" | "EY" | "UH" | "UW" | "AY" | "AW" => {
                        out.push((swap("AX"), 0))
                    }
                    "AA" | "AO" | "OH" if pst == 0 => {
                        // An unstressed "-or", "-ar" ("manor") is a schwa.
                        let last = out.len() - 1;
                        out[last] = (swap("AX"), 0);
                    }
                    _ => {}
                }
            }
            continue;
        }
        let next_is_r = phones.get(i + 1).is_some_and(|&(q, _)| Some(q) == r);
        let mapped = match name {
            "ER" => swap("ERB"),
            "AXR" => swap("AX"),
            "OW" => swap("OWB"),
            // LOT: "hot", "stop"; "car" keeps its open AA.
            "AA" if !next_is_r => swap("OH"),
            _ => p,
        };
        out.push((mapped, st));
    }
    out
}

pub fn british(phrases: &mut [Phrase]) {
    for phrase in phrases {
        for Word::Spoken { ph: phones, .. } in phrase.words.iter_mut() {
            *phones = british_word(phones);
        }
    }
}
