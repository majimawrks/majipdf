//! Own PDF -> XLSX engine ("engine b", spike iteration 5): pdfium text + ruling-line lattice -> rust_xlsxwriter.
//! Ported from `_spike/rust/src/main.rs` with the heuristics untouched. See _docs/excel-contract.md.
use pdfium_render::prelude::*;
use rust_xlsxwriter::{Format, FormatAlign, FormatBorder, FormatUnderline, Image, Workbook, Worksheet};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub const CANCELLED: &str = "cancelled";

#[derive(Clone, Copy, PartialEq)]
pub enum SheetMode {
    PerPage,
    One,
}

pub struct Options {
    /// Opt-in "Keep numbers as numbers"; false = every cell is the exact source text.
    pub numbers: bool,
    pub sheet_mode: SheetMode,
}

pub struct Stats {
    pub pages: usize,
    /// Pages without a text layer (pure scans): their sheet stays empty.
    pub empty_pages: usize,
}

// ---------- pdf2xlsx ("engine b"), iteration 3: ruling-line lattice tables +
// content-based page setup, source-shaped number formats, borders, alignment, fonts ----------

/// A word built from contiguous chars (join gap < 0.25 * line height, split on spaces).
/// Coordinates are top-down (row 0 = top of page), matching the rest of this module.
struct Word {
    text: String,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    line_rank: usize,
    /// PDF font size (pt) of the char that started this word. Words are typically
    /// font-uniform in generated PDFs, so one sample is enough.
    font_size: f64,
    bold: bool,
    /// Raw pdfium font name (may carry a subset prefix like "ABCDEF+"), same one-sample basis.
    font_name: String,
}

/// A thin ruling segment merged from path-object bounds: horizontal (pos=y, lo/hi=x range)
/// or vertical (pos=x, lo/hi=y range). `thickness` is the segment's short-axis size in pt.
#[derive(Clone, Copy)]
struct Seg {
    pos: f64,
    lo: f64,
    hi: f64,
    thickness: f64,
}

const THIN_PT: f64 = 4.0; // max short-axis size (pt) to treat a path object as a ruling line, not a filled box.
const LINE_TOL: f64 = 2.0;
const GAP_SPLIT_PT: f64 = 40.0;
// Border style thresholds (pt), measured against IKPA/Bon ruling segments with MAJIPDF_DEBUG_GEOM=1.
const MEDIUM_BORDER_PT: f64 = 1.2;
const THICK_BORDER_PT: f64 = 2.2;

/// Merges same-position segments (within LINE_TOL) and overlapping/touching ranges on that line,
/// keeping the thickest contributing segment for each merged range.
fn merge_segments(mut segs: Vec<Seg>) -> Vec<Seg> {
    segs.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap());
    let mut pos_groups: Vec<Vec<Seg>> = Vec::new();
    for s in segs {
        match pos_groups.last_mut() {
            Some(g) if (s.pos - g[0].pos).abs() <= LINE_TOL => g.push(s),
            _ => pos_groups.push(vec![s]),
        }
    }
    let mut out = Vec::new();
    for group in pos_groups {
        let pos = group.iter().map(|s| s.pos).sum::<f64>() / group.len() as f64;
        let mut ranges: Vec<(f64, f64, f64)> =
            group.iter().map(|s| (s.lo, s.hi, s.thickness)).collect();
        ranges.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut merged: Vec<(f64, f64, f64)> = Vec::new();
        for (lo, hi, th) in ranges {
            match merged.last_mut() {
                Some(last) if lo <= last.1 + LINE_TOL => {
                    last.1 = last.1.max(hi);
                    last.2 = last.2.max(th);
                }
                _ => merged.push((lo, hi, th)),
            }
        }
        for (lo, hi, th) in merged {
            out.push(Seg { pos, lo, hi, thickness: th });
        }
    }
    out
}

/// True if some horizontal segment at `y` (within LINE_TOL) spans [x0, x1] (small slack allowed).
fn horiz_covers(horiz: &[Seg], y: f64, x0: f64, x1: f64) -> bool {
    horiz
        .iter()
        .any(|s| (s.pos - y).abs() <= LINE_TOL && s.lo <= x0 + LINE_TOL && s.hi >= x1 - LINE_TOL)
}

/// True if some vertical segment at `x` (within LINE_TOL) spans [y0, y1] (small slack allowed).
fn vert_covers(vert: &[Seg], x: f64, y0: f64, y1: f64) -> bool {
    vert.iter()
        .any(|s| (s.pos - x).abs() <= LINE_TOL && s.lo <= y0 + LINE_TOL && s.hi >= y1 - LINE_TOL)
}

/// Max thickness among horizontal segments at `y` that span [x0, x1], if any (feeds borders).
fn horiz_thickness(horiz: &[Seg], y: f64, x0: f64, x1: f64) -> Option<f64> {
    horiz
        .iter()
        .filter(|s| (s.pos - y).abs() <= LINE_TOL && s.lo <= x0 + LINE_TOL && s.hi >= x1 - LINE_TOL)
        .map(|s| s.thickness)
        .fold(None, |acc: Option<f64>, t| Some(acc.map_or(t, |a| a.max(t))))
}

/// Max thickness among vertical segments at `x` that span [y0, y1], if any (feeds borders).
fn vert_thickness(vert: &[Seg], x: f64, y0: f64, y1: f64) -> Option<f64> {
    vert.iter()
        .filter(|s| (s.pos - x).abs() <= LINE_TOL && s.lo <= y0 + LINE_TOL && s.hi >= y1 - LINE_TOL)
        .map(|s| s.thickness)
        .fold(None, |acc: Option<f64>, t| Some(acc.map_or(t, |a| a.max(t))))
}

/// 0 = no border, 1 = thin (< MEDIUM_BORDER_PT), 2 = medium (< THICK_BORDER_PT), 3 = thick.
fn border_code(thickness: Option<f64>) -> u8 {
    match thickness {
        None => 0,
        Some(t) if t >= THICK_BORDER_PT => 3,
        Some(t) if t >= MEDIUM_BORDER_PT => 2,
        Some(_) => 1,
    }
}

fn border_style(code: u8) -> FormatBorder {
    match code {
        1 => FormatBorder::Thin,
        2 => FormatBorder::Medium,
        3 => FormatBorder::Thick,
        _ => FormatBorder::None,
    }
}

/// A lattice table detected from ruling lines: a grid of unique x/y boundaries, plus a
/// union-find over grid cells that records which adjacent cells have no separating line
/// (i.e. are merged in the source PDF).
struct Table {
    xs: Vec<f64>, // ncols+1 vertical boundaries, ascending
    ys: Vec<f64>, // nrows+1 horizontal boundaries, ascending (top-down)
    parent: Vec<usize>,
}

impl Table {
    fn ncols(&self) -> usize {
        self.xs.len() - 1
    }
    fn nrows(&self) -> usize {
        self.ys.len() - 1
    }
    fn cell(&self, r: usize, c: usize) -> usize {
        r * self.ncols() + c
    }
    fn find(&mut self, i: usize) -> usize {
        if self.parent[i] != i {
            self.parent[i] = self.find(self.parent[i]);
        }
        self.parent[i]
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
    /// Column index whose [xs[c], xs[c+1]) range contains (or is nearest to) `x`.
    fn col_of_x(&self, x: f64) -> usize {
        if x < self.xs[0] {
            return 0;
        }
        for c in 0..self.ncols() {
            if x < self.xs[c + 1] {
                return c;
            }
        }
        self.ncols() - 1
    }
    fn row_of_y(&self, y: f64) -> usize {
        if y < self.ys[0] {
            return 0;
        }
        for r in 0..self.nrows() {
            if y < self.ys[r + 1] {
                return r;
            }
        }
        self.nrows() - 1
    }
    /// Merge-region (r0,c0,r1,c1) for the cell containing (r,c).
    fn region_of(&mut self, r: usize, c: usize) -> (usize, usize, usize, usize) {
        let root = self.find(self.cell(r, c));
        let (mut r0, mut c0, mut r1, mut c1) = (r, c, r, c);
        for rr in 0..self.nrows() {
            for cc in 0..self.ncols() {
                if self.find(self.cell(rr, cc)) == root {
                    r0 = r0.min(rr);
                    c0 = c0.min(cc);
                    r1 = r1.max(rr);
                    c1 = c1.max(cc);
                }
            }
        }
        (r0, c0, r1, c1)
    }
}

/// Groups positions within LINE_TOL of each other, returning (running-mean position, count).
fn cluster_positions(mut positions: Vec<f64>) -> Vec<(f64, u32)> {
    positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut out: Vec<(f64, u32)> = Vec::new();
    for p in positions {
        match out.last_mut() {
            Some(c) if (p - c.0).abs() <= LINE_TOL => {
                c.0 = (c.0 * c.1 as f64 + p) / (c.1 + 1) as f64;
                c.1 += 1;
            }
            _ => out.push((p, 1)),
        }
    }
    out
}

/// Snaps a set of segment positions to unique boundaries (within LINE_TOL).
fn unique_positions(mut positions: Vec<f64>) -> Vec<f64> {
    positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut out: Vec<f64> = Vec::new();
    for p in positions {
        match out.last() {
            Some(&last) if (p - last).abs() <= LINE_TOL => {}
            _ => out.push(p),
        }
    }
    out
}

/// Collects thin horizontal/vertical path-object segments on the page (ruling lines), in
/// top-down pt coordinates. Shared by table detection and the page's content bounding box.
fn page_path_segments(page: &PdfPage, page_h: f64) -> (Vec<Seg>, Vec<Seg>) {
    let mut horiz_raw = Vec::new();
    let mut vert_raw = Vec::new();

    for object in page.objects().iter() {
        if object.object_type() != PdfPageObjectType::Path {
            continue;
        }
        let Ok(b) = object.bounds() else { continue };
        let r = b.to_rect();
        let left = r.left().value as f64;
        let right = r.right().value as f64;
        let top = page_h - r.top().value as f64;
        let bottom = page_h - r.bottom().value as f64;
        let width = right - left;
        let height = bottom - top;

        if width <= THIN_PT && height > THIN_PT {
            vert_raw.push(Seg {
                pos: (left + right) / 2.0,
                lo: top,
                hi: bottom,
                thickness: width.max(0.25),
            });
        } else if height <= THIN_PT && width > THIN_PT {
            horiz_raw.push(Seg {
                pos: (top + bottom) / 2.0,
                lo: left,
                hi: right,
                thickness: height.max(0.25),
            });
        }
    }

    (horiz_raw, vert_raw)
}

/// Detects a lattice table from merged ruling-line segments. Returns the table plus the merged
/// horizontal/vertical segments (reused later to draw cell borders).
/// ponytail: page-wide single table, not clustered into separate regions. Upgrade path if a
/// page ever has two independent tables side by side: cluster segments by connectivity first.
fn detect_table(horiz_raw: Vec<Seg>, vert_raw: Vec<Seg>) -> Option<(Table, Vec<Seg>, Vec<Seg>)> {
    let horiz = merge_segments(horiz_raw);
    let vert = merge_segments(vert_raw);
    if horiz.len() < 2 || vert.len() < 2 {
        return None;
    }

    let xs = unique_positions(vert.iter().map(|s| s.pos).collect());
    let ys = unique_positions(horiz.iter().map(|s| s.pos).collect());
    if xs.len() < 2 || ys.len() < 2 {
        return None;
    }

    let ncols = xs.len() - 1;
    let nrows = ys.len() - 1;
    let mut table = Table {
        xs,
        ys,
        parent: (0..nrows * ncols).collect(),
    };

    for r in 0..nrows {
        for c in 0..ncols.saturating_sub(1) {
            if !vert_covers(&vert, table.xs[c + 1], table.ys[r], table.ys[r + 1]) {
                let a = table.cell(r, c);
                let b = table.cell(r, c + 1);
                table.union(a, b);
            }
        }
    }
    for r in 0..nrows.saturating_sub(1) {
        for c in 0..ncols {
            if !horiz_covers(&horiz, table.ys[r + 1], table.xs[c], table.xs[c + 1]) {
                let a = table.cell(r, c);
                let b = table.cell(r + 1, c);
                table.union(a, b);
            }
        }
    }

    Some((table, horiz, vert))
}

/// Builds words from page chars: cluster into lines by vertical center, then within each line
/// join chars whose gap is small (< 0.25 * line height) and split on real whitespace.
fn build_words(page: &PdfPage, page_h: f64) -> Vec<Word> {
    struct Ch {
        ch: char,
        left: f64,
        right: f64,
        top: f64,
        bottom: f64,
        font_size: f64,
        bold: bool,
        font_name: String,
    }

    let mut chars = Vec::new();
    if let Ok(text) = page.text() {
        for c in text.chars().iter() {
            let Some(ch) = c.unicode_char() else { continue };
            let Ok(b) = c.loose_bounds() else { continue };
            let font_size = c.scaled_font_size().value as f64;
            let font_name = c.font_name();
            let bold = font_name.to_lowercase().contains("bold");
            chars.push(Ch {
                ch,
                left: b.left().value as f64,
                right: b.right().value as f64,
                top: page_h - b.top().value as f64,
                bottom: page_h - b.bottom().value as f64,
                font_size,
                bold,
                font_name,
            });
        }
    }
    if chars.is_empty() {
        return Vec::new();
    }

    let mut avg_height =
        chars.iter().map(|c| c.bottom - c.top).sum::<f64>() / chars.len() as f64;
    if avg_height <= 0.0 {
        avg_height = 10.0;
    }
    let row_tol = avg_height * 0.5;

    let mut order: Vec<usize> = (0..chars.len()).collect();
    order.sort_by(|&a, &b| {
        let ca = (chars[a].top + chars[a].bottom) / 2.0;
        let cb = (chars[b].top + chars[b].bottom) / 2.0;
        ca.partial_cmp(&cb).unwrap()
    });

    let mut row_bands: Vec<(f64, f64)> = Vec::new();
    let mut row_of: Vec<usize> = vec![0; chars.len()];
    for &ci in &order {
        let center = (chars[ci].top + chars[ci].bottom) / 2.0;
        let mut placed = false;
        for (ri, band) in row_bands.iter_mut().enumerate() {
            if (center - band.0).abs() <= row_tol {
                band.0 = (band.0 * band.1 + center) / (band.1 + 1.0);
                band.1 += 1.0;
                row_of[ci] = ri;
                placed = true;
                break;
            }
        }
        if !placed {
            row_bands.push((center, 1.0));
            row_of[ci] = row_bands.len() - 1;
        }
    }
    let mut row_order: Vec<usize> = (0..row_bands.len()).collect();
    row_order.sort_by(|&a, &b| row_bands[a].0.partial_cmp(&row_bands[b].0).unwrap());
    let mut row_rank = vec![0usize; row_bands.len()];
    for (rank, &orig) in row_order.iter().enumerate() {
        row_rank[orig] = rank;
    }

    let mut by_line: HashMap<usize, Vec<usize>> = HashMap::new();
    for (ci, &row) in row_of.iter().enumerate() {
        by_line.entry(row_rank[row]).or_default().push(ci);
    }

    let mut words = Vec::new();
    for (&line_rank, idxs) in by_line.iter() {
        let mut idxs = idxs.clone();
        idxs.sort_by(|&a, &b| chars[a].left.partial_cmp(&chars[b].left).unwrap());
        let line_height = idxs
            .iter()
            .map(|&i| chars[i].bottom - chars[i].top)
            .fold(0.0, f64::max)
            .max(1.0);
        let gap_tol = line_height * 0.25;

        let mut cur = String::new();
        let mut cur_left = 0.0;
        let mut cur_right = 0.0;
        let mut cur_top = f64::MAX;
        let mut cur_bottom = f64::MIN;
        let mut cur_font_size = 11.0f64;
        let mut cur_bold = false;
        let mut cur_font_name = String::new();
        let mut prev_right: Option<f64> = None;

        let flush = |cur: &mut String,
                          cur_left: f64,
                          cur_right: f64,
                          cur_top: f64,
                          cur_bottom: f64,
                          cur_font_size: f64,
                          cur_bold: bool,
                          cur_font_name: &str,
                          words: &mut Vec<Word>| {
            if !cur.is_empty() {
                words.push(Word {
                    text: std::mem::take(cur),
                    left: cur_left,
                    right: cur_right,
                    top: cur_top,
                    bottom: cur_bottom,
                    line_rank,
                    font_size: cur_font_size,
                    bold: cur_bold,
                    font_name: cur_font_name.to_string(),
                });
            }
        };

        for &ci in &idxs {
            let c = &chars[ci];
            if c.ch.is_whitespace() {
                flush(
                    &mut cur, cur_left, cur_right, cur_top, cur_bottom, cur_font_size, cur_bold,
                    &cur_font_name, &mut words,
                );
                prev_right = None;
                continue;
            }
            let is_new_word = match prev_right {
                Some(pr) => c.left - pr > gap_tol,
                None => true,
            };
            if is_new_word {
                flush(
                    &mut cur, cur_left, cur_right, cur_top, cur_bottom, cur_font_size, cur_bold,
                    &cur_font_name, &mut words,
                );
                cur_left = c.left;
                cur_top = f64::MAX;
                cur_bottom = f64::MIN;
                cur_font_size = c.font_size;
                cur_bold = c.bold;
                cur_font_name = c.font_name.clone();
            }
            cur.push(c.ch);
            cur_right = c.right;
            cur_top = cur_top.min(c.top);
            cur_bottom = cur_bottom.max(c.bottom);
            prev_right = Some(c.right);
        }
        flush(
            &mut cur, cur_left, cur_right, cur_top, cur_bottom, cur_font_size, cur_bold,
            &cur_font_name, &mut words,
        );
    }

    words
}

// ---------- geometry helpers: column grid, row layout, style + format cache ----------

/// Excel column width (character units, Calibri 11 default: 7px max digit width + 5px padding)
/// from a PDF-point column width. Matches `rust_xlsxwriter::set_column_width`'s own internal
/// px<->width conversion, so what we compute here round-trips through the xlsx `<col>` width.
/// PDF points -> pixels at 96 DPI (Excel's own default), for image placement/sizing.
fn pt_to_px(pt: f64) -> u32 {
    (pt * 96.0 / 72.0).round().max(0.0) as u32
}

/// Row index and pt offset within that row for a top-down y-coordinate, from the row_tops/
/// row_heights recorded while laying out rows. Clamps to the first/last row when out of range.
fn row_for_y(y: f64, row_tops: &[f64], row_heights: &[f64]) -> (u32, f64) {
    if row_tops.is_empty() {
        return (0, 0.0);
    }
    for i in 0..row_tops.len() {
        let top = row_tops[i];
        let bottom = top + row_heights[i];
        if y < bottom || i == row_tops.len() - 1 {
            return (i as u32, (y - top).max(0.0));
        }
    }
    (0, 0.0)
}

fn pt_to_excel_col_width(pt: f64) -> f64 {
    let px = pt * 96.0 / 72.0;
    ((px - 5.0) / 7.0).max(0.5)
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Column x-boundaries for a page: the table's own vertical grid lines (untouched, so its
/// merges/col_of_x keep working) extended left/right by the starting x of outside-text groups
/// that fall outside the table's x-range (margin column, side-by-side signature blocks). With
/// no table, boundaries are the starting x of every >40pt-gap group; with none of those either,
/// a single column spans the text width.
struct ColGrid {
    xs: Vec<f64>,
    /// Worksheet column index of the table's own column 0 (i.e. how many extra "left" columns
    /// were inserted before the table's grid). 0 when there's no table.
    table_col_offset: usize,
}

impl ColGrid {
    fn ncols(&self) -> usize {
        self.xs.len().saturating_sub(1).max(1)
    }
    fn col_of_x(&self, x: f64) -> usize {
        if self.xs.len() < 2 {
            return 0;
        }
        if x < self.xs[0] {
            return 0;
        }
        for c in 0..self.ncols() {
            if x < self.xs[c + 1] {
                return c;
            }
        }
        self.ncols() - 1
    }
    fn build(table: &Option<Table>, group_starts: &[f64], words: &[Word]) -> ColGrid {
        if let Some(t) = table {
            let tx0 = t.xs[0];
            let tx1 = *t.xs.last().unwrap();
            let left_extra = unique_positions(
                group_starts.iter().copied().filter(|&x| x < tx0 - LINE_TOL).collect(),
            );
            let right_extra = unique_positions(
                group_starts.iter().copied().filter(|&x| x > tx1 + LINE_TOL).collect(),
            );
            let table_col_offset = left_extra.len();
            let mut xs = Vec::new();
            xs.extend(left_extra);
            xs.extend(t.xs.iter().copied());
            xs.extend(right_extra.iter().copied());
            if !right_extra.is_empty() {
                let max_right = words.iter().map(|w| w.right).fold(tx1, f64::max);
                let last = *xs.last().unwrap();
                xs.push(max_right.max(last + 20.0));
            }
            ColGrid { xs, table_col_offset }
        } else {
            let mut xs = unique_positions(group_starts.to_vec());
            if xs.is_empty() {
                if words.is_empty() {
                    return ColGrid { xs: vec![0.0, 100.0], table_col_offset: 0 };
                }
                let min_l = words.iter().map(|w| w.left).fold(f64::MAX, f64::min);
                let max_r = words.iter().map(|w| w.right).fold(f64::MIN, f64::max);
                return ColGrid {
                    xs: vec![min_l, max_r.max(min_l + 20.0)],
                    table_col_offset: 0,
                };
            }
            let max_r = words.iter().map(|w| w.right).fold(*xs.last().unwrap(), f64::max);
            let last = *xs.last().unwrap();
            xs.push(max_r.max(last + 20.0));
            ColGrid { xs, table_col_offset: 0 }
        }
    }
}

/// Dominant (most frequent) font size, majority-bold flag, and majority font family/italic
/// (mapped from the PDF font name) among the words that make up a cell.
fn dominant_style(words: &[Word], idxs: &[usize]) -> (f64, bool, String, bool) {
    if idxs.is_empty() {
        return (11.0, false, "Arial".to_string(), false);
    }
    let mut size_counts: HashMap<i64, u32> = HashMap::new();
    let mut name_counts: HashMap<&str, u32> = HashMap::new();
    let mut bold_count = 0u32;
    for &wi in idxs {
        let key = (words[wi].font_size * 2.0).round() as i64;
        *size_counts.entry(key).or_insert(0) += 1;
        *name_counts.entry(words[wi].font_name.as_str()).or_insert(0) += 1;
        if words[wi].bold {
            bold_count += 1;
        }
    }
    let best_size = size_counts
        .iter()
        .max_by_key(|&(&k, &v)| (v, std::cmp::Reverse(k)))
        .map(|(&k, _)| k)
        .unwrap_or(22);
    let size = (best_size as f64 / 2.0).max(1.0);
    let bold = bold_count * 2 >= idxs.len() as u32;
    let best_name = name_counts
        .iter()
        .max_by_key(|&(_, &v)| v)
        .map(|(&k, _)| k)
        .unwrap_or("");
    let (family, italic) = map_font_name(best_name);
    (size, bold, family, italic)
}

/// Majority vote: does the cell's text mostly come from lines flagged as underlined?
fn dominant_underline(words: &[Word], idxs: &[usize], underline_lines: &std::collections::HashSet<usize>) -> bool {
    if idxs.is_empty() {
        return false;
    }
    let hits = idxs.iter().filter(|&&wi| underline_lines.contains(&words[wi].line_rank)).count();
    hits * 2 >= idxs.len()
}

/// Maps a PDF font name (subset prefix and style suffixes included, e.g. "ABCDEF+Calibri-Bold")
/// to an installed Windows font family, plus whether the name reads as italic. Substring
/// matching makes explicit prefix/suffix stripping unnecessary for the mapping itself.
/// ponytail: fixed small lookup table, not a full font database; unmapped names fall back to Arial.
fn map_font_name(pdf_font: &str) -> (String, bool) {
    let lower = pdf_font.to_lowercase();
    let italic = lower.contains("italic") || lower.contains("oblique");
    let family = if lower.contains("helvetica") || lower.contains("arial") {
        "Arial"
    } else if lower.contains("times") {
        "Times New Roman"
    } else if lower.contains("bookman") {
        "Bookman Old Style"
    } else if lower.contains("calibri") {
        "Calibri"
    } else if lower.contains("tahoma") {
        "Tahoma"
    } else if lower.contains("courier") {
        "Courier New"
    } else {
        "Arial"
    };
    (family.to_string(), italic)
}

/// Horizontal alignment from a text x-span vs. its cell's (or column's) x-range: gaps roughly
/// equal -> center, text hugging the right edge -> right, otherwise left.
/// Codes: 0=left, 1=center, 2=right.
fn horiz_align_code(text_left: f64, text_right: f64, cell_left: f64, cell_right: f64) -> u8 {
    let cell_w = (cell_right - cell_left).max(1.0);
    let left_gap = (text_left - cell_left).max(0.0);
    let right_gap = (cell_right - text_right).max(0.0);
    let tol = (cell_w * 0.15).max(2.0);
    if (left_gap - right_gap).abs() <= tol {
        1
    } else if right_gap <= tol {
        2
    } else {
        0
    }
}

/// Vertical alignment from a text y-span vs. its cell's y-range.
/// Codes: 0=bottom (Excel's own default), 1=top, 2=center.
fn vert_align_code(text_top: f64, text_bottom: f64, cell_top: f64, cell_bottom: f64) -> u8 {
    let cell_h = (cell_bottom - cell_top).max(1.0);
    let top_gap = (text_top - cell_top).max(0.0);
    let bottom_gap = (cell_bottom - text_bottom).max(0.0);
    let tol = (cell_h * 0.15).max(2.0);
    if (top_gap - bottom_gap).abs() <= tol {
        2
    } else if top_gap <= tol {
        1
    } else {
        0
    }
}

/// Row-height plan for a top-to-bottom sequence of outside-table line tops. Each line's own row
/// height is the pitch to the next line's top; a gap far above the local typical pitch (>1.5x)
/// gets a normal-height row for the line plus a separate empty spacer row for the rest of the
/// gap, rather than stretching one content row to the full gap. Returns (is_spacer, height_pt)
/// entries in row order; the i-th non-spacer entry corresponds to `tops[i]`.
fn layout_with_gaps(
    tops: &[f64],
    lead_gap: Option<f64>,
    trail_gap: Option<f64>,
    typical: f64,
) -> Vec<(bool, f64)> {
    let mut out = Vec::new();
    if let Some(g) = lead_gap {
        if typical > 0.0 && g > 1.5 * typical {
            out.push((true, (g - typical).max(1.0)));
        }
    }
    let n = tops.len();
    for i in 0..n {
        let pitch = if i + 1 < n {
            tops[i + 1] - tops[i]
        } else if let Some(g) = trail_gap {
            g
        } else {
            typical.max(1.0)
        };
        if typical > 0.0 && pitch > 1.5 * typical {
            out.push((false, typical.max(1.0)));
            out.push((true, (pitch - typical).max(1.0)));
        } else {
            out.push((false, pitch.max(1.0)));
        }
    }
    out
}

/// Per-cell style, independent of text content: font, alignment, and the four edge borders.
struct CellStyle {
    size: f64,
    bold: bool,
    italic: bool,
    font_name: String,
    align_h: u8,       // 0=left/default, 1=center, 2=right
    align_v: u8,       // 0=bottom/default, 1=top, 2=center
    borders: [u8; 4],  // top, bottom, left, right: 0=none, 1=thin, 2=medium, 3=thick
    underline: bool,
}

impl CellStyle {
    fn plain(size: f64, bold: bool) -> Self {
        CellStyle {
            size,
            bold,
            italic: false,
            font_name: "Arial".to_string(),
            align_h: 0,
            align_v: 0,
            borders: [0; 4],
            underline: false,
        }
    }
}

/// Format cache keyed by every knob that affects the xlsx `<xf>` record, so repeated cells with
/// identical style share one `Format` instance.
struct FormatCache {
    cache: HashMap<(i64, bool, String, bool, bool, String, u8, u8, [u8; 4], bool), Format>,
}

impl FormatCache {
    fn new() -> Self {
        FormatCache { cache: HashMap::new() }
    }
    fn get(&mut self, wrap: bool, num_fmt: &str, style: &CellStyle) -> Format {
        let key = (
            (style.size * 2.0).round() as i64,
            wrap,
            num_fmt.to_string(),
            style.bold,
            style.italic,
            style.font_name.clone(),
            style.align_h,
            style.align_v,
            style.borders,
            style.underline,
        );
        self.cache
            .entry(key)
            .or_insert_with(|| {
                let mut f = Format::new()
                    .set_font_size(style.size)
                    .set_font_name(style.font_name.as_str());
                if wrap {
                    f = f.set_text_wrap();
                }
                if style.bold {
                    f = f.set_bold();
                }
                if style.italic {
                    f = f.set_italic();
                }
                if style.underline {
                    f = f.set_underline(FormatUnderline::Single);
                }
                if !num_fmt.is_empty() {
                    f = f.set_num_format(num_fmt);
                }
                f = match style.align_h {
                    1 => f.set_align(FormatAlign::Center),
                    2 => f.set_align(FormatAlign::Right),
                    _ => f,
                };
                f = match style.align_v {
                    1 => f.set_align(FormatAlign::Top),
                    2 => f.set_align(FormatAlign::VerticalCenter),
                    _ => f,
                };
                f.set_border_top(border_style(style.borders[0]))
                    .set_border_bottom(border_style(style.borders[1]))
                    .set_border_left(border_style(style.borders[2]))
                    .set_border_right(border_style(style.borders[3]))
            })
            .clone()
    }
}

/// Nearest standard Excel paper-size code from the PDF page's own dimensions, matched on
/// (short, long) edge regardless of orientation. Falls back to A4 (9) when nothing is close.
fn paper_size_code(page_w: f64, page_h: f64) -> u8 {
    let (short, long) = if page_w < page_h { (page_w, page_h) } else { (page_h, page_w) };
    let candidates: [(f64, f64, u8); 4] = [
        (595.0, 842.0, 9),  // A4
        (612.0, 792.0, 1),  // Letter
        (612.0, 1008.0, 5), // Legal
        (612.0, 936.0, 14), // Folio / F4 8.5x13in
    ];
    candidates
        .iter()
        .min_by(|a, b| {
            let da = (a.0 - short).abs() + (a.1 - long).abs();
            let db = (b.0 - short).abs() + (b.1 - long).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|c| c.2)
        .unwrap_or(9)
}

/// Page setup taken from a page's own geometry; applied to the sheet by `convert`.
struct PageSetup {
    landscape: bool,
    paper: u8,
    /// left, right, top, bottom, header, footer (inches)
    margins: [f64; 6],
}

struct PageOut {
    /// Worksheet rows this page occupies, starting at its row base.
    rows: u32,
    /// Column widths (Excel units), index = worksheet column.
    widths: Vec<f64>,
    setup: PageSetup,
}

/// Converts `doc` into `out`. `progress(stage, page, pages)` is called per page ("reading") and once
/// before the save ("writing"); `cancel` is polled per page and answers `Err(CANCELLED)`.
pub fn convert(
    doc: &PdfDocument,
    out: &Path,
    opts: &Options,
    progress: &dyn Fn(&'static str, usize, usize),
    cancel: &AtomicBool,
) -> Result<Stats, String> {
    let mut workbook = Workbook::new();
    let mut fc = FormatCache::new();
    let pages = doc.pages().len() as usize;
    let mut empty_pages = 0usize;

    // sheet_mode "one": all pages stacked on the single sheet "Pages".
    let mut one_sheet = (opts.sheet_mode == SheetMode::One).then(|| {
        let mut ws = Worksheet::new();
        ws.set_name("Pages").ok();
        ws
    });
    let mut next_row = 0u32;
    let mut breaks: Vec<u32> = Vec::new();
    let mut max_widths: Vec<f64> = Vec::new();
    let mut first_setup: Option<PageSetup> = None;

    for (page_idx, page) in doc.pages().iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        progress("reading", page_idx + 1, pages);

        match one_sheet.as_mut() {
            None => {
                let worksheet = workbook.add_worksheet();
                worksheet.set_name(format!("Page{}", page_idx + 1)).ok();
                match write_page(doc, &page, worksheet, 0, &mut fc, opts.numbers) {
                    None => empty_pages += 1,
                    Some(p) => {
                        for (c, w) in p.widths.iter().enumerate() {
                            worksheet.set_column_width(c as u16, *w).ok();
                        }
                        apply_setup(worksheet, &p.setup);
                        // Always-on safety net rather than a fragile "would it fit" threshold: Excel's own PDF
                        // export doesn't perfectly match our pt-based row-height math (rounding, reserved
                        // unprintable margin), so a page that our math says fits can still spill in practice.
                        // Row heights are geometry-accurate, so the scale-to-fit is a no-op or near-100% anyway.
                        worksheet.set_print_fit_to_pages(1, 1);
                    }
                }
            }
            Some(worksheet) => match write_page(doc, &page, worksheet, next_row, &mut fc, opts.numbers) {
                None => empty_pages += 1,
                Some(p) => {
                    if next_row > 0 {
                        breaks.push(next_row); // page break before this page's first row
                    }
                    next_row += p.rows + 1; // one empty row between pages
                    if max_widths.len() < p.widths.len() {
                        max_widths.resize(p.widths.len(), 0.0);
                    }
                    for (c, w) in p.widths.iter().enumerate() {
                        max_widths[c] = max_widths[c].max(*w);
                    }
                    first_setup.get_or_insert(p.setup);
                }
            },
        }
    }

    if let Some(mut worksheet) = one_sheet {
        for (c, w) in max_widths.iter().enumerate() {
            worksheet.set_column_width(c as u16, *w).ok();
        }
        if let Some(s) = &first_setup {
            apply_setup(&mut worksheet, s);
        }
        // Width only (1 wide, N tall): fitting the whole stack onto one page would be unreadable.
        worksheet.set_print_fit_to_pages(1, 0);
        breaks.truncate(1023); // Excel's limit
        worksheet.set_page_breaks(&breaks).ok();
        workbook.push_worksheet(worksheet);
    }

    progress("writing", pages, pages);
    workbook.save(out).map_err(|e| format!("failed to save {}: {e}", out.display()))?;
    Ok(Stats { pages, empty_pages })
}

fn apply_setup(worksheet: &mut Worksheet, s: &PageSetup) {
    if s.landscape {
        worksheet.set_landscape();
    }
    worksheet.set_paper_size(s.paper);
    let m = &s.margins;
    worksheet.set_margins(m[0], m[1], m[2], m[3], m[4], m[5]);
}

/// Writes one page at row `base` of `worksheet`. None = the page has no text layer (nothing written).
fn write_page(
    doc: &PdfDocument,
    page: &PdfPage,
    worksheet: &mut Worksheet,
    base: u32,
    fc: &mut FormatCache,
    numbers_mode: bool,
) -> Option<PageOut> {
        let page_w = page.width().value as f64;
        let page_h = page.height().value as f64;
        let words = build_words(&page, page_h);
        if words.is_empty() {
            return None;
        }
        let (mut horiz_raw, vert_raw) = page_path_segments(&page, page_h);


        // --- underline detection: a horizontal segment not connected to any vertical ruling
        // line, lying 0-4pt below a text line's bottom and overlapping its x-span >= 80%, is a
        // text underline, not a table rule. Pull it out of horiz_raw before table detection (this
        // also kills spurious merges caused by a title underline being read as a ruling line) and
        // remember which lines get an underlined font. ---
        let mut line_bounds: HashMap<usize, (f64, f64, f64)> = HashMap::new(); // line_rank -> (bottom, left, right)
        for w in &words {
            let e = line_bounds.entry(w.line_rank).or_insert((f64::MIN, f64::MAX, f64::MIN));
            e.0 = e.0.max(w.bottom);
            e.1 = e.1.min(w.left);
            e.2 = e.2.max(w.right);
        }
        let mut underline_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();
        horiz_raw.retain(|s| {
            let connected = vert_raw.iter().any(|v| {
                v.lo - LINE_TOL <= s.pos
                    && v.hi + LINE_TOL >= s.pos
                    && ((v.pos - s.lo).abs() <= LINE_TOL || (v.pos - s.hi).abs() <= LINE_TOL)
            });
            if connected {
                return true;
            }
            for (&lr, &(bottom, left, right)) in &line_bounds {
                // "0-4pt below the text line's bottom": char loose_bounds reserves descender
                // space below the visual baseline, so a real underline often sits a couple pt
                // *above* the measured bbox bottom (observed ~-2.6pt on real samples) -- widen
                // the window below zero to still catch it, not just the literal 0..4 range.
                let below = s.pos - bottom;
                if !(-4.0..=4.0 + LINE_TOL).contains(&below) {
                    continue;
                }
                let overlap = (s.hi.min(right) - s.lo.max(left)).max(0.0);
                let text_w = (right - left).max(1.0);
                if overlap / text_w >= 0.8 {
                    underline_lines.insert(lr);
                    return false;
                }
            }
            true
        });

        // --- content bounding box: every text char + every ruling line on the page. Margins
        // are derived from this, not from the table's own offset (an offset-derived margin left
        // huge blank margins on some samples and spilled a one-page table across many pages). ---
        let mut content_min_x = f64::MAX;
        let mut content_max_x = f64::MIN;
        let mut content_min_y = f64::MAX;
        let mut content_max_y = f64::MIN;
        for w in &words {
            content_min_x = content_min_x.min(w.left);
            content_max_x = content_max_x.max(w.right);
            content_min_y = content_min_y.min(w.top);
            content_max_y = content_max_y.max(w.bottom);
        }
        for s in &horiz_raw {
            content_min_x = content_min_x.min(s.lo);
            content_max_x = content_max_x.max(s.hi);
            content_min_y = content_min_y.min(s.pos);
            content_max_y = content_max_y.max(s.pos);
        }
        for s in &vert_raw {
            content_min_y = content_min_y.min(s.lo);
            content_max_y = content_max_y.max(s.hi);
            content_min_x = content_min_x.min(s.pos);
            content_max_x = content_max_x.max(s.pos);
        }

        let (mut table, table_horiz, table_vert) = match detect_table(horiz_raw, vert_raw) {
            Some((t, h, v)) => (Some(t), h, v),
            None => (None, Vec::new(), Vec::new()),
        };

        // Bucket words: inside the table grid vs. outside it.
        let mut table_cell_words: HashMap<usize, Vec<usize>> = HashMap::new(); // cell -> word idx
        let mut outside: Vec<usize> = Vec::new();
        if let Some(t) = &table {
            for (wi, w) in words.iter().enumerate() {
                let cx = (w.left + w.right) / 2.0;
                let cy = (w.top + w.bottom) / 2.0;
                if cx >= t.xs[0] && cx <= *t.xs.last().unwrap() && cy >= t.ys[0] && cy <= *t.ys.last().unwrap() {
                    let r = t.row_of_y(cy);
                    let c = t.col_of_x(cx);
                    table_cell_words.entry(t.cell(r, c)).or_default().push(wi);
                } else {
                    outside.push(wi);
                }
            }
        } else {
            outside = (0..words.len()).collect();
        }

        // --- classify outside-table lines as above/overlap/below the table ---
        let table_rows = table.as_ref().map(|t| t.nrows()).unwrap_or(0);
        let mut by_line: HashMap<usize, Vec<usize>> = HashMap::new();
        for &wi in &outside {
            by_line.entry(words[wi].line_rank).or_default().push(wi);
        }
        let mut line_ranks: Vec<usize> = by_line.keys().copied().collect();
        line_ranks.sort();

        let table_top = table.as_ref().map(|t| t.ys[0]);
        let table_bottom = table.as_ref().map(|t| *t.ys.last().unwrap());

        enum Placement {
            Above,
            Overlap, // vertically within the table's y-range but outside its x-range
            Below,
        }
        let mut placement: HashMap<usize, Placement> = HashMap::new();
        let mut line_top_of: HashMap<usize, f64> = HashMap::new();
        for &lr in &line_ranks {
            let idxs = &by_line[&lr];
            let line_top = idxs.iter().map(|&i| words[i].top).fold(f64::MAX, f64::min);
            line_top_of.insert(lr, line_top);
            let p = match (table_top, table_bottom) {
                (Some(tt), Some(tb)) => {
                    if line_top < tt {
                        Placement::Above
                    } else if line_top >= tb {
                        Placement::Below
                    } else {
                        Placement::Overlap
                    }
                }
                _ => Placement::Above, // no table: everything is one sequential list
            };
            placement.insert(lr, p);
        }

        // --- split each outside line into groups on >40pt x gaps; also feeds column geometry ---
        let mut groups_by_line: HashMap<usize, Vec<Vec<usize>>> = HashMap::new();
        for &lr in &line_ranks {
            let mut idxs = by_line[&lr].clone();
            idxs.sort_by(|&a, &b| words[a].left.partial_cmp(&words[b].left).unwrap());
            let mut groups: Vec<Vec<usize>> = Vec::new();
            let mut cur_group: Vec<usize> = Vec::new();
            let mut prev_right: Option<f64> = None;
            for &wi in &idxs {
                let w = &words[wi];
                if let Some(pr) = prev_right {
                    if w.left - pr > GAP_SPLIT_PT {
                        groups.push(std::mem::take(&mut cur_group));
                    }
                }
                cur_group.push(wi);
                prev_right = Some(w.right);
            }
            if !cur_group.is_empty() {
                groups.push(cur_group);
            }
            groups_by_line.insert(lr, groups);
        }

        // --- column geometry: table's own grid extended left/right by outside-group starts ---
        let group_starts: Vec<f64> = groups_by_line
            .values()
            .flat_map(|gs| gs.iter().map(|g| words[g[0]].left))
            .collect();
        let grid = ColGrid::build(&table, &group_starts, &words);
        let table_col_offset = grid.table_col_offset;
        // Clusters of outside-table group starts within LINE_TOL: a cluster with >= 2 members
        // is a shared left margin across several lines (e.g. a signature block) even when it
        // falls inside the table's own x-range, where ColGrid doesn't add a column boundary.
        let start_clusters = cluster_positions(group_starts);
        // --- row geometry: above-table lines, then the table, then below-table lines. A gap
        // far bigger than the local typical pitch gets its own empty spacer row instead of
        // stretching one content row to the full gap. ---
        let above_line_ranks: Vec<usize> = line_ranks
            .iter()
            .copied()
            .filter(|lr| matches!(placement[lr], Placement::Above))
            .collect();
        let below_line_ranks: Vec<usize> = line_ranks
            .iter()
            .copied()
            .filter(|lr| matches!(placement[lr], Placement::Below))
            .collect();
        let above_tops: Vec<f64> = above_line_ranks.iter().map(|lr| line_top_of[lr]).collect();
        let below_tops: Vec<f64> = below_line_ranks.iter().map(|lr| line_top_of[lr]).collect();

        let mut all_pitches: Vec<f64> = Vec::new();
        for w in above_tops.windows(2) {
            all_pitches.push(w[1] - w[0]);
        }
        for w in below_tops.windows(2) {
            all_pitches.push(w[1] - w[0]);
        }
        let typical_pitch = if !all_pitches.is_empty() {
            median(all_pitches)
        } else {
            let avg_font = if !words.is_empty() {
                words.iter().map(|w| w.font_size).sum::<f64>() / words.len() as f64
            } else {
                11.0
            };
            (avg_font * 1.3).max(8.0)
        };

        let above_trail_gap = if !above_tops.is_empty() {
            table_top.map(|tt| (tt - *above_tops.last().unwrap()).max(0.0))
        } else {
            None
        };
        let below_lead_gap = if !below_tops.is_empty() {
            table_bottom.map(|tb| (below_tops[0] - tb).max(0.0))
        } else {
            None
        };

        let mut row_of_line: HashMap<usize, u32> = HashMap::new();
        // row_tops[r] = pt y-coordinate (top-down) of row r's top edge; used to place images.
        let mut row_tops: Vec<f64> = Vec::new();
        let mut row_heights: Vec<f64> = Vec::new();
        let mut y_cursor = content_min_y;

        let above_entries = layout_with_gaps(&above_tops, None, above_trail_gap, typical_pitch);
        let mut r = base;
        let mut above_idx = 0usize;
        for &(is_spacer, h) in &above_entries {
            worksheet.set_row_height(r, h).ok();
            row_tops.push(y_cursor);
            row_heights.push(h);
            y_cursor += h;
            if !is_spacer {
                row_of_line.insert(above_line_ranks[above_idx], r);
                above_idx += 1;
            }
            r += 1;
        }
        let row_offset = r; // table (if any) starts right after the above-table rows

        // --- table row heights, from consecutive horizontal grid lines ---
        if let Some(t) = &table {
            for rr in 0..t.nrows() {
                let h = (t.ys[rr + 1] - t.ys[rr]).max(1.0);
                worksheet.set_row_height(rr as u32 + row_offset, h).ok();
                row_tops.push(t.ys[rr]);
                row_heights.push(h);
            }
            y_cursor = *t.ys.last().unwrap();
        }

        // --- table cells (merged where ruling lines show no boundary). Borders come from
        // ruling-line thickness at the merged region's outer edges (Excel hides the internal
        // edges of a merged range regardless of the underlying cells' own border formats).
        // Alignment comes from text position vs. cell; font from the cell's dominant PDF font. ---
        if let Some(t) = &mut table {
            let mut written_regions: std::collections::HashSet<(usize, usize, usize, usize)> =
                std::collections::HashSet::new();
            for rr in 0..t.nrows() {
                for cc in 0..t.ncols() {
                    let region = t.region_of(rr, cc);
                    if !written_regions.insert(region) {
                        continue;
                    }
                    let (r0, c0, r1, c1) = region;
                    let cell_left = t.xs[c0];
                    let cell_right = t.xs[c1 + 1];
                    let cell_top = t.ys[r0];
                    let cell_bottom = t.ys[r1 + 1];
                    let borders = [
                        border_code(horiz_thickness(&table_horiz, cell_top, cell_left, cell_right)),
                        border_code(horiz_thickness(&table_horiz, cell_bottom, cell_left, cell_right)),
                        border_code(vert_thickness(&table_vert, cell_left, cell_top, cell_bottom)),
                        border_code(vert_thickness(&table_vert, cell_right, cell_top, cell_bottom)),
                    ];

                    // Gather words from every cell in this merged region, ordered top-to-bottom
                    // (by original line) then left-to-right.
                    let mut idxs: Vec<usize> = Vec::new();
                    for ri in r0..=r1 {
                        for ci in c0..=c1 {
                            if let Some(v) = table_cell_words.get(&t.cell(ri, ci)) {
                                idxs.extend(v.iter().copied());
                            }
                        }
                    }
                    if idxs.is_empty() {
                        // Still ruled: an empty grid cell should show its border like the rest.
                        let style = CellStyle { borders, ..CellStyle::plain(11.0, false) };
                        write_cell_span(
                            worksheet,
                            r0 as u32 + row_offset,
                            (c0 + table_col_offset) as u16,
                            r1 as u32 + row_offset,
                            (c1 + table_col_offset) as u16,
                            "",
                            &style,
                            fc,
                            numbers_mode,
                        );
                        continue;
                    }
                    idxs.sort_by(|&a, &b| {
                        (words[a].line_rank, (words[a].left * 100.0) as i64)
                            .cmp(&(words[b].line_rank, (words[b].left * 100.0) as i64))
                    });
                    let mut text = String::new();
                    let mut last_line = None;
                    for &wi in &idxs {
                        let w = &words[wi];
                        if Some(w.line_rank) != last_line {
                            if last_line.is_some() {
                                text.push('\n');
                            }
                            last_line = Some(w.line_rank);
                        } else {
                            text.push(' ');
                        }
                        text.push_str(&w.text);
                    }

                    let (size, bold, font_name, italic) = dominant_style(&words, &idxs);
                    let text_left = idxs.iter().map(|&i| words[i].left).fold(f64::MAX, f64::min);
                    let text_right = idxs.iter().map(|&i| words[i].right).fold(f64::MIN, f64::max);
                    let text_top = idxs.iter().map(|&i| words[i].top).fold(f64::MAX, f64::min);
                    let text_bottom = idxs.iter().map(|&i| words[i].bottom).fold(f64::MIN, f64::max);
                    let align_h = horiz_align_code(text_left, text_right, cell_left, cell_right);
                    let align_v = vert_align_code(text_top, text_bottom, cell_top, cell_bottom);
                    let underline = dominant_underline(&words, &idxs, &underline_lines);

                    let style = CellStyle { size, bold, italic, font_name, align_h, align_v, borders, underline };
                    write_cell_span(
                        worksheet,
                        r0 as u32 + row_offset,
                        (c0 + table_col_offset) as u16,
                        r1 as u32 + row_offset,
                        (c1 + table_col_offset) as u16,
                        &text,
                        &style,
                        fc,
                        numbers_mode,
                    );
                }
            }
        }

        // --- below-table row geometry ---
        let next_row_below = row_offset + table_rows as u32;
        let below_entries = layout_with_gaps(&below_tops, below_lead_gap, None, typical_pitch);
        let mut r2 = next_row_below;
        let mut below_idx = 0usize;
        for &(is_spacer, h) in &below_entries {
            worksheet.set_row_height(r2, h).ok();
            row_tops.push(y_cursor);
            row_heights.push(h);
            y_cursor += h;
            if !is_spacer {
                row_of_line.insert(below_line_ranks[below_idx], r2);
                below_idx += 1;
            }
            r2 += 1;
        }

        // --- write non-table text as whole lines, one cell per >40pt-gap group. Only horizontal
        // alignment is derived here (from the group's column x-range); vertical stays Excel's
        // default bottom, since these are single-line rows outside the ruled grid. ---
        for &lr in &line_ranks {
            let groups = &groups_by_line[&lr];
            let line_top = line_top_of[&lr];
            let row: u32 = match placement[&lr] {
                Placement::Overlap => table
                    .as_ref()
                    .map(|t| t.row_of_y(line_top) as u32 + row_offset)
                    .unwrap_or(0),
                _ => row_of_line[&lr],
            };
            for group in groups {
                let text = group
                    .iter()
                    .map(|&wi| words[wi].text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let start_x = words[group[0]].left;
                let col = grid.col_of_x(start_x);
                let (size, bold, font_name, italic) = dominant_style(&words, group);
                let cell_left = grid.xs[col];
                let cell_right = grid.xs.get(col + 1).copied().unwrap_or(cell_left + 20.0);
                let text_left = group.iter().map(|&i| words[i].left).fold(f64::MAX, f64::min);
                let text_right = group.iter().map(|&i| words[i].right).fold(f64::MIN, f64::max);
                // Only trust the alignment heuristic when the text actually fits inside the
                // snapped column; a free-text line (e.g. a signature block) wider than its
                // grid column relies on left-anchored overflow into empty neighbor cells, and
                // Excel can't spill center/right-aligned overflow past a non-empty cell on the
                // other side (this clipped signature text before this guard was added).
                // Multiple outside-table lines starting at (near) the same x read as a
                // left-aligned block in the source (e.g. a signature block), even when that x
                // falls inside the table's own column range (no ColGrid boundary there) -- text
                // hugging a shared left margin is always left, regardless of trailing column width.
                let shares_a_column_start = start_clusters
                    .iter()
                    .any(|&(rep, count)| count >= 2 && (start_x - rep).abs() <= LINE_TOL);
                let align_h = if shares_a_column_start {
                    0
                } else if text_right - text_left <= cell_right - cell_left {
                    horiz_align_code(text_left, text_right, cell_left, cell_right)
                } else {
                    0
                };
                let underline = dominant_underline(&words, group, &underline_lines);
                let style = CellStyle { size, bold, italic, font_name, align_h, underline, ..CellStyle::plain(size, bold) };
                write_cell(worksheet, row, col as u16, &text, &style, fc, numbers_mode);
            }
        }

        // --- images: place raster image page objects at their PDF position/size. Skip page-
        // covering scans (would turn the sheet into one big picture) and tiny/decorative ones. ---
        let page_area = (page_w * page_h).max(1.0);
        // A scan is often stored as several image strips (big_scan_footer: 9 per page), which the
        // per-image >50% rule below misses: decoding + embedding them costs ~1.2 s/page. Sum the
        // image boxes first (cheap, no decoding); if they cover ≥80% of the page (same threshold as
        // `pdf::is_scanned`), it's a scan page → no images at all.
        let image_cover: f64 = page
            .objects()
            .iter()
            .filter(|o| o.object_type() == PdfPageObjectType::Image)
            .filter_map(|o| o.bounds().ok())
            .map(|b| {
                let r = b.to_rect();
                let w = (r.right().value.min(page_w as f32) - r.left().value.max(0.0)).max(0.0) as f64;
                let h = (r.top().value.min(page_h as f32) - r.bottom().value.max(0.0)).max(0.0) as f64;
                w * h
            })
            .sum();
        let scan_page = image_cover >= 0.8 * page_area;
        for object in page.objects().iter() {
            if scan_page || object.object_type() != PdfPageObjectType::Image {
                continue;
            }
            let Some(img_obj) = object.as_image_object() else { continue };
            let Ok(b) = object.bounds() else { continue };
            let r = b.to_rect();
            let left = r.left().value as f64;
            let right = r.right().value as f64;
            let top = page_h - r.top().value as f64;
            let bottom = page_h - r.bottom().value as f64;
            let w_pt = right - left;
            let h_pt = bottom - top;
            if w_pt < 4.0 || h_pt < 4.0 {
                continue;
            }
            if w_pt * h_pt > 0.5 * page_area {
                continue;
            }
            let Ok(dyn_img) = img_obj.get_processed_image(&doc) else { continue };
            let mut png_bytes: Vec<u8> = Vec::new();
            if dyn_img
                .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
                .is_err()
            {
                continue;
            }
            let Ok(xl_image) = Image::new_from_buffer(&png_bytes) else { continue };
            let w_px = pt_to_px(w_pt).max(1);
            let h_px = pt_to_px(h_pt).max(1);
            let xl_image = xl_image.set_scale_to_size(w_px, h_px, false);

            let col = grid.col_of_x(left);
            let col_off_px = pt_to_px((left - grid.xs[col]).max(0.0));
            let (row_idx, row_off_pt) = row_for_y(top, &row_tops, &row_heights);
            let row_off_px = pt_to_px(row_off_pt.max(0.0));
            worksheet
                .insert_image_with_offset(row_idx + base, col as u16, &xl_image, col_off_px, row_off_px)
                .ok();
        }

        let landscape = page_w > page_h;
        let left_in = (content_min_x / 72.0).max(0.1);
        let top_in = (content_min_y / 72.0).max(0.1);
        let right_in = ((page_w - content_max_x) / 72.0).max(0.1);
        let bottom_in = ((page_h - content_max_y) / 72.0).max(0.2);
        let header_in = (top_in * 0.5).min(top_in);
        let footer_in = (bottom_in * 0.5).min(bottom_in);
        Some(PageOut {
            rows: r2 - base,
            widths: (0..grid.ncols()).map(|c| pt_to_excel_col_width(grid.xs[c + 1] - grid.xs[c])).collect(),
            setup: PageSetup {
                landscape,
                paper: paper_size_code(page_w, page_h),
                margins: [left_in, right_in, top_in, bottom_in, header_in, footer_in],
            },
        })
}

/// `numbers_mode`: false (default) writes every cell as the exact source text, string-typed, so
/// it displays identically regardless of Windows locale ("follow the source as is"). true is the
/// opt-in `--numbers` path: parse Indonesian-formatted numbers into real numeric cells.
fn write_cell(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    col: u16,
    text: &str,
    style: &CellStyle,
    fc: &mut FormatCache,
    numbers_mode: bool,
) {
    match numbers_mode.then(|| parse_number_cell(text)).flatten() {
        Some((n, _is_percent, fmt)) => {
            let f = fc.get(false, &fmt, style);
            worksheet.write_number_with_format(row, col, n, &f).ok();
        }
        // Stacked values ("100,00\n10\n10,00") need wrap or Excel shows them run together.
        None if text.contains('\n') => {
            let f = fc.get(true, "", style);
            worksheet.write_string_with_format(row, col, text, &f).ok();
        }
        None => {
            let f = fc.get(false, "", style);
            worksheet.write_string_with_format(row, col, text, &f).ok();
        }
    };
}

/// Like write_cell, but for a (possibly merged) cell range. rust_xlsxwriter's merge_range only
/// accepts a string, so a numeric merged value is written as a string first, then the top-left
/// cell is overwritten with a proper number (matches the pattern in rust_xlsxwriter's own docs).
fn write_cell_span(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    r0: u32,
    c0: u16,
    r1: u32,
    c1: u16,
    text: &str,
    style: &CellStyle,
    fc: &mut FormatCache,
    numbers_mode: bool,
) {
    if r0 == r1 && c0 == c1 {
        write_cell(worksheet, r0, c0, text, style, fc, numbers_mode);
        return;
    }
    let wrap = text.contains('\n');
    let text_fmt = fc.get(wrap, "", style);
    worksheet.merge_range(r0, c0, r1, c1, text, &text_fmt).ok();
    if numbers_mode {
        if let Some((n, _is_percent, num_fmt)) = parse_number_cell(text) {
            let nf = fc.get(false, &num_fmt, style);
            worksheet.write_number_with_format(r0, c0, n, &nf).ok();
        }
    }
}

/// Builds an Excel number format from how many decimals the source text had. Always uses a
/// thousands-grouping placeholder ("#,##0..."): it's a no-op display-wise for values under 1000,
/// so this is safe for small integers too, while fixing the original "#,##0.###" showing "6." /
/// "1." for whole numbers (0 decimals now means no decimal point at all).
fn build_num_format(decimals: usize, is_percent: bool) -> String {
    let mut s = "#,##0".to_string();
    if decimals > 0 {
        s.push('.');
        s.push_str(&"0".repeat(decimals));
    }
    if is_percent {
        s.push('%');
    }
    s
}

/// Parses a cell's text into (value, is_percent, num_format). Strips a trailing '%' before
/// delegating to parse_indonesian_number_full; a percent value is stored as a fraction
/// (100,00% -> 1.0) so Excel's percent number format displays it correctly.
fn parse_number_cell(raw: &str) -> Option<(f64, bool, String)> {
    let s = raw.trim();
    if let Some(stripped) = s.strip_suffix('%') {
        let (v, decimals) = parse_indonesian_number_full(stripped.trim())?;
        Some((v / 100.0, true, build_num_format(decimals, true)))
    } else {
        let (v, decimals) = parse_indonesian_number_full(s)?;
        Some((v, false, build_num_format(decimals, false)))
    }
}

/// Parses Indonesian-formatted numbers: "." as thousands separator, "," as decimal separator,
/// "(...)" for negatives. Leaves codes/dates/IDs with leading zeros or non-numeric dotted
/// patterns as text by returning None.
#[cfg(test)]
fn parse_indonesian_number(raw: &str) -> Option<f64> {
    parse_indonesian_number_full(raw).map(|(v, _)| v)
}

/// Like parse_indonesian_number, but also returns the number of decimal digits the source had
/// (0 if none), so the caller can build a display format that mirrors the source shape.
fn parse_indonesian_number_full(raw: &str) -> Option<(f64, usize)> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    let (negative, inner) = if s.starts_with('(') && s.ends_with(')') {
        (true, &s[1..s.len() - 1])
    } else {
        (false, s)
    };

    // Must be composed only of digits, '.', ',', and an optional leading '-'.
    let body = inner.strip_prefix('-').unwrap_or(inner);
    let negative = negative || inner.starts_with('-');
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',') {
        return None;
    }

    // Leading zero (e.g. "015.01", "0812...") is almost always a code/ID, not a number.
    if body.starts_with('0') && body.len() > 1 && !body.contains(',') {
        return None;
    }

    // More than 15 significant digits (NIP/NIK/account numbers) can't round-trip through an
    // f64/Excel double without losing precision; keep those as text.
    if body.chars().filter(|c| c.is_ascii_digit()).count() > 15 {
        return None;
    }

    let comma_count = body.matches(',').count();
    let dot_count = body.matches('.').count();

    if comma_count > 1 {
        return None; // not a recognized number shape
    }

    // Decimal comma part, if any, must be 1-2 digits (Indonesian convention).
    if comma_count == 1 {
        let decimals = body.rsplit(',').next().unwrap();
        if decimals.is_empty() || decimals.len() > 2 || !decimals.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }

    // Dotted groups (thousands separators) must each be exactly 3 digits, e.g. "1.204.500".
    // A single dot followed by something that isn't a 3-digit group (e.g. "015.01") is a code.
    let integer_part = if comma_count == 1 {
        body.rsplit_once(',').unwrap().0
    } else {
        body
    };
    if dot_count > 0 {
        let groups: Vec<&str> = integer_part.split('.').collect();
        if groups.len() < 2 {
            return None;
        }
        // First group: 1-3 digits; subsequent groups: exactly 3 digits.
        if groups[0].is_empty() || groups[0].len() > 3 {
            return None;
        }
        for g in &groups[1..] {
            if g.len() != 3 || !g.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
        }
    }

    let normalized = integer_part.replace('.', "");
    let decimal_part = if comma_count == 1 {
        body.rsplit_once(',').unwrap().1
    } else {
        ""
    };

    let full = if decimal_part.is_empty() {
        normalized
    } else {
        format!("{}.{}", normalized, decimal_part)
    };

    let value: f64 = full.parse().ok()?;
    let decimals = decimal_part.len();
    Some((if negative { -value } else { value }, decimals))
}

#[cfg(test)]
mod tests {
    use super::{parse_indonesian_number, parse_number_cell};

    #[test]
    fn number_parsing() {
        assert_eq!(parse_indonesian_number("1.204.500"), Some(1204500.0));
        assert_eq!(parse_indonesian_number("73,5"), Some(73.5));
        assert_eq!(parse_indonesian_number("(12.000)"), Some(-12000.0));
        assert_eq!(parse_indonesian_number("12.000,50"), Some(12000.5));
        assert_eq!(parse_indonesian_number("015.01"), None);
        assert_eq!(parse_indonesian_number("2024-01-01"), None);
        assert_eq!(parse_indonesian_number("0812345678"), None);
        assert_eq!(parse_indonesian_number("ABC"), None);
        assert_eq!(parse_indonesian_number("100"), Some(100.0));

        // NIP-style 18-digit ID: too many significant digits to round-trip through f64/Excel.
        assert_eq!(parse_indonesian_number("123456789012345678"), None);
        // Leading-zero code, e.g. Kode Satker.
        assert_eq!(parse_indonesian_number("054"), None);
        // Multi-line cell content must never be numberized.
        assert_eq!(parse_indonesian_number("Tisu\n6 kotak"), None);
    }

    #[test]
    fn percent_parsing() {
        let r = parse_number_cell("100,00%").unwrap();
        assert_eq!((r.0, r.1), (1.0, true));
        let r2 = parse_number_cell("0,00").unwrap();
        assert_eq!((r2.0, r2.1), (0.0, false));
        let r3 = parse_number_cell("73,5%").unwrap();
        assert_eq!((r3.0, r3.1), (0.735, true));
        assert!(parse_number_cell("123456789012345678").is_none());
    }

    #[test]
    fn num_format_reflects_source_decimals() {
        // Plain integer: no decimal point at all (was showing "6." with the old "#,##0.###").
        assert_eq!(parse_number_cell("6").unwrap().2, "#,##0");
        // "89,65" -> exactly 2 decimals, matching the source.
        assert_eq!(parse_number_cell("89,65").unwrap().2, "#,##0.00");
        // "100,00%" -> 2 decimals + percent.
        assert_eq!(parse_number_cell("100,00%").unwrap().2, "#,##0.00%");
        assert_eq!(parse_number_cell("1.204.500").unwrap().2, "#,##0");
    }
}
