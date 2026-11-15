//! Strings from map files are untrusted: before they are kept or printed
//! they lose every character that a terminal could interpret or that could
//! disguise the text.

/// Characters replaced by [`clean`]: every control character (C0, DEL, C1,
/// so ESC and the 8-bit CSI cannot start an escape sequence, and no line
/// break can forge an extra `info` line), and the invisible formatting
/// characters that reorder or hide text (bidirectional overrides and
/// isolates, zero-width characters, the byte order mark).
pub fn is_unsafe(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{061C}'                // Arabic letter mark
            | '\u{200B}'..='\u{200F}' // zero-width space/joiners, LRM, RLM
            | '\u{2028}' | '\u{2029}' // line and paragraph separators
            | '\u{202A}'..='\u{202E}' // bidirectional embeddings and overrides
            | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
            | '\u{2066}'..='\u{2069}' // bidirectional isolates
            | '\u{FEFF}'              // byte order mark
            | '\u{FFF9}'..='\u{FFFB}' // interlinear annotation
        )
}

/// `s` with surrounding white space removed and every [`is_unsafe`]
/// character replaced by `?`.
pub fn clean(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| if is_unsafe(c) { '?' } else { c })
        .collect()
}

/// [`clean`], then cut to at most `max` characters (an ellipsis `...` marks
/// the cut). For messages that quote file content, such as SQLite errors.
pub fn clean_short(s: &str, max: usize) -> String {
    let c = clean(s);
    if c.chars().count() <= max {
        c
    } else {
        let mut out: String = c.chars().take(max.saturating_sub(3)).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sequences_and_line_breaks_are_replaced() {
        assert_eq!(clean("a\x1b[31mred\x1b[0m"), "a?[31mred?[0m");
        assert_eq!(clean("one\ntwo\rthree\tfour"), "one?two?three?four");
        assert_eq!(clean("\u{9b}2J"), "?2J");
        assert_eq!(clean("x\u{7f}y\0z"), "x?y?z");
    }

    #[test]
    fn invisible_reordering_characters_are_replaced() {
        assert_eq!(clean("abc\u{202E}fed"), "abc?fed");
        assert_eq!(clean("a\u{200B}b\u{2066}c\u{FEFF}"), "a?b?c?");
    }

    #[test]
    fn ordinary_text_is_kept() {
        assert_eq!(
            clean("  Île-de-France, Saint-Étienne  "),
            "Île-de-France, Saint-Étienne"
        );
        assert_eq!(clean("Αθήνα 東京 (c) OSM"), "Αθήνα 東京 (c) OSM");
    }

    #[test]
    fn short_messages_are_cut() {
        assert_eq!(clean_short("abcdef", 6), "abcdef");
        assert_eq!(clean_short("abcdefg", 6), "abc...");
        assert_eq!(clean_short("\x1b\x1b\x1b", 10), "???");
    }
}
