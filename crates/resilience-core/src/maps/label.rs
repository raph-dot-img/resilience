//! Map labels: folding untrusted names to what the console can show, and
//! greedy placement (ADR 0016 §3).
//!
//! Names come from map files, which are untrusted. [`fold`] turns any
//! string into printable ASCII plus the accented letters the console font
//! has ([`CONSOLE_LETTERS`]: "Orléans", "Zürich"; ADR 0016, "Decided while
//! showing accented names"): control and formatting characters are
//! removed, other accented Latin letters and ligatures become their ASCII
//! letters ("Łódź" is "Lodz"), and every run of other characters becomes
//! one `?`. Attribution strings go through the same function (`©` becomes
//! `(c)`). [`fold_ascii_to`] is the same without the kept letters (place
//! search compares in ASCII).
//!
//! [`place`] puts labels on the screen in priority order without overlap,
//! and [`street_anchor`] finds where a street name fits along its line.

use super::canvas::{CONSOLE_LETTERS, Canvas, Colour, MAX_COLS, MAX_ROWS, Point};

/// Longest label, in characters (ADR 0016 §3).
pub const MAX_LABEL: usize = 40;

/// At most this many candidates are considered, highest priority first.
pub const MAX_CANDIDATES: usize = 4096;

/// [`fold_to`] with the label length cap.
pub fn fold(name: &str) -> Option<String> {
    fold_to(name, MAX_LABEL)
}

/// Fold untrusted text to what the console shows, at most `max`
/// characters (longer text is cut and ends with `...`). Every character of
/// the result is one cell wide: printable ASCII or one of
/// [`CONSOLE_LETTERS`].
///
/// - whitespace of any kind becomes one space; leading and trailing
///   spaces are removed;
/// - control characters, invisible formatting characters (bidirectional
///   overrides, zero-width characters, soft hyphen, byte order mark) and
///   combining marks are dropped, so a decomposed `e` + acute is `e`;
/// - the accented letters of [`CONSOLE_LETTERS`] are kept (é, ü, ß, æ...);
/// - other Latin-1, Latin Extended-A, a few Latin Extended-B letters,
///   fullwidth ASCII and common punctuation are folded by a table (È→E,
///   Ø→O, Ł→L, Œ→OE, ©→(c), “→");
/// - any other run of characters becomes a single `?`, so the reader can
///   see that part of the name could not be shown (Cyrillic, Greek, CJK,
///   right-to-left scripts). The builder prefers `name:latin` or
///   `name:en` for such names.
///
/// Returns `None` when nothing worth showing is left: no letter or digit
/// (an empty name, only `?`, only punctuation). The cost is bounded by
/// `max`, not by the input length.
pub fn fold_to(text: &str, max: usize) -> Option<String> {
    fold_with(text, max, true)
}

/// [`fold_to`] to printable ASCII only: every accented Latin letter is
/// folded by the table (é→e, ß→ss, Æ→AE). Used for comparison
/// (`search::search_fold`), which must match what the pack builder wrote.
pub fn fold_ascii_to(text: &str, max: usize) -> Option<String> {
    fold_with(text, max, false)
}

fn fold_with(text: &str, max: usize, keep_letters: bool) -> Option<String> {
    let max = max.max(4);
    let mut out = String::new();
    // Characters in `out` (not bytes: kept letters are two bytes long).
    let mut count = 0usize;
    let mut unknown_run = false;
    // Characters consumed from the input, so a huge string of dropped
    // characters cannot cost more than this.
    let budget = max.saturating_mul(16).max(1024);
    for (n, c) in text.chars().enumerate() {
        if n >= budget || count > max {
            break;
        }
        let f = if keep_letters && CONSOLE_LETTERS.contains(c) {
            Fold::Char(c)
        } else {
            fold_char(c)
        };
        match f {
            Fold::Drop => continue,
            Fold::Unknown => {
                if !unknown_run {
                    out.push('?');
                    count += 1;
                }
                unknown_run = true;
                continue;
            }
            Fold::Space => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                    count += 1;
                }
            }
            Fold::Char(ch) => {
                out.push(ch);
                count += 1;
            }
            Fold::Str(s) => {
                // Table entries are ASCII.
                out.push_str(s);
                count += s.len();
            }
        }
        unknown_run = false;
    }
    let mut out = out.trim_end().to_string();
    if out.chars().count() > max {
        let mut cut: String = out.chars().take(max - 3).collect();
        cut.truncate(cut.trim_end().len());
        cut.push_str("...");
        out = cut;
    }
    debug_assert!(out.chars().all(|c| c == ' '
        || c.is_ascii_graphic()
        || (keep_letters && CONSOLE_LETTERS.contains(c))));
    if out.chars().any(char::is_alphanumeric) {
        Some(out)
    } else {
        None
    }
}

enum Fold {
    Char(char),
    Str(&'static str),
    Space,
    Drop,
    Unknown,
}

fn fold_char(c: char) -> Fold {
    use Fold::{Char, Drop, Space, Str, Unknown};
    if c.is_whitespace() {
        return Space;
    }
    if c.is_ascii_graphic() {
        return Char(c);
    }
    if c.is_control() {
        return Drop;
    }
    let s: &'static str = match c {
        // Invisible formatting: soft hyphen, zero-width and bidirectional
        // controls, word joiners, BOM, Arabic letter mark, Mongolian vowel
        // separator, variation selectors.
        '\u{AD}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}'
        | '\u{061C}'
        | '\u{180E}'
        | '\u{FE00}'..='\u{FE0F}' => {
            return Drop;
        }
        // Combining marks.
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}' => return Drop,
        // Fullwidth ASCII.
        '\u{FF01}'..='\u{FF5E}' => {
            return char::from_u32(c as u32 - 0xFEE0).map_or(Unknown, Char);
        }
        // Latin-1 Supplement.
        '¡' => "!",
        '¢' => "c",
        '£' => "GBP",
        '¥' => "JPY",
        '¦' => "|",
        '§' => "S",
        '©' => "(c)",
        'ª' => "a",
        '«' | '»' => "\"",
        '¬' => "-",
        '®' => "(R)",
        '¯' => "-",
        '°' => "deg",
        '±' => "+/-",
        '²' => "2",
        '³' => "3",
        '´' => "'",
        'µ' => "u",
        '·' => ".",
        '¹' => "1",
        'º' => "o",
        '¼' => "1/4",
        '½' => "1/2",
        '¾' => "3/4",
        '¿' => "?",
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "A",
        'Æ' => "AE",
        'Ç' => "C",
        'È' | 'É' | 'Ê' | 'Ë' => "E",
        'Ì' | 'Í' | 'Î' | 'Ï' => "I",
        'Ð' => "D",
        'Ñ' => "N",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => "O",
        '×' => "x",
        'Ù' | 'Ú' | 'Û' | 'Ü' => "U",
        'Ý' => "Y",
        'Þ' => "TH",
        'ß' => "ss",
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
        'æ' => "ae",
        'ç' => "c",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ð' => "d",
        'ñ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => "o",
        '÷' => "/",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'ý' | 'ÿ' => "y",
        'þ' => "th",
        // Latin Extended-A.
        'Ā' | 'Ă' | 'Ą' => "A",
        'ā' | 'ă' | 'ą' => "a",
        'Ć' | 'Ĉ' | 'Ċ' | 'Č' => "C",
        'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'Ď' | 'Đ' => "D",
        'ď' | 'đ' => "d",
        'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => "E",
        'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => "G",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'Ĥ' | 'Ħ' => "H",
        'ĥ' | 'ħ' => "h",
        'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'İ' => "I",
        'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'Ĳ' => "IJ",
        'ĳ' => "ij",
        'Ĵ' => "J",
        'ĵ' => "j",
        'Ķ' => "K",
        'ķ' | 'ĸ' => "k",
        'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => "L",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'Ń' | 'Ņ' | 'Ň' | 'Ŋ' => "N",
        'ń' | 'ņ' | 'ň' | 'ŋ' => "n",
        'ŉ' => "'n",
        'Ō' | 'Ŏ' | 'Ő' => "O",
        'ō' | 'ŏ' | 'ő' => "o",
        'Œ' => "OE",
        'œ' => "oe",
        'Ŕ' | 'Ŗ' | 'Ř' => "R",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'Ś' | 'Ŝ' | 'Ş' | 'Š' => "S",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ſ' => "s",
        'Ţ' | 'Ť' | 'Ŧ' => "T",
        'ţ' | 'ť' | 'ŧ' => "t",
        'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => "U",
        'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'Ŵ' => "W",
        'ŵ' => "w",
        'Ŷ' | 'Ÿ' => "Y",
        'ŷ' => "y",
        'Ź' | 'Ż' | 'Ž' => "Z",
        'ź' | 'ż' | 'ž' => "z",
        // Latin Extended-B letters used in European place names.
        'Ș' => "S",
        'ș' => "s",
        'Ț' => "T",
        'ț' => "t",
        'Ə' => "E",
        'ə' => "e",
        'Ơ' => "O",
        'ơ' => "o",
        'Ư' => "U",
        'ư' => "u",
        'ƒ' => "f",
        'Ǆ' | 'ǅ' => "Dz",
        'ǆ' => "dz",
        'Ǉ' | 'ǈ' => "Lj",
        'ǉ' => "lj",
        'Ǌ' | 'ǋ' => "Nj",
        'ǌ' => "nj",
        // Punctuation and symbols.
        '\u{2010}'..='\u{2015}' | '\u{2212}' => "-",
        '\u{2018}'..='\u{201B}' | '\u{2032}' => "'",
        '\u{201C}'..='\u{201F}' | '\u{2033}' => "\"",
        '\u{2039}' => "<",
        '\u{203A}' => ">",
        '\u{2022}' => "*",
        '\u{2026}' => "...",
        '\u{20AC}' => "EUR",
        '\u{2116}' => "No",
        '\u{2122}' => "TM",
        _ => return Unknown,
    };
    Str(s)
}

/// A label the viewer would like to show.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// Higher is placed first (`style::LabelRule::priority`).
    pub priority: u16,
    /// The cell of the feature (the marker's cell). For a label without a
    /// marker, the text is centred on it.
    pub col: i32,
    pub row: i32,
    /// Drawn at the anchor (`*`, `^`, a POI letter).
    pub marker: Option<char>,
    /// The text, already folded with [`fold`]; may be empty.
    pub text: String,
    pub colour: Colour,
    /// Place the marker alone when no position fits the text.
    pub keep_marker: bool,
}

/// A rectangle of cells nothing may cover (the crosshair, status lines).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub col: u16,
    pub row: u16,
    pub width: u16,
    pub height: u16,
}

/// A placed label: draw it with [`draw`].
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// Index of the candidate in the slice given to [`place`].
    pub index: usize,
    pub marker: Option<(u16, u16, char)>,
    /// Column, row and text.
    pub text: Option<(u16, u16, String)>,
    pub colour: Colour,
}

/// Where the text goes relative to the anchor: (row offset, start column
/// rule). Tried in this order.
#[derive(Clone, Copy)]
enum Start {
    /// The column right of the anchor.
    Right,
    /// Ending in the column left of the anchor.
    Left,
    /// Centred on the anchor.
    Centre,
}

const WITH_MARKER: [(i32, Start); 8] = [
    (0, Start::Right),
    (0, Start::Left),
    (-1, Start::Centre),
    (1, Start::Centre),
    (-1, Start::Right),
    (1, Start::Right),
    (-1, Start::Left),
    (1, Start::Left),
];

const WITHOUT_MARKER: [(i32, Start); 3] =
    [(0, Start::Centre), (-1, Start::Centre), (1, Start::Centre)];

struct Grid {
    cols: i32,
    rows: i32,
    reserved: Vec<bool>,
    used: Vec<bool>,
}

impl Grid {
    fn idx(&self, col: i32, row: i32) -> Option<usize> {
        if col >= 0 && row >= 0 && col < self.cols && row < self.rows {
            Some(row as usize * self.cols as usize + col as usize)
        } else {
            None
        }
    }

    /// The span `col..col+len` on `row` is on the canvas, not reserved,
    /// and at least one cell away from every placed label.
    fn free(&self, col: i32, row: i32, len: i32) -> bool {
        if len <= 0
            || len > self.cols
            || col < 0
            || row < 0
            || row >= self.rows
            || col > self.cols - len
        {
            return false;
        }
        for c in col..col + len {
            if self.idx(c, row).is_none_or(|i| self.reserved[i]) {
                return false;
            }
        }
        for r in row - 1..=row + 1 {
            for c in col - 1..=col + len {
                if self.idx(c, r).is_some_and(|i| self.used[i]) {
                    return false;
                }
            }
        }
        true
    }

    fn take(&mut self, col: i32, row: i32, len: i32) {
        for c in col..col + len {
            if let Some(i) = self.idx(c, row) {
                self.used[i] = true;
            }
        }
    }
}

/// Place labels greedily on a canvas of `cols` x `rows` cells.
///
/// - candidates are taken by priority (highest first), then by row,
///   column, text and marker, so equal priorities give the same result
///   whatever the input order; at most [`MAX_CANDIDATES`] are considered;
/// - a label never overlaps another, keeps one empty cell around itself
///   (the margin) and never covers a reserved cell;
/// - a label with a marker tries 8 positions around it (right, left,
///   above, below, then the corners); without a marker, centred on the
///   anchor, then one row above or below;
/// - a label must fit entirely on the canvas: a position that would cut
///   it at an edge is not used (a cut name can read as another name), and
///   the next position is tried, so a label near the right edge goes to
///   the left of its marker;
/// - when no position fits, the marker alone is placed if the candidate
///   says so, otherwise the candidate is dropped;
/// - a letter or digit marker (a point of interest) is kept one blank cell
///   away from its text, so that it does not read as part of the name
///   (`o Bakery`, not `oBakery`); symbols (`*`, `^`) touch their text;
/// - a marker that falls on a reserved cell (the crosshair) is not drawn,
///   but its text is still placed around it, so the place at the centre
///   of the view keeps its name;
/// - a text already placed for an anchor less than [`SAME_NAME_COLS`]
///   columns and [`SAME_NAME_ROWS`] rows away is not placed again (one
///   place mapped twice, as a node and an area, gives one name).
///
/// Cost: O(n) to select, O(k log k) to sort the k considered, and O(text
/// length) per position tried.
pub fn place(candidates: &[Candidate], cols: u16, rows: u16, reserved: &[CellRect]) -> Vec<Placed> {
    // Anchors and texts placed so far, for the same-name rule.
    let mut names: Vec<(i32, i32, &str)> = Vec::new();
    let near_same = |names: &[(i32, i32, &str)], c: &Candidate| {
        !c.text.is_empty()
            && names.iter().any(|&(col, row, t)| {
                t == c.text
                    && (col - c.col).abs() < SAME_NAME_COLS
                    && (row - c.row).abs() < SAME_NAME_ROWS
            })
    };
    // Same limits as the canvas.
    let cols = cols.min(MAX_COLS);
    let rows = rows.min(MAX_ROWS);
    let mut grid = Grid {
        cols: i32::from(cols),
        rows: i32::from(rows),
        reserved: vec![false; usize::from(cols) * usize::from(rows)],
        used: vec![false; usize::from(cols) * usize::from(rows)],
    };
    for r in reserved {
        for row in r.row..r.row.saturating_add(r.height).min(rows) {
            for col in r.col..r.col.saturating_add(r.width).min(cols) {
                if let Some(i) = grid.idx(i32::from(col), i32::from(row)) {
                    grid.reserved[i] = true;
                }
            }
        }
    }

    let mut order: Vec<usize> = (0..candidates.len())
        .filter(|&i| grid.idx(candidates[i].col, candidates[i].row).is_some())
        .collect();
    let key = |i: &usize| {
        let c = &candidates[*i];
        (
            std::cmp::Reverse(c.priority),
            c.row,
            c.col,
            c.text.as_str(),
            c.marker,
        )
    };
    if order.len() > MAX_CANDIDATES {
        order.select_nth_unstable_by(MAX_CANDIDATES - 1, |a, b| key(a).cmp(&key(b)));
        order.truncate(MAX_CANDIDATES);
    }
    order.sort_unstable_by(|a, b| key(a).cmp(&key(b)));

    let mut placed = Vec::new();
    for i in order {
        let c = &candidates[i];
        let len = i32::try_from(c.text.chars().count()).unwrap_or(i32::MAX);
        let (ac, ar) = (c.col, c.row);
        if near_same(&names, c) {
            continue;
        }
        if let Some(m) = c.marker {
            let hidden = grid.idx(ac, ar).is_some_and(|i| grid.reserved[i]);
            if !hidden && !grid.free(ac, ar, 1) {
                continue;
            }
            let gap = i32::from(m.is_ascii_alphanumeric());
            let mut text_at = None;
            if len > 0 {
                for (dr, start) in WITH_MARKER {
                    let col = start_col(ac, len, start, gap);
                    let row = ar + dr;
                    // The marker and the text are one label: the text may
                    // touch its own marker, which is not placed yet.
                    if grid.free(col, row, len) {
                        text_at = Some((col, row));
                        break;
                    }
                }
                if text_at.is_none() && (hidden || !c.keep_marker) {
                    continue;
                }
            }
            if hidden && len == 0 {
                continue;
            }
            if !hidden {
                grid.take(ac, ar, 1);
            }
            let text = text_at.map(|(col, row)| {
                grid.take(col, row, len);
                names.push((ac, ar, c.text.as_str()));
                (col as u16, row as u16, c.text.clone())
            });
            placed.push(Placed {
                index: i,
                marker: (!hidden).then_some((ac as u16, ar as u16, m)),
                text,
                colour: c.colour,
            });
        } else if len > 0 {
            for (dr, start) in WITHOUT_MARKER {
                let col = start_col(ac, len, start, 0);
                let row = ar + dr;
                if grid.free(col, row, len) {
                    grid.take(col, row, len);
                    names.push((ac, ar, c.text.as_str()));
                    placed.push(Placed {
                        index: i,
                        marker: None,
                        text: Some((col as u16, row as u16, c.text.clone())),
                        colour: c.colour,
                    });
                    break;
                }
            }
        }
    }
    placed
}

/// Same text within this many columns (exclusive) of a placed one: skipped.
pub const SAME_NAME_COLS: i32 = 16;
/// ... and within this many rows (exclusive).
pub const SAME_NAME_ROWS: i32 = 4;

/// First column of a text of `len` cells at `start` of the anchor, `gap`
/// blank cells away from it on the sides.
fn start_col(anchor: i32, len: i32, start: Start, gap: i32) -> i32 {
    match start {
        Start::Right => anchor.saturating_add(1 + gap),
        Start::Left => anchor.saturating_sub(len + gap),
        Start::Centre => anchor.saturating_sub((len - 1) / 2),
    }
}

/// Draw placed labels: the text in its colour on the default background.
pub fn draw(canvas: &mut Canvas, placed: &[Placed]) {
    for p in placed {
        if let Some((col, row, m)) = p.marker {
            canvas.put_char(
                i64::from(col),
                i64::from(row),
                m,
                Some(p.colour),
                None,
                false,
            );
        }
        if let Some((col, row, text)) = &p.text {
            canvas.put_text(
                i64::from(*col),
                i64::from(*row),
                text,
                Some(p.colour),
                None,
                false,
            );
        }
    }
}

/// A nearly horizontal stretch may rise or fall by at most this many
/// pixels in all (one cell row is two pixels).
const STREET_MAX_DY: f64 = 2.0;

/// Where to centre a street name of `text_len` characters along a line
/// given in pixel coordinates on a canvas of `cols` x `rows` cells: the
/// middle of the longest nearly horizontal stretch (consecutive segments
/// going the same way across, staying within 2 pixels vertically), counting
/// only its part on the canvas. The stretch must be at least `text_len + 2`
/// cells long, so a bit of the street shows on each side of its name;
/// otherwise `None`. Points that are not finite break the line. Cost:
/// O(points).
pub fn street_anchor(
    points: &[Point],
    text_len: usize,
    cols: u16,
    rows: u16,
) -> Option<(i32, i32)> {
    let w = f64::from(cols);
    let h = f64::from(rows) * 2.0;
    let need = text_len as f64 + 2.0;
    let mut best: Option<(f64, i32, i32)> = None;

    // The current stretch: x range, y range, direction.
    struct Run {
        x0: f64,
        x1: f64,
        ylo: f64,
        yhi: f64,
        dir: f64,
    }
    let mut consider = |run: &Run| {
        let lo = run.x0.min(run.x1).max(0.0);
        let hi = run.x0.max(run.x1).min(w);
        let ymid = (run.ylo + run.yhi) / 2.0;
        let visible = hi - lo;
        if visible >= need && ymid >= 0.0 && ymid < h && best.as_ref().is_none_or(|b| visible > b.0)
        {
            // Bounded by the canvas, so the conversions are exact.
            best = Some((
                visible,
                ((lo + hi) / 2.0).floor() as i32,
                (ymid / 2.0).floor() as i32,
            ));
        }
    };

    let mut run: Option<Run> = None;
    for seg in points.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
            if let Some(r) = run.take() {
                consider(&r);
            }
            continue;
        }
        let clamp = |v: f64| v.clamp(-1e9, 1e9);
        let (ax, ay, bx, by) = (clamp(a.0), clamp(a.1), clamp(b.0), clamp(b.1));
        let dx = bx - ax;
        let dy = by - ay;
        let flat = dx != 0.0 && dy.abs() <= dx.abs() * 0.5;
        let dir = dx.signum();
        let extends = run.as_ref().is_some_and(|r| {
            flat && r.dir == dir && r.x1 == ax && r.yhi.max(by) - r.ylo.min(by) <= STREET_MAX_DY
        });
        if extends {
            if let Some(r) = run.as_mut() {
                r.x1 = bx;
                r.ylo = r.ylo.min(by);
                r.yhi = r.yhi.max(by);
            }
        } else {
            if let Some(r) = run.take() {
                consider(&r);
            }
            if flat && dy.abs() <= STREET_MAX_DY {
                run = Some(Run {
                    x0: ax,
                    x1: bx,
                    ylo: ay.min(by),
                    yhi: ay.max(by),
                    dir,
                });
            }
        }
    }
    if let Some(r) = run.take() {
        consider(&r);
    }
    best.map(|(_, col, row)| (col, row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maps::canvas::is_console_char;
    use std::time::Instant;

    fn f(s: &str) -> Option<String> {
        fold(s)
    }

    #[test]
    fn folding_table() {
        let cases = [
            // Letters the console font has are kept.
            ("Zürich", "Zürich"),
            ("Besançon", "Besançon"),
            ("Orléans", "Orléans"),
            ("Straße", "Straße"),
            ("Ålesund Malmö Århus", "Ålesund Malmö Århus"),
            ("Æblegård", "Æblegård"),
            ("Ñuñoa", "Ñuñoa"),
            ("Évreux Ühlingen Öland Ærø", "Évreux Ühlingen Öland Æro"),
            ("L’Haÿ-les-Roses", "L'Haÿ-les-Roses"),
            // Letters it does not have are folded; kept ones stay.
            ("Saint-Étienne", "Saint-Étienne"),
            ("Ørsted Ålesund", "Orsted Ålesund"),
            ("Œuvre cœur", "OEuvre coeur"),
            ("Þingvellir", "THingvellir"),
            ("Łódź", "Lódz"),
            ("Kraków Gdańsk", "Kraków Gdansk"),
            ("İstanbul ışık", "Istanbul isik"),
            ("Ĳsselmeer", "IJsselmeer"),
            ("Brașov Timișoara", "Brasov Timisoara"),
            ("Čeština Ďáblice Ňůž", "Cestina Dáblice Nuz"),
            ("Győr Szőny", "Gyor Szony"),
            ("Île-de-France Ête", "Ile-de-France Ete"),
            ("São Tomé", "Sao Tomé"),
            ("Àger Ìsola Ùrbe Ýdra", "Ager Isola Urbe Ydra"),
            ("ƒ ª º", "f a o"),
            (
                "© OpenStreetMap contributors",
                "(c) OpenStreetMap contributors",
            ),
            ("Natural Earth", "Natural Earth"),
            ("«Rue du Pont»", "\"Rue du Pont\""),
            ("Ａｂｃ１", "Abc1"),
            ("Mont\u{A0}Blanc", "Mont Blanc"),
            ("  Two   spaces\tand\ttabs\n", "Two spaces and tabs"),
        ];
        for (input, want) in cases {
            assert_eq!(f(input).as_deref(), Some(want), "{input:?}");
        }
    }

    #[test]
    fn ascii_folding_is_unchanged() {
        // fold_ascii_to keeps the Phase 5 table: no accented letter left.
        let cases = [
            ("Zürich", "Zurich"),
            ("Besançon", "Besancon"),
            ("Straße", "Strasse"),
            ("Ørsted Ålesund", "Orsted Alesund"),
            ("Æblegård", "AEblegard"),
            ("Łódź", "Lodz"),
            ("L’Haÿ-les-Roses", "L'Hay-les-Roses"),
            ("Tokyo 東京", "Tokyo ?"),
        ];
        for (input, want) in cases {
            assert_eq!(fold_ascii_to(input, MAX_LABEL).as_deref(), Some(want));
        }
        for c in CONSOLE_LETTERS.chars() {
            let out = fold_ascii_to(&c.to_string(), 8).unwrap_or_default();
            assert!(
                !out.is_empty() && out.bytes().all(|b| b.is_ascii_alphabetic()),
                "{c:?} -> {out:?}"
            );
        }
    }

    #[test]
    fn kept_letters_are_exactly_the_console_letters() {
        // Every console letter is kept alone and in context, and its
        // decomposed form (base + combining mark) folds to the base letter.
        for c in CONSOLE_LETTERS.chars() {
            assert_eq!(f(&format!("A{c}b")), Some(format!("A{c}b")));
            assert_eq!(f(&c.to_string()), Some(c.to_string()), "{c:?}");
        }
        assert_eq!(f("e\u{301}").as_deref(), Some("e"));
        // A name made only of kept letters is shown.
        assert_eq!(f("Åå").as_deref(), Some("Åå"));
        // Nothing outside ASCII and CONSOLE_LETTERS ever comes out.
        for cp in 0x80..0x3_0000u32 {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            let out = f(&format!("x{c}")).unwrap_or_default();
            for o in out.chars() {
                assert!(
                    o == ' ' || o.is_ascii_graphic() || (o == c && CONSOLE_LETTERS.contains(c)),
                    "U+{cp:04X} -> {out:?}"
                );
            }
        }
    }

    #[test]
    fn widths_count_characters_not_bytes() {
        // 50 two-byte letters: cut at 40 characters (37 + "..."), not at
        // 40 bytes.
        let long = "é".repeat(50);
        let got = f(&long).unwrap_or_default();
        assert_eq!(got.chars().count(), MAX_LABEL);
        assert_eq!(got, format!("{}...", "é".repeat(37)));
        // Exactly 40 letters: not cut.
        let forty = "ü".repeat(40);
        assert_eq!(f(&forty).as_deref(), Some(forty.as_str()));
        // The cap applies to a mix too, and the dots follow a whole letter.
        let mixed = format!("{} {}", "Orléans".repeat(5), "x".repeat(10));
        let got = f(&mixed).unwrap_or_default();
        assert_eq!(got.chars().count(), MAX_LABEL);
        assert!(got.ends_with("...") && got.starts_with("OrléansOrléans"));
        // A cut never leaves a trailing space before the dots.
        let spaced = format!("{} {}", "ß".repeat(36), "b".repeat(10));
        assert_eq!(f(&spaced), Some(format!("{}...", "ß".repeat(36))));
        // Placement measures in characters: "Orléans" takes 7 cells.
        let placed = place(&[cand(10, 10, 1, None, "Orléans")], 20, 3, &[]);
        let (col, _, text) = placed[0].text.clone().unwrap_or_default();
        assert_eq!(text, "Orléans");
        assert_eq!(col, 7);
        let drawn = render(20, 3, &placed);
        assert_eq!(drawn.lines().nth(1), Some("       Orléans      "));
    }

    #[test]
    fn folding_untrusted_text() {
        // Control characters and escape sequences: the ESC is removed, so
        // the rest is harmless text.
        assert_eq!(
            f("Evil\u{1b}[2J\u{1b}]0;x\u{7}Town").as_deref(),
            Some("Evil[2J]0;xTown")
        );
        assert_eq!(f("A\u{0}B\u{9b}C\u{7f}D").as_deref(), Some("ABCD"));
        // Bidirectional overrides and zero-width characters vanish.
        assert_eq!(f("\u{202e}nwoT\u{202c}\u{200b}").as_deref(), Some("nwoT"));
        // Combining marks: decomposed accents fold like composed ones.
        assert_eq!(f("Ze\u{301}ro\u{308}\u{20dd}").as_deref(), Some("Zero"));
        // Right-to-left and CJK scripts: one ? per run, and no label when
        // nothing else is left.
        assert_eq!(f("القاهرة"), None);
        assert_eq!(f("東京"), None);
        assert_eq!(f("Москва"), None);
        assert_eq!(f("Tokyo 東京"), Some("Tokyo ?".to_string()));
        assert_eq!(f("Athens Αθήνα"), Some("Athens ?".to_string()));
        assert_eq!(f("a東b京c"), Some("a?b?c".to_string()));
        // Nothing to show.
        for s in [
            "",
            "   ",
            "\u{1b}\u{7}",
            "???",
            "\u{301}",
            "--",
            "\u{feff}",
            "🏔️",
        ] {
            assert_eq!(f(s), None, "{s:?}");
        }
        // Very long names are cut to 40 with "...", cheaply.
        let long = "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch";
        let got = f(long).unwrap_or_default();
        assert_eq!(got.len(), MAX_LABEL);
        assert!(got.ends_with("...") && got.starts_with("Llanfair"));
        let start = Instant::now();
        let huge = "é".repeat(5_000_000) + &"\u{301}".repeat(5_000_000);
        assert_eq!(f(&huge).map(|s| s.chars().count()), Some(MAX_LABEL));
        let huge = "ę".repeat(5_000_000);
        assert_eq!(f(&huge).map(|s| s.len()), Some(MAX_LABEL));
        let only_marks = "\u{301}".repeat(5_000_000) + "Late";
        assert_eq!(f(&only_marks), None);
        assert!(start.elapsed().as_millis() < 200);
        // A cut never leaves a trailing space before the dots.
        let spaced = format!("{} {}", "a".repeat(36), "b".repeat(10));
        assert_eq!(f(&spaced), Some(format!("{}...", "a".repeat(36))));
        // A custom cap for attribution lines.
        assert_eq!(
            fold_to("© OpenStreetMap contributors", 80).as_deref(),
            Some("(c) OpenStreetMap contributors")
        );
    }

    #[test]
    fn folding_output_is_always_console_text() {
        // Every character of the Basic Multilingual Plane and a few beyond,
        // alone and in context: printable ASCII or a console letter, at
        // most MAX_LABEL characters.
        for cp in (0..0x3_0000u32)
            .step_by(1)
            .filter(|c| *c < 0xD800 || *c > 0xDFFF)
        {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            let s = format!("A{c}b");
            let out = f(&s).unwrap_or_default();
            assert!(
                out.chars()
                    .all(|c| c == ' ' || c.is_ascii_graphic() || CONSOLE_LETTERS.contains(c)),
                "U+{cp:04X} -> {out:?}"
            );
            assert!(out.chars().all(is_console_char));
            assert!(out.chars().count() <= MAX_LABEL);
            let ascii = fold_ascii_to(&s, MAX_LABEL).unwrap_or_default();
            assert!(
                ascii.chars().all(|c| c == ' ' || c.is_ascii_graphic()),
                "U+{cp:04X} -> {ascii:?}"
            );
        }
    }

    fn cand(priority: u16, col: i32, row: i32, marker: Option<char>, text: &str) -> Candidate {
        Candidate {
            priority,
            col,
            row,
            marker,
            text: text.to_string(),
            colour: Colour::White,
            keep_marker: false,
        }
    }

    fn render(cols: u16, rows: u16, placed: &[Placed]) -> String {
        let mut c = Canvas::new(cols, rows);
        draw(&mut c, placed);
        c.text()
    }

    #[test]
    fn placement_positions_margins_and_priority() {
        let mut hospital = cand(50, 18, 3, Some('H'), "Hospital");
        hospital.keep_marker = true;
        let cands = vec![
            cand(70, 3, 1, Some('*'), "Vil"),
            cand(100, 2, 1, Some('*'), "City"),
            cand(90, 12, 3, Some('*'), "Town"),
            hospital,
            cand(80, 7, 3, Some('^'), "Peak"),
        ];
        let placed = place(&cands, 20, 5, &[]);
        // City first, right of its marker. Town next, right of its marker.
        // The peak's name would touch Town on the right, so it goes on the
        // left. The village's marker is covered by
        // City's name: dropped. The hospital's name fits nowhere without
        // being cut at the edge or touching Town, so its letter stays alone.
        assert_eq!(
            render(20, 5, &placed),
            [
                "                    ",
                "  *City             ",
                "                    ",
                "   Peak^    *Town H ",
                "                    ",
                "",
            ]
            .join("\n")
        );
        assert!(placed.iter().all(|p| p.index != 0));
        let order: Vec<usize> = placed.iter().map(|p| p.index).collect();
        assert_eq!(order, [1, 2, 4, 3]);
    }

    #[test]
    fn placement_never_overlaps_and_keeps_priority() {
        // A dense random field of candidates.
        let mut seed = 0x1234_5678_u32;
        let mut rnd = |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % n
        };
        let mut cands = Vec::new();
        for i in 0..3000 {
            let len = rnd(12) as usize;
            let marker = if i % 3 == 0 { None } else { Some('*') };
            let mut c = cand(
                rnd(100) as u16,
                rnd(90) as i32 - 5,
                rnd(30) as i32 - 3,
                marker,
                &"x".repeat(len),
            );
            c.keep_marker = i % 2 == 0;
            cands.push(c);
        }
        let reserved = [
            CellRect {
                col: 40,
                row: 12,
                width: 1,
                height: 1,
            },
            CellRect {
                col: 0,
                row: 23,
                width: 80,
                height: 1,
            },
        ];
        let placed = place(&cands, 80, 24, &reserved);
        assert!(!placed.is_empty());
        let mut owner = vec![usize::MAX; 80 * 24];
        let mut boxes = Vec::new();
        for (n, p) in placed.iter().enumerate() {
            let mut cells = Vec::new();
            if let Some((c, r, _)) = p.marker {
                cells.push((c, r));
            }
            if let Some((c, r, t)) = &p.text {
                for k in 0..t.len() as u16 {
                    cells.push((c + k, *r));
                }
            }
            for &(c, r) in &cells {
                assert!(c < 80 && r < 24, "inside the canvas");
                assert!(!(c == 40 && r == 12) && r != 23, "reserved cell used");
                let i = usize::from(r) * 80 + usize::from(c);
                assert_eq!(owner[i], usize::MAX, "overlap");
                owner[i] = n;
            }
            boxes.push(cells);
        }
        // One-cell margin between different labels.
        for (n, cells) in boxes.iter().enumerate() {
            for &(c, r) in cells {
                for dr in -1i32..=1 {
                    for dc in -1i32..=1 {
                        let (cc, rr) = (i32::from(c) + dc, i32::from(r) + dr);
                        if (0..80).contains(&cc) && (0..24).contains(&rr) {
                            let o = owner[rr as usize * 80 + cc as usize];
                            assert!(o == usize::MAX || o == n, "labels {n} and {o} touch");
                        }
                    }
                }
            }
        }
        // Priority is kept: placed in non-increasing priority.
        let prios: Vec<u16> = placed.iter().map(|p| cands[p.index].priority).collect();
        assert!(prios.windows(2).all(|w| w[0] >= w[1]));
        // The highest-priority candidate on the canvas is always placed
        // when it fits.
        // Deterministic whatever the input order.
        let mut rev = cands.clone();
        rev.reverse();
        let again = place(&rev, 80, 24, &reserved);
        let a: Vec<_> = placed.iter().map(|p| (p.marker, p.text.clone())).collect();
        let b: Vec<_> = again.iter().map(|p| (p.marker, p.text.clone())).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn placement_edges_markers_and_limits() {
        // Near the right edge the name goes to the left.
        let p = place(&[cand(1, 9, 0, Some('*'), "Abc")], 10, 1, &[]);
        assert_eq!(render(10, 1, &p), "      Abc*\n");
        // When the sides are taken, above (centred) is next.
        let sides = [
            CellRect {
                col: 2,
                row: 1,
                width: 3,
                height: 1,
            },
            CellRect {
                col: 6,
                row: 1,
                width: 3,
                height: 1,
            },
        ];
        let p = place(&[cand(1, 5, 1, Some('*'), "Abc")], 10, 3, &sides);
        assert_eq!(render(10, 3, &p), "    Abc   \n     *    \n          \n");
        // Off-canvas anchors and names longer than the canvas are dropped,
        // unless the marker may stay alone.
        let mut poi = cand(1, 2, 0, Some('W'), "A very long name indeed");
        assert!(place(&[poi.clone()], 10, 1, &[]).is_empty());
        poi.keep_marker = true;
        assert_eq!(render(10, 1, &place(&[poi], 10, 1, &[])), "  W       \n");
        assert!(
            place(
                &[cand(1, -1, 0, Some('*'), "A"), cand(1, 0, 5, None, "B")],
                10,
                1,
                &[]
            )
            .is_empty()
        );
        // A marker on a reserved cell (the crosshair) is not drawn, but its
        // name is, beside it; with no room for the name, nothing is placed.
        let r = [CellRect {
            col: 4,
            row: 0,
            width: 1,
            height: 1,
        }];
        let p = place(&[cand(1, 4, 0, Some('*'), "Lux")], 10, 1, &r);
        assert_eq!(render(10, 1, &p), "     Lux  \n");
        assert_eq!(p[0].marker, None);
        assert!(place(&[cand(1, 4, 0, Some('*'), "Luxembourg")], 10, 1, &r).is_empty());
        let mut alone = cand(1, 4, 0, Some('H'), "");
        alone.keep_marker = true;
        assert!(place(&[alone], 10, 1, &r).is_empty());
        // A letter marker keeps a blank cell from its text, on either side;
        // a symbol touches it.
        let p = place(&[cand(1, 1, 0, Some('o'), "Cafe")], 12, 1, &[]);
        assert_eq!(render(12, 1, &p), " o Cafe     \n");
        let p = place(&[cand(1, 10, 0, Some('H'), "Cafe")], 12, 1, &[]);
        assert_eq!(render(12, 1, &p), "     Cafe H \n");
        let p = place(&[cand(1, 1, 0, Some('*'), "Cafe")], 12, 1, &[]);
        assert_eq!(render(12, 1, &p), " *Cafe      \n");
        // The same name near a placed one is skipped (a place mapped as a
        // node and as an area); far enough away, or another name, it is not.
        let twins = [
            cand(9, 2, 1, Some('*'), "Mamer"),
            cand(8, 3, 4, Some('*'), "Mamer"),
            cand(7, 3, 7, Some('*'), "Mamer"),
            cand(6, 20, 1, Some('*'), "Mamer"),
            cand(5, 3, 3, None, "Rue"),
        ];
        let p = place(&twins, 40, 9, &[]);
        let shown: Vec<_> = p.iter().map(|q| (q.index, q.text.is_some())).collect();
        assert_eq!(shown, [(0, true), (2, true), (3, true), (4, true)]);
        // A label without a marker is centred on its anchor.
        let s = place(&[cand(1, 5, 0, None, "Rue")], 10, 1, &[]);
        assert_eq!(render(10, 1, &s), "    Rue   \n");
        // Many thousands of candidates stay cheap; the cap keeps the
        // highest priorities.
        let start = Instant::now();
        let many: Vec<_> = (0..50_000)
            .map(|i| {
                cand(
                    (i % 1000) as u16,
                    i % 200,
                    (i / 200) % 60,
                    Some('*'),
                    "Name",
                )
            })
            .collect();
        let placed = place(&many, 200, 60, &[]);
        // 50 candidates per priority, 4096 considered: priorities 999 down
        // to 918 (999 - 4095 / 50).
        assert!(!placed.is_empty());
        assert!(placed.iter().all(|p| many[p.index].priority >= 918));
        assert!(start.elapsed().as_millis() < 500);
        assert!(place(&many, 0, 0, &[]).is_empty());
        // Unfolded huge text, extreme anchors and an absurd canvas size.
        let odd = [
            cand(1, 5, 5, Some('*'), &"x".repeat(100_000)),
            cand(1, i32::MAX, i32::MIN, Some('*'), "A"),
            cand(1, 6, 6, None, &"y".repeat(100_000)),
            cand(u16::MAX, 0, 0, Some('*'), "\u{1b}[2J"),
        ];
        let p = place(
            &odd,
            u16::MAX,
            u16::MAX,
            &[CellRect {
                col: u16::MAX,
                row: u16::MAX,
                width: u16::MAX,
                height: u16::MAX,
            }],
        );
        assert_eq!(p.len(), 1);
        let mut c = Canvas::new(10, 1);
        draw(&mut c, &p);
        assert_eq!(c.text(), "*?[2J     \n");
    }

    #[test]
    fn street_names_go_on_long_flat_stretches() {
        // A steep part, then a long flat part.
        let line = [
            (0.0, 0.0),
            (5.0, 20.0),
            (10.0, 20.5),
            (30.0, 21.0),
            (31.0, 40.0),
        ];
        // The flat part runs from x 5 to 30, around y 20.5 (cell row 10).
        assert_eq!(street_anchor(&line, 10, 40, 20), Some((17, 10)));
        assert_eq!(street_anchor(&line, 23, 40, 20), Some((17, 10)));
        // Too short for the text and a cell on each side.
        assert_eq!(street_anchor(&line, 24, 40, 20), None);
        // A slowly climbing line is cut into stretches within 2 pixels.
        let climb: Vec<Point> = (0..=40)
            .map(|x| (f64::from(x), f64::from(x) * 0.25))
            .collect();
        let a = street_anchor(&climb, 4, 40, 20);
        assert!(a.is_some());
        assert_eq!(street_anchor(&climb, 10, 40, 20), None);
        // Only the part on the canvas counts.
        let off = [(-1e12, 5.0), (3.0, 5.0)];
        assert_eq!(street_anchor(&off, 4, 40, 20), None);
        let across = [(-1e12, 5.0), (1e12, 5.0)];
        assert_eq!(street_anchor(&across, 4, 40, 20), Some((20, 2)));
        // Outside vertically, vertical lines, NaN, degenerate input.
        assert_eq!(street_anchor(&[(0.0, 50.0), (40.0, 50.0)], 4, 40, 20), None);
        assert_eq!(street_anchor(&[(5.0, 0.0), (5.0, 39.0)], 1, 40, 20), None);
        assert_eq!(
            street_anchor(&[(0.0, f64::NAN), (40.0, 5.0)], 1, 40, 20),
            None
        );
        assert_eq!(street_anchor(&[], 1, 40, 20), None);
        assert_eq!(street_anchor(&[(1.0, 1.0)], 1, 40, 20), None);
        // Back and forth does not count as one stretch.
        let zig = [(0.0, 5.0), (10.0, 5.0), (0.0, 5.0), (10.0, 5.0)];
        assert_eq!(street_anchor(&zig, 9, 40, 20), None);
    }

    /// A small map drawn the way the viewer will: style lookups, passes,
    /// canvas, labels. `RESILIENCE_DUMP=1` prints it.
    fn scene(scale: f64, cols: u16, rows: u16, covered: bool) -> Canvas {
        use crate::maps::style::{self, Draw, NO_DETAIL_SHADE, Pass, SEA};
        let mut c = Canvas::new(cols, rows);
        let (w, h) = (f64::from(cols), f64::from(rows) * 2.0);
        c.fill(SEA);
        // (layer, class, geometry) in pixel coordinates.
        let land = vec![
            (-5.0, 4.0),
            (w + 5.0, -2.0),
            (w + 5.0, h + 5.0),
            (-5.0, h + 5.0),
        ];
        let lake = vec![(4.0, 14.0), (16.0, 12.0), (18.0, 22.0), (6.0, 24.0)];
        let island = vec![(9.0, 16.0), (12.0, 16.0), (12.0, 19.0), (9.0, 19.0)];
        let forest = vec![
            (44.0, 22.0),
            (w + 3.0, 20.0),
            (w + 3.0, h + 3.0),
            (40.0, h + 3.0),
        ];
        let areas: Vec<(&str, &str, Vec<Vec<Point>>)> = vec![
            ("land", "landmass", vec![land]),
            ("land", "forest", vec![forest]),
            ("water", "lake", vec![lake, island]),
        ];
        let lines: Vec<(&str, &str, Vec<Point>)> = vec![
            (
                "water",
                "river",
                vec![(17.0, 18.0), (30.0, 26.0), (34.0, h + 1.0)],
            ),
            ("roads", "residential", vec![(20.0, 6.0), (24.0, 30.0)]),
            (
                "roads",
                "primary",
                vec![(-3.0, 30.0), (30.0, 21.0), (w + 3.0, 20.0)],
            ),
            ("roads", "path", vec![(36.0, 8.0), (56.0, 18.0)]),
            ("rail", "rail", vec![(0.0, 9.0), (w, 7.0)]),
            ("boundaries", "8", vec![(52.0, -1.0), (50.0, h + 1.0)]),
        ];
        let mut features: Vec<(&style::Rule, usize)> = Vec::new();
        for (i, (l, k, _)) in areas.iter().enumerate() {
            if let Some(r) = style::lookup(l, k, scale) {
                features.push((r, i));
            }
        }
        for (i, (l, k, _)) in lines.iter().enumerate() {
            if let Some(r) = style::lookup(l, k, scale) {
                features.push((r, i));
            }
        }
        features.sort_by_key(|(r, _)| r.sort_key());
        for (r, i) in features {
            match (r.pass(), r.draw) {
                (Pass::Areas, _) => c.fill_polygon(&areas[i].2, r.colour),
                (Pass::Lines, Draw::Line(p)) => c.polyline(&lines[i].2, r.colour, p),
                _ => {}
            }
        }
        if !covered {
            c.shade_polygon(
                &[vec![(0.0, 0.0), (30.0, 0.0), (30.0, h), (0.0, h)]],
                false,
                NO_DETAIL_SHADE,
            );
        }
        // Labels.
        let mut cands = Vec::new();
        let points = [
            ("places", "town", 26, 2, "Testville"),
            ("places", "village", 60, 3, "Le Hameau-d'Essai"),
            ("peaks", "peak", 70, 14, "Mont Test"),
            ("pois", "health", 27, 6, "Pharmacie"),
            ("pois", "water", 30, 7, "Fontaine"),
        ];
        for (l, k, col, row, name) in points {
            let Some(r) = style::lookup(l, k, scale) else {
                continue;
            };
            let Some(lr) = r.label() else { continue };
            let mut text = if lr.name_visible(scale) {
                fold(name).unwrap_or_default()
            } else {
                String::new()
            };
            if lr.detail_visible(scale) {
                text.push_str(" 812");
            }
            cands.push(Candidate {
                priority: lr.priority,
                col,
                row,
                marker: lr.marker,
                text,
                colour: r.colour,
                keep_marker: lr.keep_marker,
            });
        }
        if style::STREET_NAMES.name_visible(scale) {
            let street = [(-3.0, 30.0), (30.0, 21.0), (w + 3.0, 20.0)];
            let name = fold("Rue de l’Essai").unwrap_or_default();
            if let Some((col, row)) = street_anchor(&street, name.len(), cols, rows) {
                cands.push(Candidate {
                    priority: style::STREET_NAMES.priority,
                    col,
                    row,
                    marker: None,
                    text: name,
                    colour: style::STREET_NAME_COLOUR,
                    keep_marker: false,
                });
            }
        }
        let centre = (cols / 2, rows / 2);
        let reserved = [CellRect {
            col: centre.0,
            row: centre.1,
            width: 1,
            height: 1,
        }];
        draw(&mut c, &place(&cands, cols, rows, &reserved));
        c.put_char(
            i64::from(centre.0),
            i64::from(centre.1),
            style::CROSSHAIR,
            None,
            None,
            true,
        );
        c
    }

    #[test]
    fn whole_scenes_are_console_safe_and_drawn_by_scale() {
        let dump = std::env::var_os("RESILIENCE_DUMP").is_some();
        for (scale, covered) in [(10.0, true), (50.0, true), (500.0, false)] {
            let c = scene(scale, 80, 16, covered);
            let text = c.text();
            if dump {
                println!("--- {scale} m per pixel ---\n{text}{}", c.pixel_picture());
            }
            assert!(text.chars().all(|ch| ch == '\n' || is_console_char(ch)));
            assert!(c.cells().iter().all(|cell| is_console_char(cell.ch)));
            assert_eq!(text.contains("Rue de l'Essai"), scale <= 10.0, "{scale}");
            assert!(text.contains("Testville"));
            assert_eq!(text.contains("Pharmacie"), scale <= 10.0);
            assert_eq!(c.cell(27, 6).ch == 'H', scale <= 20.0);
            assert_eq!(c.cell(27, 6).fg == Some(Colour::Cyan), scale <= 20.0);
            assert_eq!(text.contains('░'), !covered);
            assert!(text.contains('+'));
        }
        let legend = crate::maps::style::legend(None);
        assert!(
            legend
                .iter()
                .flat_map(|e| &e.sample)
                .all(|cell| is_console_char(cell.ch))
        );
    }

    #[test]
    fn drawn_labels_are_console_safe() {
        let mut c = Canvas::new(30, 3);
        let names = ["Zürich", "東京", "\u{1b}[31mRed", "Ærø"];
        let cands: Vec<_> = names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| fold(n).map(|t| cand(10, 2 + 7 * i as i32, 1, Some('*'), &t)))
            .collect();
        draw(&mut c, &place(&cands, 30, 3, &[]));
        assert!(c.text().chars().all(|ch| ch == '\n' || is_console_char(ch)));
        assert!(c.text().contains("Zürich"));
    }
}
