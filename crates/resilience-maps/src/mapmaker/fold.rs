//! `rs_place.fold`: a name folded for comparison (ADR 0016 §4, ranking
//! rule 1, "exact match on the folded name").
//!
//! This is not `resilience_core::maps::label::fold`, which folds for
//! *display*: it keeps upper case, cuts at 40 characters with `...`, and
//! turns every non-Latin run into one `?`, so all Cyrillic or Greek names
//! of the same length class would compare equal and long names would
//! compare by their first 37 characters. For comparison we need:
//!
//! - lower case;
//! - diacritics removed, using the display table for Latin letters
//!   (`é` -> `e`, `ß` -> `ss`, `Ø` -> `o`), so the two agree on Latin text;
//! - letters of other scripts kept (lower-cased), not replaced;
//! - words separated by exactly one space, splitting where SQLite's
//!   `unicode61` tokenizer of `rs_place_fts` splits (at every character
//!   that is not a letter or a digit), so "Rue de l'Essai" folds to
//!   "rue de l essai" and an exact match agrees with the FTS tokens;
//! - no length cut beyond the name's own bound.
//!
//! The search slice must fold the user's words with this same function
//! (it belongs in `resilience_core::maps::search` once that exists).

use resilience_core::maps::label::fold_to;

fn combining(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// Fold `name` for comparison (see the module documentation).
pub fn search_fold(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut sep = false;
    let mut buf = [0u8; 4];
    for c in name.chars() {
        if combining(c) {
            continue;
        }
        if !c.is_alphanumeric() {
            sep = true;
            continue;
        }
        // The display fold of one Latin letter is its ASCII letters; for
        // any other letter it is "?" (or nothing), and the letter is kept.
        let ascii = if c.is_ascii() {
            None
        } else {
            fold_to(c.encode_utf8(&mut buf), 8)
                .filter(|s| s.bytes().all(|b| b.is_ascii_alphanumeric()))
        };
        if sep && !out.is_empty() {
            out.push(' ');
        }
        sep = false;
        match ascii {
            Some(s) => out.push_str(&s.to_ascii_lowercase()),
            None => out.extend(c.to_lowercase()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_names_lose_case_accents_and_punctuation() {
        assert_eq!(search_fold("Rue de l'Essai"), "rue de l essai");
        assert_eq!(search_fold("Esch-sur-Alzette"), "esch sur alzette");
        assert_eq!(search_fold("  Île-de-France "), "ile de france");
        assert_eq!(search_fold("Straße"), "strasse");
        assert_eq!(search_fold("ØRESUND"), "oresund");
        assert_eq!(search_fold("Œuvre"), "oeuvre");
        assert_eq!(search_fold("Saint-Étienne"), search_fold("SAINT ETIENNE"));
        // A decomposed accent (e + U+0301) is the same as the composed one.
        assert_eq!(search_fold("Caf\u{65}\u{301}"), search_fold("Café"));
        assert_eq!(search_fold("A1 / N7"), "a1 n7");
    }

    #[test]
    fn other_scripts_are_kept_not_replaced() {
        assert_eq!(search_fold("Москва"), "москва");
        assert_ne!(search_fold("Москва"), search_fold("Казань"));
        assert_eq!(search_fold("Αθήνα"), "αθήνα");
        assert_eq!(search_fold("東京 駅"), "東京 駅");
    }

    #[test]
    fn nothing_left_gives_an_empty_fold() {
        assert_eq!(search_fold(""), "");
        assert_eq!(search_fold("--- ..."), "");
        let long = "Ab ".repeat(100);
        assert_eq!(search_fold(&long).len(), 299);
    }
}
