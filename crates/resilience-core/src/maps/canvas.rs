//! Pixel canvas for the map viewer (ADR 0016 §3, §7).
//!
//! A canvas of `cols` x `rows` console cells holds `cols` x `2 * rows`
//! square pixels: each cell shows two pixels stacked, printed with `▀`
//! (top pixel as foreground, bottom as background) or `▄` when only the
//! bottom pixel is drawn. Two equal pixels print as a space with that
//! background. Text overlays (labels, markers, the crosshair) replace whole
//! cells, and shaded cells (no detailed map here) print as `░`.
//!
//! [`Canvas::cells`] gives a grid of [`Cell`]s that a ratatui widget copies
//! as it is. Every character it can contain is printable ASCII or one of
//! the console glyphs in [`CONSOLE_GLYPHS`] (CLAUDE.md §4).
//!
//! Geometry is given in pixel coordinates as `(x, y)` pairs of `f64`, `x`
//! to the right and `y` downwards; pixel `(i, j)` covers `[i, i+1) x [j,
//! j+1)`. Coordinates may lie anywhere (far outside the canvas, infinite
//! or NaN): every drawing function clips, and its cost is bounded by the
//! canvas size and the vertex count, never by the coordinate magnitude.
//! Coordinates are clamped to ±[`COORD_LIMIT`] first, which keeps every
//! intermediate result finite; geometry that far away is not meaningful
//! and the clamp moves what is drawn on the canvas by far less than a
//! pixel.

/// The 8 basic ANSI colours, the only ones the Linux console has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Colour {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

impl Colour {
    pub const ALL: [Colour; 8] = [
        Colour::Black,
        Colour::Red,
        Colour::Green,
        Colour::Yellow,
        Colour::Blue,
        Colour::Magenta,
        Colour::Cyan,
        Colour::White,
    ];

    /// Lower-case English name, e.g. `"blue"`.
    pub const fn name(self) -> &'static str {
        match self {
            Colour::Black => "black",
            Colour::Red => "red",
            Colour::Green => "green",
            Colour::Yellow => "yellow",
            Colour::Blue => "blue",
            Colour::Magenta => "magenta",
            Colour::Cyan => "cyan",
            Colour::White => "white",
        }
    }

    /// One letter per colour, used by [`Canvas::pixel_picture`]: `K` for
    /// black, then `R G Y B M C W`.
    pub const fn letter(self) -> char {
        match self {
            Colour::Black => 'K',
            Colour::Red => 'R',
            Colour::Green => 'G',
            Colour::Yellow => 'Y',
            Colour::Blue => 'B',
            Colour::Magenta => 'M',
            Colour::Cyan => 'C',
            Colour::White => 'W',
        }
    }
}

/// A pixel: a colour, or `None` where nothing was drawn (the terminal's
/// default background).
pub type Pixel = Option<Colour>;

/// A point in pixel coordinates.
pub type Point = (f64, f64);

/// The non-ASCII glyphs the console font has (CLAUDE.md §4).
pub const CONSOLE_GLYPHS: &str = "▀▄█═║╔╗╚╝─│┌┐└┘░▒▓";

/// True for printable ASCII (space included) and the console glyphs: the
/// only characters a [`Cell`] may hold.
pub fn is_console_char(c: char) -> bool {
    c == ' ' || c.is_ascii_graphic() || CONSOLE_GLYPHS.contains(c)
}

/// Largest canvas, in cells. Larger requests are reduced to it (no console
/// is that large; it bounds memory to about 1 MiB).
pub const MAX_COLS: u16 = 1024;
pub const MAX_ROWS: u16 = 512;

/// Coordinates are clamped to ±2^40 pixels before any arithmetic.
pub const COORD_LIMIT: f64 = 1_099_511_627_776.0;

/// How a line is drawn. Dashed is 3 pixels on, 2 off; dotted is 1 on, 2
/// off. The pattern continues across the vertices of a polyline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LinePattern {
    Solid,
    Dashed,
    Dotted,
}

impl LinePattern {
    /// (pixels on, pixels off) of one period.
    pub const fn period(self) -> (u32, u32) {
        match self {
            LinePattern::Solid => (1, 0),
            LinePattern::Dashed => (3, 2),
            LinePattern::Dotted => (1, 2),
        }
    }

    /// Whether the pixel at distance `pos` from the start is drawn.
    fn on(self, pos: f64) -> bool {
        let (on, off) = self.period();
        if off == 0 {
            return true;
        }
        let period = f64::from(on + off);
        pos.floor().rem_euclid(period) < f64::from(on)
    }
}

/// One console cell as the viewer prints it. `fg`/`bg` `None` mean the
/// terminal's default colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
    pub reverse: bool,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: None,
        bg: None,
        reverse: false,
    };
}

/// A set of cells, e.g. the cells inside a pack's coverage polygon. A cell
/// is inside a polygon when its centre is (even-odd rule).
#[derive(Clone, Debug)]
pub struct CellMask {
    cols: usize,
    rows: usize,
    bits: Vec<bool>,
}

impl CellMask {
    /// An empty mask for a canvas of `cols` x `rows` cells (capped like
    /// [`Canvas::new`]).
    pub fn new(cols: u16, rows: u16) -> CellMask {
        let cols = usize::from(cols.min(MAX_COLS));
        let rows = usize::from(rows.min(MAX_ROWS));
        CellMask {
            cols,
            rows,
            bits: vec![false; cols * rows],
        }
    }

    /// Add the cells inside a polygon given in **pixel** coordinates (the
    /// same coordinates as the canvas drawing functions). Rings as in
    /// [`Canvas::fill_polygon`]. Several calls make a union.
    pub fn add_polygon<R: AsRef<[Point]>>(&mut self, rings: &[R]) {
        let cols = self.cols;
        let mut scratch = Vec::new();
        let bits = &mut self.bits;
        even_odd_spans(rings, cols, self.rows, 2.0, &mut scratch, |row, c0, c1| {
            for b in &mut bits[row * cols + c0..row * cols + c1] {
                *b = true;
            }
        });
    }

    /// Every cell not in the mask, and only those.
    pub fn invert(&mut self) {
        for b in &mut self.bits {
            *b = !*b;
        }
    }

    pub fn contains(&self, col: usize, row: usize) -> bool {
        col < self.cols && row < self.rows && self.bits[row * self.cols + col]
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }
}

/// The pixel canvas. See the module documentation.
#[derive(Clone, Debug)]
pub struct Canvas {
    cols: usize,
    rows: usize,
    pixels: Vec<Pixel>,
    shade: Vec<Option<Colour>>,
    overlay: Vec<Option<Cell>>,
    scratch: Vec<u8>,
}

impl Canvas {
    /// A blank canvas of `cols` x `rows` cells, so `cols` x `2 * rows`
    /// pixels. Sizes above [`MAX_COLS`] x [`MAX_ROWS`] are reduced.
    pub fn new(cols: u16, rows: u16) -> Canvas {
        let cols = usize::from(cols.min(MAX_COLS));
        let rows = usize::from(rows.min(MAX_ROWS));
        Canvas {
            cols,
            rows,
            pixels: vec![None; cols * rows * 2],
            shade: vec![None; cols * rows],
            overlay: vec![None; cols * rows],
            scratch: Vec::new(),
        }
    }

    /// Width in cells (and in pixels).
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Height in cells.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Width in pixels (= cells).
    pub fn width(&self) -> usize {
        self.cols
    }

    /// Height in pixels (2 per cell).
    pub fn height(&self) -> usize {
        self.rows * 2
    }

    /// Erase pixels, shading and overlays.
    pub fn clear(&mut self) {
        self.pixels.fill(None);
        self.shade.fill(None);
        self.overlay.fill(None);
    }

    /// Paint every pixel (e.g. the sea before the land is drawn).
    pub fn fill(&mut self, colour: Colour) {
        self.pixels.fill(Some(colour));
    }

    /// The pixel at `(x, y)`, `None` outside the canvas or where nothing
    /// was drawn.
    pub fn pixel(&self, x: usize, y: usize) -> Pixel {
        if x < self.width() && y < self.height() {
            self.pixels[y * self.cols + x]
        } else {
            None
        }
    }

    /// Set the pixel containing the point, if it is on the canvas.
    pub fn set_point(&mut self, p: Point, colour: Colour) {
        let (x, y) = clamp_point(p);
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        let (x, y) = (x.floor(), y.floor());
        if x >= 0.0 && y >= 0.0 && x < self.width() as f64 && y < self.height() as f64 {
            // In range, so the conversions are exact.
            self.put_pixel(x as usize, y as usize, colour);
        }
    }

    fn put_pixel(&mut self, x: usize, y: usize, colour: Colour) {
        let i = y * self.cols + x;
        self.pixels[i] = Some(colour);
    }

    /// A straight line from `a` to `b` (both end pixels included).
    pub fn line(&mut self, a: Point, b: Point, colour: Colour, pattern: LinePattern) {
        self.polyline(&[a, b], colour, pattern);
    }

    /// A polyline through `points`. The dash pattern continues across
    /// vertices, and counts the length of the parts outside the canvas
    /// too, so panning does not make dashes jump. Segments with a
    /// non-finite end are skipped. Cost: O(points + drawn pixels).
    pub fn polyline(&mut self, points: &[Point], colour: Colour, pattern: LinePattern) {
        self.polyline_from(points, colour, pattern, 0.0);
    }

    /// [`Canvas::polyline`] whose pattern starts at `phase` pixels into its
    /// period instead of 0 (a non-finite phase counts as 0). The map viewer
    /// passes a phase derived from the line's absolute position, so the two
    /// pieces of a line cut at a tile edge continue each other's dashes.
    pub fn polyline_from(
        &mut self,
        points: &[Point],
        colour: Colour,
        pattern: LinePattern,
        phase: f64,
    ) {
        let (on, off) = pattern.period();
        let period = f64::from(on + off);
        let mut phase = if phase.is_finite() {
            phase.rem_euclid(period)
        } else {
            0.0
        };
        let w = self.width() as f64;
        let h = self.height() as f64;
        for seg in points.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
                continue;
            }
            let (a, b) = (clamp_point(a), clamp_point(b));
            let len = (b.0 - a.0).abs().max((b.1 - a.1).abs());
            if let Some((t0, t1)) = clip_segment(a, b, w, h) {
                let p0 = lerp(a, b, t0);
                let p1 = lerp(a, b, t1);
                self.bresenham(p0, p1, colour, pattern, phase + t0 * len);
            }
            phase = (phase + len).rem_euclid(period);
        }
    }

    /// Bresenham between two points already clipped to `[0, w) x [0, h)`.
    fn bresenham(
        &mut self,
        p0: Point,
        p1: Point,
        colour: Colour,
        pattern: LinePattern,
        phase: f64,
    ) {
        let max_x = self.width() as i64 - 1;
        let max_y = self.height() as i64 - 1;
        if max_x < 0 || max_y < 0 {
            return;
        }
        // Clipped coordinates are within [0, w) x [0, h), so these
        // conversions are exact and small.
        let cell = |v: f64, max: i64| (v.floor() as i64).clamp(0, max);
        let (mut x, mut y) = (cell(p0.0, max_x), cell(p0.1, max_y));
        let (x1, y1) = (cell(p1.0, max_x), cell(p1.1, max_y));
        let dx = (x1 - x).abs();
        let dy = -(y1 - y).abs();
        let sx = if x < x1 { 1 } else { -1 };
        let sy = if y < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut step = 0.0_f64;
        loop {
            if pattern.on(phase + step) {
                // In range by the clamps above.
                self.put_pixel(x as usize, y as usize, colour);
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
            step += 1.0;
        }
    }

    /// Fill a polygon: `rings` are closed rings of points (the last point
    /// joins the first; repeating the first point is allowed). A pixel is
    /// filled when its centre is inside by the **even-odd** rule, so
    /// vector-tile polygons (an exterior ring followed by its interior
    /// rings, which are holes) come out right whatever the winding of the
    /// rings, and several exteriors in one call each get their holes.
    /// Hostile winding cannot change the result. A ring with a non-finite
    /// point is ignored as a whole (dropping one edge would break the
    /// parity of every row). Cost: O(vertices + crossings + bounding box),
    /// where crossings are at most vertices x height.
    pub fn fill_polygon<R: AsRef<[Point]>>(&mut self, rings: &[R], colour: Colour) {
        let cols = self.cols;
        let mut scratch = std::mem::take(&mut self.scratch);
        let pixels = &mut self.pixels;
        even_odd_spans(
            rings,
            cols,
            self.rows * 2,
            1.0,
            &mut scratch,
            |row, c0, c1| {
                for p in &mut pixels[row * cols + c0..row * cols + c1] {
                    *p = Some(colour);
                }
            },
        );
        self.scratch = scratch;
    }

    /// Shade the cells of `mask` with `░` in `colour` (no detailed map
    /// here, ADR 0016 §3). A shaded cell whose two pixels are equal prints
    /// `░` with the pixels' colour as background, so land and sea still
    /// show; a shaded cell with two different pixels keeps its half block,
    /// so coasts, borders and roads of the overview stay visible.
    pub fn shade(&mut self, mask: &CellMask, colour: Colour) {
        for row in 0..self.rows.min(mask.rows) {
            for col in 0..self.cols.min(mask.cols) {
                if mask.contains(col, row) {
                    self.shade[row * self.cols + col] = Some(colour);
                }
            }
        }
    }

    /// Shade the cells inside a polygon (`inside`) or outside it.
    pub fn shade_polygon<R: AsRef<[Point]>>(&mut self, rings: &[R], inside: bool, colour: Colour) {
        let mut mask = CellMask::new(self.cols as u16, self.rows as u16);
        mask.add_polygon(rings);
        if !inside {
            mask.invert();
        }
        self.shade(&mask, colour);
    }

    /// Put one character in a cell, replacing what is drawn there. A
    /// character the console cannot show becomes `?`. Off-canvas: ignored.
    pub fn put_char(
        &mut self,
        col: i64,
        row: i64,
        ch: char,
        fg: Option<Colour>,
        bg: Option<Colour>,
        reverse: bool,
    ) {
        if col < 0 || row < 0 || col >= self.cols as i64 || row >= self.rows as i64 {
            return;
        }
        let ch = if is_console_char(ch) { ch } else { '?' };
        let i = row as usize * self.cols + col as usize;
        self.overlay[i] = Some(Cell {
            ch,
            fg,
            bg,
            reverse,
        });
    }

    /// Put text starting at a cell, one character per cell, clipped at the
    /// canvas edges. Characters the console cannot show become `?`.
    pub fn put_text(
        &mut self,
        col: i64,
        row: i64,
        text: &str,
        fg: Option<Colour>,
        bg: Option<Colour>,
        reverse: bool,
    ) {
        if row < 0 || row >= self.rows as i64 {
            return;
        }
        for (i, ch) in text.chars().enumerate() {
            let c = col.saturating_add(i as i64);
            if c >= self.cols as i64 {
                break;
            }
            self.put_char(c, row, ch, fg, bg, reverse);
        }
    }

    /// Whether a text overlay covers the cell.
    pub fn has_overlay(&self, col: usize, row: usize) -> bool {
        col < self.cols && row < self.rows && self.overlay[row * self.cols + col].is_some()
    }

    /// The composed cell at `(col, row)` (blank outside the canvas).
    pub fn cell(&self, col: usize, row: usize) -> Cell {
        if col >= self.cols || row >= self.rows {
            return Cell::BLANK;
        }
        let i = row * self.cols + col;
        if let Some(c) = self.overlay[i] {
            return c;
        }
        let top = self.pixels[2 * row * self.cols + col];
        let bottom = self.pixels[(2 * row + 1) * self.cols + col];
        // Shading shows only on flat cells: a cell with two different
        // pixels (a coast, a border, a road of the overview) keeps its
        // half block, so the overview stays readable under the shading.
        if let Some(shade) = self.shade[i].filter(|_| top == bottom) {
            let bg = top;
            let fg = if bg == Some(shade) {
                if shade == Colour::Black {
                    Colour::White
                } else {
                    Colour::Black
                }
            } else {
                shade
            };
            return Cell {
                ch: '░',
                fg: Some(fg),
                bg,
                reverse: false,
            };
        }
        match (top, bottom) {
            (t, b) if t == b => Cell {
                ch: ' ',
                fg: None,
                bg: t,
                reverse: false,
            },
            (Some(t), b) => Cell {
                ch: '▀',
                fg: Some(t),
                bg: b,
                reverse: false,
            },
            (None, b) => Cell {
                ch: '▄',
                fg: b,
                bg: None,
                reverse: false,
            },
        }
    }

    /// All cells, row by row (`rows * cols` of them).
    pub fn cells(&self) -> Vec<Cell> {
        let mut out = Vec::with_capacity(self.cols * self.rows);
        for row in 0..self.rows {
            for col in 0..self.cols {
                out.push(self.cell(col, row));
            }
        }
        out
    }

    /// The characters of the composed cells, one line per row (for tests
    /// and screen dumps).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                s.push(self.cell(col, row).ch);
            }
            s.push('\n');
        }
        s
    }

    /// The pixels as letters ([`Colour::letter`], `.` for none), one line
    /// per pixel row (for tests and screen dumps).
    pub fn pixel_picture(&self) -> String {
        let mut s = String::new();
        for y in 0..self.height() {
            for x in 0..self.width() {
                s.push(self.pixel(x, y).map_or('.', Colour::letter));
            }
            s.push('\n');
        }
        s
    }
}

/// Clamp both coordinates to ±COORD_LIMIT (NaN stays NaN).
fn clamp_point(p: Point) -> Point {
    (
        p.0.clamp(-COORD_LIMIT, COORD_LIMIT),
        p.1.clamp(-COORD_LIMIT, COORD_LIMIT),
    )
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Liang-Barsky clipping of the segment a-b (finite, clamped) to the
/// canvas `[0, w) x [0, h)` (as the box `[0, w - EDGE] x [0, h - EDGE]`,
/// so a segment lying on the right or bottom edge is outside): the
/// parameters `0 <= t0 <= t1 <= 1` of the visible part, or `None`.
/// Constant time whatever the coordinates.
fn clip_segment(a: Point, b: Point, w: f64, h: f64) -> Option<(f64, f64)> {
    const EDGE: f64 = 1e-6;
    let (w, h) = (w - EDGE, h - EDGE);
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    for (p, q) in [(-dx, a.0), (dx, w - a.0), (-dy, a.1), (dy, h - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some((t0, t1))
}

/// Scanline even-odd fill shared by the canvas (pixels, `y_scale` 1) and
/// the cell mask (cells, `y_scale` 2: a cell's centre is at pixel row
/// `2r + 1`). Row `r` is sampled at `y = (r + 0.5) * y_scale`, column `c`
/// at `x = c + 0.5`. Calls `span(row, c0, c1)` for each filled run
/// `c0..c1`. Only rows inside both the polygon and the canvas are visited.
/// The edges of a closed ring, clamped.
fn ring_edges(ring: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    let n = ring.len();
    (0..n).map(move |i| (clamp_point(ring[i]), clamp_point(ring[(i + 1) % n])))
}

fn even_odd_spans<R: AsRef<[Point]>>(
    rings: &[R],
    cols: usize,
    rows: usize,
    y_scale: f64,
    scratch: &mut Vec<u8>,
    mut span: impl FnMut(usize, usize, usize),
) {
    if cols == 0 || rows == 0 {
        return;
    }
    // Row range [r0, r1) of an edge: rows whose sample y lies in
    // [min y, max y).
    let row_range = |ya: f64, yb: f64| -> (usize, usize) {
        let (lo, hi) = if ya < yb { (ya, yb) } else { (yb, ya) };
        let to_row = |y: f64| -> usize {
            let r = (y / y_scale - 0.5).ceil();
            if r <= 0.0 {
                0
            } else if r >= rows as f64 {
                rows
            } else {
                r as usize
            }
        };
        (to_row(lo), to_row(hi))
    };
    let usable =
        |ring: &[Point]| ring.len() >= 2 && ring.iter().all(|p| p.0.is_finite() && p.1.is_finite());

    // Pass 1: the rows touched.
    let (mut rmin, mut rmax) = (rows, 0);
    for ring in rings {
        let ring = ring.as_ref();
        if !usable(ring) {
            continue;
        }
        for (a, b) in ring_edges(ring) {
            if a.1 == b.1 {
                continue;
            }
            let (r0, r1) = row_range(a.1, b.1);
            if r0 < r1 {
                rmin = rmin.min(r0);
                rmax = rmax.max(r1);
            }
        }
    }
    if rmin >= rmax {
        return;
    }

    // Pass 2: toggle the parity at the first column right of each crossing.
    let stride = cols + 1;
    scratch.clear();
    scratch.resize((rmax - rmin) * stride, 0);
    for ring in rings {
        let ring = ring.as_ref();
        if !usable(ring) {
            continue;
        }
        for (a, b) in ring_edges(ring) {
            if a.1 == b.1 {
                continue;
            }
            let (r0, r1) = row_range(a.1, b.1);
            let dy = b.1 - a.1;
            for r in r0..r1 {
                let ys = (r as f64 + 0.5) * y_scale;
                // t is in [0, 1], so x stays finite even for nearly flat edges.
                let x = a.0 + (b.0 - a.0) * ((ys - a.1) / dy);
                // Pixel centres c + 0.5 > x toggle: c = floor(x - 0.5) + 1.
                let c = (x - 0.5).floor() + 1.0;
                let c = if c.is_nan() || c <= 0.0 {
                    0
                } else if c >= cols as f64 {
                    cols
                } else {
                    c as usize
                };
                scratch[(r - rmin) * stride + c] ^= 1;
            }
        }
    }

    // Pass 3: running parity per row.
    for r in rmin..rmax {
        let toggles = &scratch[(r - rmin) * stride..(r - rmin + 1) * stride];
        let mut inside = false;
        let mut start = 0;
        for (c, &t) in toggles.iter().enumerate().take(cols) {
            if t != 0 {
                if inside {
                    span(r, start, c);
                } else {
                    start = c;
                }
                inside = !inside;
            }
        }
        if inside {
            span(r, start, cols);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn picture(lines: &[&str]) -> String {
        let mut s = String::new();
        for l in lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    #[test]
    fn half_block_composition() {
        let mut c = Canvas::new(4, 1);
        // col 0: both red; col 1: top only; col 2: bottom only; col 3: two colours.
        c.set_point((0.0, 0.0), Colour::Red);
        c.set_point((0.0, 1.0), Colour::Red);
        c.set_point((1.5, 0.5), Colour::Green);
        c.set_point((2.0, 1.9), Colour::Blue);
        c.set_point((3.0, 0.0), Colour::Yellow);
        c.set_point((3.0, 1.0), Colour::Cyan);
        assert_eq!(c.text(), " ▀▄▀\n");
        let cells = c.cells();
        assert_eq!(
            cells[0],
            Cell {
                ch: ' ',
                fg: None,
                bg: Some(Colour::Red),
                reverse: false
            }
        );
        assert_eq!((cells[1].fg, cells[1].bg), (Some(Colour::Green), None));
        assert_eq!((cells[2].fg, cells[2].bg), (Some(Colour::Blue), None));
        assert_eq!(
            (cells[3].fg, cells[3].bg),
            (Some(Colour::Yellow), Some(Colour::Cyan))
        );
        // Every half-block cell has a real foreground colour.
        for cell in cells {
            if cell.ch != ' ' {
                assert!(cell.fg.is_some());
            }
        }
    }

    #[test]
    fn lines_in_all_directions() {
        let mut c = Canvas::new(7, 4);
        c.line((0.0, 0.0), (6.0, 6.0), Colour::Red, LinePattern::Solid);
        c.line((6.0, 0.0), (0.0, 0.0), Colour::Green, LinePattern::Solid);
        c.line((0.5, 7.5), (6.5, 7.5), Colour::Blue, LinePattern::Solid);
        c.line((6.0, 7.0), (6.0, 1.0), Colour::White, LinePattern::Solid);
        assert_eq!(
            c.pixel_picture(),
            picture(&[
                "GGGGGGG", ".R....W", "..R...W", "...R..W", "....R.W", ".....RW", "......W",
                "BBBBBBW",
            ])
        );
    }

    #[test]
    fn lines_are_clipped() {
        let mut c = Canvas::new(5, 2);
        // Enters on the left, leaves at the right edge.
        c.line((-10.0, 1.5), (20.0, 1.5), Colour::Red, LinePattern::Solid);
        // Entirely outside: above, below, left, right.
        c.line((-3.0, -1.0), (10.0, -0.1), Colour::Blue, LinePattern::Solid);
        c.line((-3.0, 4.0), (10.0, 9.0), Colour::Blue, LinePattern::Solid);
        c.line((-3.0, 0.0), (-0.01, 3.0), Colour::Blue, LinePattern::Solid);
        c.line((5.0, 0.0), (9.0, 3.0), Colour::Blue, LinePattern::Solid);
        // A diagonal entering on the left, leaving at the top.
        c.line((-2.5, 6.0), (4.5, -1.0), Colour::Green, LinePattern::Solid);
        assert_eq!(
            c.pixel_picture(),
            picture(&["...G.", "RRGRR", ".G...", "G...."])
        );
    }

    #[test]
    fn dashes_and_dots() {
        let mut c = Canvas::new(12, 1);
        c.line(
            (0.0, 0.0),
            (11.0, 0.0),
            Colour::Magenta,
            LinePattern::Dashed,
        );
        c.line((0.0, 1.0), (11.0, 1.0), Colour::White, LinePattern::Dotted);
        assert_eq!(
            c.pixel_picture(),
            picture(&["MMM..MMM..MM", "W..W..W..W.."])
        );
        assert_eq!(c.text(), "▀▀▀▄ ▀▀▀ ▄▀▀\n");
    }

    #[test]
    fn dashes_continue_across_segments() {
        // A line cut into 1-pixel segments looks the same as one segment.
        let mut one = Canvas::new(30, 1);
        one.line((0.0, 0.5), (29.0, 0.5), Colour::Cyan, LinePattern::Dashed);
        let mut many = Canvas::new(30, 1);
        let pts: Vec<Point> = (0..30).map(|x| (f64::from(x), 0.5)).collect();
        many.polyline(&pts, Colour::Cyan, LinePattern::Dashed);
        assert_eq!(one.pixel_picture(), many.pixel_picture());
        // Even finer segments (0.25 px) still show gaps and dashes.
        let mut fine = Canvas::new(30, 1);
        let pts: Vec<Point> = (0..=116).map(|i| (f64::from(i) * 0.25, 0.5)).collect();
        fine.polyline(&pts, Colour::Cyan, LinePattern::Dashed);
        let row: String = fine
            .pixel_picture()
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        assert!(row.contains("CCC..") && row.contains("..CCC"), "{row}");
        // The off-canvas part counts: the phase on the canvas depends only
        // on the distance from the start.
        let mut off = Canvas::new(30, 1);
        off.polyline(
            &[(-1001.0, 0.5), (-501.0, 0.5), (29.0, 0.5)],
            Colour::Cyan,
            LinePattern::Dashed,
        );
        let mut direct = Canvas::new(30, 1);
        direct.line(
            (-1001.0, 0.5),
            (29.0, 0.5),
            Colour::Cyan,
            LinePattern::Dashed,
        );
        assert_eq!(off.pixel_picture(), direct.pixel_picture());
    }

    #[test]
    fn a_start_phase_shifts_the_pattern() {
        let mut c = Canvas::new(12, 1);
        c.polyline_from(
            &[(0.0, 0.0), (11.0, 0.0)],
            Colour::Magenta,
            LinePattern::Dashed,
            2.0,
        );
        // Phase 2 is the last "on" pixel of the period 3 on, 2 off.
        c.polyline_from(
            &[(0.0, 1.0), (11.0, 1.0)],
            Colour::White,
            LinePattern::Dotted,
            -1.0,
        );
        assert_eq!(
            c.pixel_picture(),
            picture(&["M..MMM..MMM.", ".W..W..W..W."])
        );
        // A phase of a whole number of periods, or a non-finite one, is 0.
        for phase in [5.0, -10.0, f64::NAN, f64::INFINITY] {
            let mut a = Canvas::new(12, 1);
            a.polyline_from(
                &[(0.0, 0.0), (11.0, 0.0)],
                Colour::Red,
                LinePattern::Dashed,
                phase,
            );
            let mut b = Canvas::new(12, 1);
            b.line((0.0, 0.0), (11.0, 0.0), Colour::Red, LinePattern::Dashed);
            assert_eq!(a.pixel_picture(), b.pixel_picture(), "{phase}");
        }
    }

    #[test]
    fn polygon_with_hole() {
        let mut c = Canvas::new(8, 4);
        let outer = vec![(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)];
        // Hole with the same winding as the exterior: even-odd still holes it.
        let hole = vec![(2.0, 3.0), (6.0, 3.0), (6.0, 5.0), (2.0, 5.0)];
        c.fill_polygon(&[outer, hole], Colour::Green);
        assert_eq!(
            c.pixel_picture(),
            picture(&[
                "GGGGGGGG", "GGGGGGGG", "GGGGGGGG", "GG....GG", "GG....GG", "GGGGGGGG", "GGGGGGGG",
                "GGGGGGGG",
            ])
        );
        assert_eq!(c.text(), "        \n  ▀▀▀▀  \n  ▄▄▄▄  \n        \n");
        // Equal pixels are spaces with that background.
        assert_eq!(c.cell(0, 0).bg, Some(Colour::Green));
        assert_eq!(c.cell(3, 1).fg, Some(Colour::Green));
        assert_eq!(c.cell(3, 1).bg, None);
    }

    #[test]
    fn triangle_and_two_exteriors() {
        let mut c = Canvas::new(9, 3);
        let tri = vec![(0.0, 0.0), (9.0, 0.0), (0.0, 6.0)];
        c.fill_polygon(&[tri], Colour::Red);
        assert_eq!(
            c.pixel_picture(),
            picture(&[
                "RRRRRRRR.",
                "RRRRRRR..",
                "RRRRR....",
                "RRRR.....",
                "RR.......",
                "R........",
            ])
        );
        let mut d = Canvas::new(6, 1);
        let a = vec![(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let b = vec![(4.0, 0.0), (6.0, 0.0), (6.0, 2.0), (4.0, 2.0), (4.0, 0.0)];
        d.fill_polygon(&[a, b], Colour::Blue);
        assert_eq!(d.pixel_picture(), picture(&["BB..BB", "BB..BB"]));
    }

    #[test]
    fn polygon_far_outside_and_covering() {
        let mut c = Canvas::new(4, 1);
        let huge = vec![(-1e12, -1e12), (1e12, -1e12), (1e12, 1e12), (-1e12, 1e12)];
        c.fill_polygon(std::slice::from_ref(&huge), Colour::Yellow);
        assert_eq!(c.pixel_picture(), picture(&["YYYY", "YYYY"]));
        // A hole bigger than the canvas empties it.
        let mut c = Canvas::new(4, 1);
        let hole = vec![(-1e6, -1e6), (1e6, -1e6), (1e6, 1e6), (-1e6, 1e6)];
        c.fill_polygon(&[huge, hole], Colour::Yellow);
        assert_eq!(c.pixel_picture(), picture(&["....", "...."]));
        // Off to one side.
        let mut c = Canvas::new(4, 1);
        let right = vec![(100.0, -5.0), (200.0, -5.0), (200.0, 9.0)];
        c.fill_polygon(&[right], Colour::Yellow);
        assert_eq!(c.pixel_picture(), picture(&["....", "...."]));
    }

    #[test]
    fn shading_inside_or_outside() {
        let mut c = Canvas::new(6, 2);
        c.fill(Colour::Blue);
        // Coverage: the left half (pixels x 0..3).
        let cov = vec![(0.0, 0.0), (3.0, 0.0), (3.0, 4.0), (0.0, 4.0)];
        c.shade_polygon(std::slice::from_ref(&cov), false, Colour::White);
        assert_eq!(c.text(), "   ░░░\n   ░░░\n");
        let cell = c.cell(4, 0);
        assert_eq!(
            (cell.fg, cell.bg),
            (Some(Colour::White), Some(Colour::Blue))
        );
        // Union of two packs, then the complement.
        let mut c = Canvas::new(6, 1);
        let mut mask = CellMask::new(6, 1);
        mask.add_polygon(&[vec![(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]]);
        mask.add_polygon(&[vec![(4.0, 0.0), (6.0, 0.0), (6.0, 2.0), (4.0, 2.0)]]);
        mask.invert();
        c.shade(&mask, Colour::White);
        assert_eq!(c.text(), "  ░░  \n");
        // Shading over a pixel of the shade colour stays visible.
        let mut c = Canvas::new(1, 1);
        c.fill(Colour::White);
        c.shade_polygon(std::slice::from_ref(&cov), true, Colour::White);
        assert_eq!(c.cell(0, 0).fg, Some(Colour::Black));
        // Detail under the shading (a coast, a border) is not hidden, and
        // shading over nothing drawn is still visible.
        let mut c = Canvas::new(3, 1);
        c.line((0.0, 1.0), (2.0, 1.0), Colour::Magenta, LinePattern::Solid);
        c.set_point((2.0, 0.0), Colour::Magenta);
        c.shade_polygon(
            &[vec![(-1.0, -1.0), (9.0, -1.0), (9.0, 9.0)]],
            true,
            Colour::White,
        );
        c.shade_polygon(&[cov], false, Colour::White);
        let mut d = Canvas::new(1, 1);
        d.shade_polygon(
            &[vec![(-1.0, -1.0), (9.0, -1.0), (9.0, 9.0), (-1.0, 9.0)]],
            true,
            Colour::White,
        );
        // Column 0 is not shaded, column 1 has detail, column 2 is flat.
        assert_eq!(c.text(), "▄▄░\n");
        assert_eq!(c.cell(2, 0).bg, Some(Colour::Magenta));
        assert_eq!((d.cell(0, 0).ch, d.cell(0, 0).bg), ('░', None));
    }

    #[test]
    fn overlays_replace_cells_and_are_sanitised() {
        let mut c = Canvas::new(8, 2);
        c.fill(Colour::Blue);
        c.put_text(-2, 0, "abcdef", Some(Colour::White), None, false);
        c.put_text(
            6,
            1,
            "x\u{1b}é東",
            Some(Colour::Red),
            Some(Colour::Black),
            true,
        );
        c.put_char(3, 1, '+', None, None, true);
        c.put_char(30, 1, '+', None, None, true);
        c.put_text(0, 9, "no", None, None, false);
        assert_eq!(c.text(), "cdef    \n   +  x?\n");
        assert!(c.cell(3, 1).reverse);
        assert_eq!(c.cell(0, 0).fg, Some(Colour::White));
        assert!(c.has_overlay(7, 1) && !c.has_overlay(5, 1));
        c.clear();
        assert_eq!(c.text(), "        \n        \n");
    }

    #[test]
    fn hostile_geometry_is_fast_and_safe() {
        let start = Instant::now();
        let mut c = Canvas::new(80, 24);
        let bad = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            -0.0,
            i64::MAX as f64,
            i64::MIN as f64,
            f64::from(i32::MAX),
            f64::from(i32::MIN),
            1e300,
            -1e300,
            40.0,
        ];
        for pattern in [LinePattern::Solid, LinePattern::Dashed, LinePattern::Dotted] {
            for &x0 in &bad {
                for &y0 in &bad {
                    for &x1 in &bad {
                        c.polyline(
                            &[(x0, y0), (x1, 10.0), (x1, y0), (5.0, 5.0)],
                            Colour::Red,
                            pattern,
                        );
                    }
                }
            }
        }
        for &x0 in &bad {
            for &y0 in &bad {
                c.fill_polygon(
                    &[vec![(x0, y0), (10.0, 10.0), (y0, x0), (-x0, 3.0)]],
                    Colour::Green,
                );
                c.set_point((x0, y0), Colour::Blue);
                let mut m = CellMask::new(80, 24);
                m.add_polygon(&[vec![(x0, y0), (70.0, 40.0), (3.0, x0)]]);
            }
        }
        // Far left to far right through the canvas: drawn, and instantly.
        let mut c = Canvas::new(80, 24);
        c.line(
            (-1e300, 20.5),
            (1e300, 20.5),
            Colour::Red,
            LinePattern::Dashed,
        );
        c.line(
            (i64::MIN as f64, 3.0),
            (i64::MAX as f64, 3.0),
            Colour::Red,
            LinePattern::Solid,
        );
        let pic = c.pixel_picture();
        let rows: Vec<&str> = pic.lines().collect();
        assert_eq!(rows[3], "R".repeat(80));
        assert!(rows[20].contains("RRR..") && rows[20].contains("..RRR"));
        // A polygon with a huge vertex count (a zigzag across the canvas).
        let zig: Vec<Point> = (0..20_000)
            .map(|i| (f64::from(i % 80), if i % 2 == 0 { -1e9 } else { 1e9 }))
            .collect();
        c.fill_polygon(std::slice::from_ref(&zig), Colour::Blue);
        c.polyline(&zig, Colour::White, LinePattern::Dotted);
        // Nearly flat edges between far-apart points keep the parity right.
        let mut flat = Canvas::new(4, 1);
        let thin = vec![
            (-1e12, 0.25),
            (1e12, 0.25 + 1e-15),
            (1e12, 1.5),
            (-1e12, 1.5),
        ];
        flat.fill_polygon(&[thin], Colour::Red);
        assert_eq!(flat.pixel_picture(), picture(&["RRRR", "...."]));
        // Degenerate shapes: nothing drawn, nothing panics.
        let mut d = Canvas::new(5, 2);
        d.fill_polygon::<Vec<Point>>(&[], Colour::Red);
        d.fill_polygon(&[vec![(1.0, 1.0)]], Colour::Red);
        d.fill_polygon(&[vec![(1.0, 1.0), (3.0, 1.0), (4.0, 1.0)]], Colour::Red);
        d.fill_polygon(&[vec![(1.0, 0.0), (1.0, 4.0)]], Colour::Red);
        d.polyline(&[], Colour::Red, LinePattern::Solid);
        d.polyline(&[(2.0, 2.0)], Colour::Red, LinePattern::Solid);
        assert_eq!(
            d.pixel_picture(),
            picture(&[".....", ".....", ".....", "....."])
        );
        // A zero-length segment is one pixel.
        d.line((2.0, 2.0), (2.0, 2.0), Colour::Red, LinePattern::Solid);
        assert_eq!(d.pixel(2, 2), Some(Colour::Red));
        // Zero-sized and oversized canvases.
        let mut z = Canvas::new(0, 0);
        z.line((0.0, 0.0), (10.0, 10.0), Colour::Red, LinePattern::Solid);
        z.fill_polygon(&[vec![(0.0, 0.0), (9.0, 0.0), (0.0, 9.0)]], Colour::Red);
        assert_eq!(z.text(), "");
        let big = Canvas::new(u16::MAX, u16::MAX);
        assert_eq!((big.cols(), big.rows()), (1024, 512));
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn output_is_console_safe() {
        let mut c = Canvas::new(20, 5);
        c.fill(Colour::Blue);
        c.fill_polygon(&[vec![(2.0, 1.0), (15.0, 3.0), (5.0, 9.0)]], Colour::Black);
        c.line((0.0, 0.0), (19.0, 9.0), Colour::Red, LinePattern::Dashed);
        c.shade_polygon(
            &[vec![(0.0, 0.0), (5.0, 0.0), (5.0, 10.0)]],
            true,
            Colour::White,
        );
        c.put_text(
            1,
            1,
            "Zürich \u{7}\u{202e}",
            Some(Colour::White),
            None,
            false,
        );
        for cell in c.cells() {
            assert!(is_console_char(cell.ch), "{:?}", cell.ch);
        }
        assert!(!is_console_char('é') && !is_console_char('\n') && is_console_char('░'));
    }
}
