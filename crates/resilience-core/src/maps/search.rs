//! Place search in map packs (ADR 0016 §4): the user's words to an FTS5
//! query, the folding shared with the pack builder, and the ranking of
//! results from several packs. Pure: the queries run in the viewer's
//! reader (`resilience-maps`).
//!
//! **User text never reaches FTS5's query syntax.** [`tokens`] keeps only
//! letters and digits (everything else separates words, as SQLite's
//! `unicode61` tokenizer does), at most [`MAX_TOKENS`] words of at most
//! [`MAX_TOKEN_CHARS`] characters; [`fts_query`] quotes each one and gives
//! the last a `*` for a prefix match. Quotes, `*`, `^`, `-`, `:`,
//! parentheses, `NEAR`, `AND`/`OR`/`NOT` typed by the user are therefore
//! either separators or plain quoted words.

use super::geo::{Compass, LatLon, distance_m, format_distance, initial_bearing};
use super::label::fold_ascii_to;

/// At most this many words are searched for.
pub const MAX_TOKENS: usize = 8;
/// Longer words are cut to this many characters.
pub const MAX_TOKEN_CHARS: usize = 32;
/// At most this many characters of the input are read.
pub const MAX_INPUT_CHARS: usize = 200;
/// Rows asked of each pack.
pub const PER_PACK: usize = 50;
/// Results shown after merging.
pub const MAX_RESULTS: usize = 100;

fn combining(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// `rs_place.fold`: a name folded for comparison (ranking rule 1, "exact
/// match on the folded name"), written by the pack builder and computed
/// here from the user's words the same way.
///
/// This is not [`super::label::fold`], which folds for *display*: it keeps
/// upper case and the console font's accented letters, cuts at 40
/// characters and turns every non-Latin run into one `?`. For comparison:
///
/// - lower case;
/// - diacritics removed with the ASCII table for Latin letters
///   ([`super::label::fold_ascii_to`])
///   (`é` -> `e`, `ß` -> `ss`, `Ø` -> `o`);
/// - letters of other scripts kept (lower-cased), not replaced;
/// - words separated by exactly one space, splitting where `unicode61`
///   splits (every character that is not a letter or a digit), so "Rue de
///   l'Essai" folds to "rue de l essai";
/// - no length cut beyond the name's own bound.
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
        // The ASCII fold of one Latin letter is its ASCII letters; for
        // any other letter it is "?" (or nothing), and the letter is kept.
        let ascii = if c.is_ascii() {
            None
        } else {
            fold_ascii_to(c.encode_utf8(&mut buf), 8)
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

/// The words to search for: the folded input split at spaces, at most
/// [`MAX_TOKENS`] of at most [`MAX_TOKEN_CHARS`] characters, each made
/// only of letters and digits.
pub fn tokens(input: &str) -> Vec<String> {
    let head: String = input.chars().take(MAX_INPUT_CHARS).collect();
    search_fold(&head)
        .split(' ')
        .filter(|w| !w.is_empty())
        .take(MAX_TOKENS)
        .map(|w| w.chars().take(MAX_TOKEN_CHARS).collect())
        .collect()
}

/// The FTS5 `MATCH` expression for `tokens`: every word quoted (they hold
/// only letters and digits, so nothing needs escaping), the last one a
/// prefix. `None` when there is nothing to search for.
pub fn fts_query(tokens: &[String]) -> Option<String> {
    let last = tokens.len().checked_sub(1)?;
    let parts: Vec<String> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.chars().all(char::is_alphanumeric) && !t.is_empty())
        .map(|(i, t)| {
            if i == last {
                format!("\"{t}\"*")
            } else {
                format!("\"{t}\"")
            }
        })
        .collect();
    (parts.len() == tokens.len()).then(|| parts.join(" "))
}

/// What a result is (`rs_place.kind`), in ranking order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Settlement,
    Street,
    Poi,
    Natural,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "settlement" => Some(Kind::Settlement),
            "street" => Some(Kind::Street),
            "poi" => Some(Kind::Poi),
            "natural" => Some(Kind::Natural),
            _ => None,
        }
    }

    /// The scale (metres per pixel) at which the map shows a result of
    /// this kind and class.
    pub fn scale(self, class: &str) -> f64 {
        match (self, class) {
            (Kind::Settlement, "city") => 50.0,
            (Kind::Settlement, "town") => 20.0,
            (Kind::Settlement, _) => 10.0,
            (Kind::Natural, _) => 20.0,
            _ => 5.0,
        }
    }
}

/// One row found in one pack.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// The viewer's index of the pack it comes from.
    pub pack: usize,
    pub kind: Kind,
    /// `town`, `pharmacy`, `residential`... (untrusted: shown folded).
    pub class: String,
    pub name: String,
    pub fold: String,
    pub near: Option<String>,
    pub at: LatLon,
    /// The builder's rank within the kind (smaller is more important).
    pub rank: i64,
}

/// Merge the rows of every pack and order them (ADR 0016 §4): an exact
/// match on the folded name first, then by kind, rank and distance from
/// `centre`; the same place found in two packs once; at most
/// [`MAX_RESULTS`].
pub fn rank(mut hits: Vec<Hit>, query: &str, centre: LatLon) -> Vec<Hit> {
    let q = search_fold(query);
    hits.sort_by(|a, b| {
        let exact = |h: &Hit| h.fold != q;
        exact(a)
            .cmp(&exact(b))
            .then(a.kind.cmp(&b.kind))
            .then(a.rank.cmp(&b.rank))
            .then(
                distance_m(centre, a.at)
                    .partial_cmp(&distance_m(centre, b.at))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.name.cmp(&b.name))
    });
    let mut out: Vec<Hit> = Vec::new();
    // Every row looked at, kept or not: the pieces of one road form a chain
    // (each near the next), so a piece is compared with all of them.
    let mut seen: Vec<(String, Kind, LatLon)> = Vec::new();
    for h in hits {
        let reach = same_place_m(h.kind);
        let dup = seen
            .iter()
            .any(|(f, k, at)| *f == h.fold && *k == h.kind && distance_m(*at, h.at) < reach);
        if seen.len() < 4 * MAX_RESULTS {
            seen.push((h.fold.clone(), h.kind, h.at));
        }
        if !dup {
            out.push(h);
        }
        if out.len() >= MAX_RESULTS {
            break;
        }
    }
    out
}

/// Two rows of one kind with the same name closer than this are one place.
/// A long road is mapped as many pieces, each a row near its own hamlet:
/// streets are merged over 2 km (same-name streets of two villages are
/// normally further apart); other kinds over 200 m (a place found in two
/// packs).
pub const SAME_STREET_M: f64 = 2_000.0;
pub const SAME_PLACE_M: f64 = 200.0;

fn same_place_m(kind: Kind) -> f64 {
    match kind {
        Kind::Street => SAME_STREET_M,
        _ => SAME_PLACE_M,
    }
}

/// "15 km SW" from `centre` (or "here" closer than a metre).
pub fn where_from(centre: LatLon, at: LatLon) -> String {
    let d = distance_m(centre, at);
    match initial_bearing(centre, at).and_then(Compass::from_bearing) {
        Some(c) => format!("{} {c}", format_distance(d)),
        None => "here".into(),
    }
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

    #[test]
    fn user_text_never_reaches_the_query_syntax() {
        let q = |s: &str| fts_query(&tokens(s));
        assert_eq!(q("Esch Alz").as_deref(), Some("\"esch\" \"alz\"*"));
        assert_eq!(
            q("Saint-Étienne").as_deref(),
            Some("\"saint\" \"etienne\"*")
        );
        for hostile in [
            "\"",
            "x\" OR \"y",
            "a*",
            "^a",
            "-a",
            "name:a",
            "NEAR(a b)",
            "a AND b",
            "(a)",
            "a\"*NOT b",
            "{a b}",
            "a + b",
            "'",
            "\0",
            "a\u{202e}b",
        ] {
            if let Some(f) = q(hostile) {
                // Only quoted words of letters and digits, the last with `*`.
                for part in f.split(' ') {
                    let inner = part
                        .strip_prefix('"')
                        .and_then(|p| p.strip_suffix('"').or_else(|| p.strip_suffix("\"*")))
                        .unwrap_or_else(|| panic!("{hostile:?} gave {f:?}"));
                    assert!(
                        inner.chars().all(char::is_alphanumeric),
                        "{hostile:?} gave {f:?}"
                    );
                }
            }
        }
        // Operators typed as words are only words.
        assert_eq!(q("a AND b").as_deref(), Some("\"a\" \"and\" \"b\"*"));
        assert_eq!(q("").as_deref(), None);
        assert_eq!(q("--- ''").as_deref(), None);
        assert_eq!(tokens(&"w ".repeat(50)).len(), MAX_TOKENS);
        assert_eq!(tokens(&"x".repeat(100))[0].chars().count(), MAX_TOKEN_CHARS);
        assert!(tokens(&"y".repeat(10_000))[0].len() <= MAX_TOKEN_CHARS);
    }

    fn hit(kind: Kind, name: &str, rank: i64, lat: f64, lon: f64) -> Hit {
        Hit {
            pack: 0,
            kind,
            class: "x".into(),
            name: name.into(),
            fold: search_fold(name),
            near: None,
            at: LatLon::new(lat, lon).expect("in range"),
            rank,
        }
    }

    #[test]
    fn pieces_of_one_road_are_one_result() {
        let c = LatLon::new(48.0, -4.1).expect("c");
        // Five pieces of one road over 4 km (about 1 km apart), the same
        // name 8 km away (another village), and a town of that name.
        let mut v: Vec<Hit> = (0..5)
            .map(|i| {
                hit(
                    Kind::Street,
                    "Route de Quimper",
                    0,
                    48.0 + 0.009 * f64::from(i),
                    -4.0,
                )
            })
            .collect();
        v.push(hit(Kind::Street, "Route de Quimper", 0, 48.1, -4.0));
        v.push(hit(Kind::Settlement, "Route de Quimper", 0, 48.0, -4.0));
        let got = rank(v, "quimper", c);
        let streets = got.iter().filter(|h| h.kind == Kind::Street).count();
        assert_eq!(streets, 2, "{got:?}");
        assert_eq!(got.len(), 3);
        // Other kinds still merge only when (almost) at the same place.
        let towns = vec![
            hit(Kind::Settlement, "Kirchberg", 0, 49.62, 6.15),
            hit(Kind::Settlement, "Kirchberg", 0, 49.63, 6.15),
        ];
        assert_eq!(rank(towns, "kirchberg", c).len(), 2);
    }

    #[test]
    fn ranking_exact_kind_rank_distance() {
        let centre = LatLon::new(49.6, 6.1).expect("c");
        let got = rank(
            vec![
                hit(Kind::Poi, "Esch Pharmacy", 1, 49.5, 6.0),
                hit(Kind::Street, "Rue d'Esch", 1, 49.61, 6.1),
                hit(Kind::Settlement, "Esch-sur-Sûre", 3, 49.9, 5.9),
                hit(Kind::Settlement, "Esch-sur-Alzette", 2, 49.5, 5.98),
                hit(Kind::Poi, "Esch", 5, 50.5, 7.0),
            ],
            "esch",
            centre,
        );
        let names: Vec<&str> = got.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Esch",
                "Esch-sur-Alzette",
                "Esch-sur-Sûre",
                "Rue d'Esch",
                "Esch Pharmacy"
            ]
        );
        // The same place in two packs (overview and detailed) once.
        let mut a = hit(Kind::Settlement, "Testville", 1, 49.6, 6.1);
        let mut b = a.clone();
        b.pack = 1;
        b.at = LatLon::new(49.6005, 6.1).expect("b");
        a.pack = 0;
        assert_eq!(rank(vec![a, b], "test", centre).len(), 1);
    }

    #[test]
    fn where_and_how_far() {
        let c = LatLon::new(49.6, 6.1).expect("c");
        assert_eq!(where_from(c, c), "here");
        let s = where_from(c, LatLon::new(49.5, 5.98).expect("p"));
        assert!(s.ends_with(" SW"), "{s}");
        assert!(s.contains("km"), "{s}");
        assert_eq!(Kind::parse("street"), Some(Kind::Street));
        assert_eq!(Kind::parse("STREET"), None);
        assert!(Kind::Settlement.scale("city") > Kind::Poi.scale("pharmacy"));
    }
}
