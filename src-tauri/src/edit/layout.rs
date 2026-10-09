//! Edit PDF layout: lines, paragraphs, alignment, word diff, greedy breaker, line emission.
//! See _docs/edit-contract.md ("Page analysis" 5-9, "Re-typeset the new paragraph").
use super::content::{esc, fill_css, fmt, mul, tr, Show, M};
use super::fonts::{pc_font, win_ansi, Fnt, Kind, Pc};
use lopdf::ObjectId;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Center,
    Right,
    Justified,
}

#[derive(Clone, Debug)]
pub struct Seg {
    pub show: Option<usize>,
    pub x0: f64,
    pub x1: f64,
    pub base: f64,
    pub size: f64,
    pub text: String,
    pub class: u8, // 0 visible, 1 invisible, 2 form, 3 other lock (own line each)
    pub reason: Option<&'static str>,
    pub fam: String,
}

impl Seg {
    fn top(&self) -> f64 {
        self.base + 0.78 * self.size
    }
    fn bot(&self) -> f64 {
        self.base - 0.22 * self.size
    }
}

struct Line {
    segs: Vec<usize>, // x order
    l: f64,
    r: f64,
    base: f64,
    size: f64,
    top: f64,
    bot: f64,
    class: u8,
    fam: String,
    marker: bool, // segs[0] is a list marker
    text_l: f64,
}

/// Style of a run: where its glyphs come from and how to write them.
#[derive(Clone, Debug)]
pub struct Style {
    pub font: String, // resource name
    pub size: f64,
    pub fill: String,
    pub nb: u8,
    pub sp_code: Option<u32>,
    pub sp_w: f64, // 1/1000 em
    pub via_tw: bool,
}

#[derive(Clone, Debug)]
pub struct Ch {
    pub t: String,
    pub code: Option<u32>,
    pub w: f64,    // 1/1000 em
    pub kern: f64, // adjustment after the char (TJ number + Tc), 1/1000 em, positive = tighter
    pub gap: f64,  // last char of a word only: adjustment around the following space
    pub style: usize,
}

#[derive(Clone)]
pub struct EditData {
    pub line_words: Vec<usize>, // words per original line
    pub line_r: Vec<f64>,       // right edge of each original line
    pub line_base: Vec<f64>,    // baseline of each original line (Word rounds them, the pitch is a median)
    pub shows: Vec<usize>,
    pub words: Vec<Vec<Ch>>,
    pub styles: Vec<Style>,
    pub nlines: usize,
    pub pitch: f64,
    pub first_x: f64,
    pub left_x: f64,
    pub right_x: f64,
    pub centre: f64,
    pub base0: f64,
    pub top: f64,
    pub bottom: f64,
    pub align: Align,
    pub size: f64,
}

#[derive(Clone)]
pub struct Para {
    pub id: u32,
    pub locked: Option<&'static str>, // Some(reason) = no action
    pub pc_font: bool,
    pub l: f64,
    pub r: f64,
    pub top: f64,
    pub bot: f64,
    pub text: String,
    pub marker: Option<String>,
    pub align: Align,
    pub size: f64,
    pub pitch: f64,
    pub first_indent: f64,
    pub css_font: String,
    pub bold: bool,
    pub italic: bool,
    pub color: String,
    pub nlines: usize,
    pub ed: Option<EditData>,
}

pub fn family(base: &str) -> String {
    let mut s: String = base.to_lowercase().chars().filter(|c| c.is_ascii_alphabetic()).collect();
    for w in ["bold", "italic", "oblique", "regular", "semibold", "black", "mt", "ps"] {
        s = s.replace(w, "");
    }
    s
}

pub fn show_class(s: &Show, f: Option<&Fnt>) -> (u8, Option<&'static str>) {
    if s.tr == 3 {
        (1, Some("invisible"))
    } else if s.rotated() {
        (3, Some("rotated"))
    } else if f.is_some_and(|f| f.kind == Kind::Type3) {
        (3, Some("type3"))
    } else if f.is_none() || !f.is_some_and(|f| f.measurable) || s.rise != 0.0 || (s.th - 1.0).abs() > 1e-9 {
        (3, Some("structure"))
    } else {
        (0, None)
    }
}

fn is_marker(t: &str) -> bool {
    let t = t.trim();
    let n = t.chars().count();
    if n == 0 {
        return false;
    }
    if n == 1 {
        return !t.chars().next().unwrap().is_alphanumeric();
    }
    let (body, last) = t.split_at(t.len() - 1);
    n <= 5 && (last == "." || last == ")") && body.chars().all(|c| c.is_ascii_alphanumeric())
}

fn make_lines(segs: &[Seg]) -> Vec<Line> {
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| segs[a].class.cmp(&segs[b].class).then(segs[b].base.total_cmp(&segs[a].base)).then(segs[a].x0.total_cmp(&segs[b].x0)));
    let mut groups: Vec<Vec<usize>> = vec![];
    for i in order {
        let s = &segs[i];
        let own = s.class == 3;
        match groups.last_mut() {
            Some(g) if !own && segs[g[0]].class == s.class && (segs[g[0]].base - s.base).abs() <= 0.3 * s.size.max(1.0) => g.push(i),
            _ => groups.push(vec![i]),
        }
    }
    let mut lines = vec![];
    for mut g in groups {
        g.sort_by(|&a, &b| segs[a].x0.total_cmp(&segs[b].x0));
        let mut cur: Vec<usize> = vec![];
        let mut parts: Vec<Vec<usize>> = vec![];
        for i in g {
            if let Some(&p) = cur.last() {
                if segs[i].x0 - segs[p].x1 > 2.5 * segs[i].size {
                    parts.push(std::mem::take(&mut cur));
                }
            }
            cur.push(i);
        }
        parts.push(cur);
        for p in parts {
            let main = *p.iter().max_by_key(|&&i| segs[i].text.chars().count()).unwrap();
            let marker = p.len() > 1 && segs[p[0]].class == 0 && is_marker(&segs[p[0]].text) && segs[p[1]].x0 - segs[p[0]].x1 >= 0.2 * segs[p[1]].size;
            let l = segs[p[0]].x0.min(segs[p[p.len() - 1]].x0);
            lines.push(Line {
                l,
                r: p.iter().map(|&i| segs[i].x1).fold(f64::MIN, f64::max),
                base: segs[p[0]].base,
                size: segs[main].size,
                top: p.iter().map(|&i| segs[i].top()).fold(f64::MIN, f64::max),
                bot: p.iter().map(|&i| segs[i].bot()).fold(f64::MAX, f64::min),
                class: segs[p[0]].class,
                fam: segs[main].fam.clone(),
                text_l: if marker { segs[p[1]].x0 } else { l },
                marker,
                segs: p,
            });
        }
    }
    lines.sort_by(|a, b| b.base.total_cmp(&a.base).then(a.l.total_cmp(&b.l)));
    lines
}

fn joinable(lines: &[Line], p: &[usize], b: &Line) -> bool {
    let a = &lines[*p.last().unwrap()];
    if a.class != b.class || a.class == 3 || b.marker {
        return false;
    }
    let pitch = a.base - b.base;
    if pitch <= 0.5 * b.size || a.fam != b.fam || (a.size - b.size).abs() >= 0.2 {
        return false;
    }
    if p.len() == 1 {
        let hang = a.marker && (b.l - a.text_l).abs() <= 2.0;
        pitch <= 1.8 * b.size && (hang || (b.l <= a.l + 2.0 && a.l - b.l <= 40.0))
    } else {
        let pp = lines[p[p.len() - 2]].base - a.base;
        (pitch - pp).abs() <= 0.2 * pp.abs() && (lines[p[1]].l - b.l).abs() <= 2.0
    }
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

pub struct PageIn<'a> {
    pub shows: &'a [Show],
    pub fonts: &'a BTreeMap<String, Fnt>,
    pub forms: Vec<Seg>,
    pub page: [f64; 4], // ox, oy, w, h
}

/// Paragraphs of a page (sorted top to bottom, then left to right) and the page's typical line pitch.
pub fn find_paras(pin: &PageIn) -> (Vec<Para>, f64) {
    let mut segs: Vec<Seg> = vec![];
    for (i, s) in pin.shows.iter().enumerate() {
        if s.blank() {
            continue;
        }
        let f = pin.fonts.get(&s.font);
        let (class, reason) = show_class(s, f);
        // extents of the visible glyphs: edge spaces would blur the right edge of justified lines
        let vis: Vec<&super::content::Glyph> = s.glyphs.iter().filter(|g| g.text.as_deref().is_none_or(|t| !t.trim().is_empty())).collect();
        let (gx0, gx1) = (vis.first().map(|g| g.x0).unwrap_or(s.x0), vis.last().map(|g| g.x1).unwrap_or(s.x1));
        segs.push(Seg { show: Some(i), x0: gx0.min(gx1), x1: gx0.max(gx1), base: s.base, size: s.size, text: s.text(), class, reason, fam: family(f.map(|f| f.base.as_str()).unwrap_or("")) });
    }
    segs.extend(pin.forms.iter().cloned());
    let lines = make_lines(&segs);
    let mut paras: Vec<Vec<usize>> = vec![];
    for (i, b) in lines.iter().enumerate() {
        match paras.iter().rposition(|p| joinable(&lines, p, b)) {
            Some(k) => paras[k].push(i),
            None => paras.push(vec![i]),
        }
    }
    let body: Vec<&Line> = lines.iter().filter(|l| l.class == 0).collect();
    let body_l = body.iter().map(|l| l.l).fold(f64::MAX, f64::min);
    let body_r = body.iter().map(|l| l.r).fold(f64::MIN, f64::max);
    let page_centre = pin.page[0] + pin.page[2] / 2.0;
    let page_pitch = median(paras.iter().filter(|p| p.len() > 1).flat_map(|p| p.windows(2).map(|w| lines[w[0]].base - lines[w[1]].base)).collect())
        .unwrap_or_else(|| 1.2 * median(body.iter().map(|l| l.size).collect()).unwrap_or(11.0));
    let mut out: Vec<Para> = paras.iter().map(|p| build_para(pin, &segs, &lines, p, (body_l, body_r, page_centre, page_pitch))).collect();
    out.sort_by(|a, b| b.top.total_cmp(&a.top).then(a.l.total_cmp(&b.l)));
    for (i, p) in out.iter_mut().enumerate() {
        p.id = i as u32;
    }
    (out, page_pitch)
}

/// The last char's own adjustment belongs to the gap behind the word (`space_kern` is the space glyph's).
fn end_word(mut w: Vec<Ch>, space_kern: f64) -> Vec<Ch> {
    if let Some(l) = w.last_mut() {
        l.gap = l.kern + space_kern;
        l.kern = 0.0;
    }
    w
}

fn build_para(pin: &PageIn, segs: &[Seg], lines: &[Line], p: &[usize], (body_l, body_r, page_centre, page_pitch): (f64, f64, f64, f64)) -> Para {
    let pl: Vec<&Line> = p.iter().map(|&i| &lines[i]).collect();
    let n = pl.len();
    let class = pl[0].class;
    let l = pl.iter().map(|l| l.l).fold(f64::MAX, f64::min);
    let r = pl.iter().map(|l| l.r).fold(f64::MIN, f64::max);
    let (top, bot) = (pl.iter().map(|l| l.top).fold(f64::MIN, f64::max), pl.iter().map(|l| l.bot).fold(f64::MAX, f64::min));
    let size = pl[0].size;
    let line_text = |ln: &Line| ln.segs.iter().skip(ln.marker as usize).map(|&i| segs[i].text.as_str()).collect::<Vec<_>>().join(" ");
    let text = pl.iter().map(|l| line_text(l)).collect::<Vec<_>>().join(" ");
    let mut para = Para {
        id: 0,
        locked: None,
        pc_font: false,
        l,
        r,
        top,
        bot,
        text,
        marker: pl[0].marker.then(|| segs[pl[0].segs[0]].text.trim().to_string()),
        align: Align::Left,
        size,
        pitch: page_pitch,
        first_indent: 0.0,
        css_font: "Arial".into(),
        bold: false,
        italic: false,
        color: "#000000".into(),
        nlines: n,
        ed: None,
    };
    if class != 0 {
        para.locked = Some(match class {
            1 => "invisible",
            2 => "form",
            _ => segs[pl[0].segs[0]].reason.unwrap_or("structure"),
        });
        return para;
    }
    // ---- geometry
    let first_x = pl[0].text_l;
    let left_x = if n > 1 { pl[1..].iter().map(|l| l.text_l).fold(f64::MAX, f64::min) } else { first_x };
    let max_r = r;
    let pitch = median((0..n.saturating_sub(1)).map(|i| pl[i].base - pl[i + 1].base).collect()).unwrap_or(page_pitch);
    let widest = pl.iter().map(|l| l.r - l.text_l).fold(0.0, f64::max);
    let centres: Vec<f64> = pl.iter().map(|l| (l.text_l + l.r) / 2.0).collect();
    let align = if n >= 2 {
        let cont_ok = if n == 2 { pl[1].text_l >= pl[0].text_l - 2.0 && pl[1].text_l <= pl[0].text_l + 40.0 } else { pl[1..].iter().all(|l| (l.text_l - left_x).abs() <= 1.5) };
        if cont_ok && pl[..n - 1].iter().all(|l| l.r >= max_r - 1.5) {
            Align::Justified
        } else if centres.iter().all(|c| (c - centres[0]).abs() <= 1.5) {
            Align::Center
        } else if pl.iter().all(|l| (l.r - max_r).abs() <= 1.5) {
            Align::Right
        } else {
            Align::Left
        }
    } else if ((centres[0] - page_centre).abs() <= 2.0 || (centres[0] - (body_l + body_r) / 2.0).abs() <= 2.0) && widest < 0.9 * (body_r - body_l).max(1.0) {
        Align::Center
    } else if (pl[0].r - body_r).abs() <= 2.0 && widest < 0.8 * (body_r - body_l) {
        Align::Right
    } else {
        Align::Left
    };
    // ---- styles and words
    let mut styles: Vec<Style> = vec![];
    let mut style_of = |s: &Show| -> usize {
        let f = &pin.fonts[&s.font];
        let key = (s.font.clone(), (s.size * 100.0).round() / 100.0, s.fill.clone());
        if let Some(i) = styles.iter().position(|x| (x.font.clone(), (x.size * 100.0).round() / 100.0, x.fill.clone()) == key) {
            return i;
        }
        let sp = f.enc.get(&' ').copied();
        styles.push(Style { font: s.font.clone(), size: s.size, fill: s.fill.clone(), nb: f.nb, sp_code: sp, sp_w: sp.map(|c| f.w(c)).filter(|w| *w > 0.0).unwrap_or(250.0), via_tw: f.kind == Kind::Simple && sp == Some(32) });
        styles.len() - 1
    };
    let mut words: Vec<Vec<Ch>> = vec![];
    let mut line_words: Vec<usize> = vec![];
    let mut cur: Vec<Ch> = vec![];
    let mut show_ids: Vec<usize> = vec![];
    let mut nonenc = false;
    for ln in pl.iter() {
        let before = words.len();
        let mut prev_x1: Option<f64> = None;
        for &si in ln.segs.iter().skip(ln.marker as usize) {
            let sg = &segs[si];
            let s = &pin.shows[sg.show.unwrap()];
            show_ids.push(sg.show.unwrap());
            let st = style_of(s);
            nonenc |= !pin.fonts[&s.font].encodable;
            // a gap between two shows without a space glyph is a word break
            if prev_x1.is_some_and(|px| sg.x0 - px > 0.15 * sg.size) && !cur.is_empty() {
                words.push(end_word(std::mem::take(&mut cur), 0.0));
            }
            for g in &s.glyphs {
                let t = g.text.clone().unwrap_or_else(|| "�".into());
                if t.trim().is_empty() {
                    if !cur.is_empty() {
                        words.push(end_word(std::mem::take(&mut cur), g.kern));
                    }
                } else {
                    cur.push(Ch { t, code: Some(g.code), w: g.w, kern: g.kern, gap: 0.0, style: st });
                }
            }
            prev_x1 = Some(sg.x1);
        }
        if !cur.is_empty() {
            words.push(end_word(std::mem::take(&mut cur), 0.0));
        }
        line_words.push(words.len() - before);
    }
    // dominant style
    let mut count = vec![0usize; styles.len()];
    for c in words.iter().flatten() {
        count[c.style] += 1;
    }
    let dom = count.iter().enumerate().max_by_key(|x| x.1).map(|x| x.0).unwrap_or(0);
    if let Some(st) = styles.get(dom) {
        let f = &pin.fonts[&st.font];
        para.size = st.size;
        para.css_font = f.css_font();
        para.bold = f.bold;
        para.italic = f.italic;
        para.color = fill_css(&st.fill);
    }
    para.text = words.iter().map(|w| w.iter().map(|c| c.t.as_str()).collect::<String>()).collect::<Vec<_>>().join(" ");
    para.pc_font = nonenc;
    para.align = align;
    para.pitch = pitch;
    para.first_indent = if n > 1 { (first_x - left_x).max(0.0) } else { 0.0 };
    para.l = first_x.min(left_x);
    // layout box
    let (mut box_l, mut box_r) = (left_x, max_r.max(left_x + widest));
    let centre = if n == 1 { centres[0] } else { centres.iter().sum::<f64>() / n as f64 };
    match (align, n) {
        (Align::Left | Align::Justified, 1) => box_r = box_r.max(body_r),
        (Align::Center, _) => {
            let half = if n == 1 { (centre - body_l).min(body_r - centre).max(widest / 2.0) } else { widest / 2.0 };
            (box_l, box_r) = (centre - half, centre + half);
        }
        (Align::Right, 1) => box_l = box_l.min(body_l),
        (Align::Right, _) => box_l = box_r - widest,
        _ => {}
    }
    para.ed = Some(EditData {
        line_words,
        line_r: pl.iter().map(|l| l.r).collect(),
        line_base: pl.iter().map(|l| l.base).collect(),
        shows: show_ids,
        words,
        styles,
        nlines: n,
        pitch,
        first_x,
        left_x: box_l,
        right_x: box_r,
        centre,
        base0: pl[0].base,
        top,
        bottom: bot,
        align,
        size: para.size,
    });
    para
}

// ---------------- word diff ----------------

/// For each new word: Some(old index) when it is an unchanged word (LCS), else None.
pub fn diff_words(old: &[String], new: &[&str]) -> Vec<Option<usize>> {
    let (n, m) = (old.len(), new.len());
    let mut out = vec![None; m];
    // ponytail: O(n*m) table; paragraphs are a few hundred words. Falls back to prefix/suffix matching beyond 4M cells.
    if n * m > 4_000_000 {
        let pre = (0..n.min(m)).take_while(|&i| old[i] == new[i]).count();
        for (i, o) in out.iter_mut().enumerate().take(pre) {
            *o = Some(i);
        }
        let suf = (0..(n - pre).min(m - pre)).take_while(|&i| old[n - 1 - i] == new[m - 1 - i]).count();
        for i in 0..suf {
            out[m - 1 - i] = Some(n - 1 - i);
        }
        return out;
    }
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[at(i, j)] = if old[i] == new[j] { t[at(i + 1, j + 1)] + 1 } else { t[at(i + 1, j)].max(t[at(i, j + 1)]) };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            out[j] = Some(i);
            i += 1;
            j += 1;
        } else if t[at(i + 1, j)] >= t[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

// ---------------- retyping ----------------

pub enum Fail {
    Missing(String),
    Unsupported(String),
    NoPcFont,
}

/// PC fonts used while building: one per (family, bold, italic), numbered MjF1...
#[derive(Default)]
pub struct PcSet {
    pub faces: Vec<((String, bool, bool), Pc)>,
}

impl PcSet {
    fn style(&mut self, f: &Fnt, size: f64, fill: &str) -> Result<Style, Fail> {
        let key = (family(&f.base), f.bold, f.italic);
        let idx = match self.faces.iter().position(|x| x.0 == key) {
            Some(i) => i,
            None => {
                let pc = pc_font(&f.base, f.bold, f.italic).map_err(|_| Fail::NoPcFont)?;
                self.faces.push((key, pc));
                self.faces.len() - 1
            }
        };
        Ok(Style { font: format!("MjF{}", idx + 1), size, fill: fill.into(), nb: 1, sp_code: Some(32), sp_w: self.faces[idx].1.widths[32].max(250.0), via_tw: true })
    }
}

fn uniq(chars: impl Iterator<Item = char>) -> String {
    let mut seen = HashSet::new();
    chars.filter(|c| seen.insert(*c)).collect()
}

/// New paragraph content: words with per-char styles and codes. Unchanged words keep their original codes.
pub fn retype(
    ed: &EditData,
    fonts: &BTreeMap<String, Fnt>,
    used: &HashMap<ObjectId, HashSet<u32>>,
    text: &str,
    pc: Option<&mut PcSet>,
) -> Result<(Vec<Style>, Vec<Vec<Ch>>), Fail> {
    let new: Vec<&str> = text.split_whitespace().collect();
    let old: Vec<String> = ed.words.iter().map(|w| w.iter().map(|c| c.t.as_str()).collect()).collect();
    let map = diff_words(&old, &new);
    // style of each new word's chars: kept words own, inserted words inherit the previous word's last style (first word: the next kept one)
    let mut style_for: Vec<usize> = vec![];
    let mut prev: Option<usize> = None;
    for (k, m) in map.iter().enumerate() {
        let st = match m {
            Some(o) => ed.words[*o].first().map(|c| c.style).unwrap_or(0),
            None => prev.unwrap_or_else(|| map[k..].iter().flatten().next().and_then(|o| ed.words[*o].first()).map(|c| c.style).unwrap_or_else(|| ed.words.first().and_then(|w| w.first()).map(|c| c.style).unwrap_or(0))),
        };
        style_for.push(st);
        prev = Some(match m {
            Some(o) => ed.words[*o].last().map(|c| c.style).unwrap_or(st),
            None => st,
        });
    }
    match pc {
        None => {
            let mut missing = String::new();
            let mut words = vec![];
            for (k, w) in new.iter().enumerate() {
                if let Some(o) = map[k] {
                    words.push(ed.words[o].clone());
                    continue;
                }
                let st = style_for[k];
                let f = &fonts[&ed.styles[st].font];
                let none = HashSet::new();
                let u = f.id.and_then(|id| used.get(&id)).unwrap_or(&none);
                let mut chars = vec![];
                for c in w.chars() {
                    match f.code_for(c, u) {
                        Some(code) => chars.push(Ch { t: c.to_string(), code: Some(code), w: f.w(code), kern: 0.0, gap: 0.0, style: st }),
                        None => missing.push(c),
                    }
                }
                words.push(chars);
            }
            if missing.is_empty() {
                Ok((ed.styles.clone(), words))
            } else {
                Err(Fail::Missing(uniq(missing.chars())))
            }
        }
        Some(pcs) => {
            let mut styles = vec![];
            for s in &ed.styles {
                styles.push(pcs.style(&fonts[&s.font], s.size, &s.fill)?);
            }
            let mut bad = String::new();
            let mut words = vec![];
            for (k, w) in new.iter().enumerate() {
                let st = style_for[k];
                let mut chars = vec![];
                // an unchanged word keeps its own per-char styles
                let own: Option<&Vec<Ch>> = map[k].map(|o| &ed.words[o]);
                for (ci, c) in w.chars().enumerate() {
                    let cst = own.and_then(|o| o.get(ci)).map(|c| c.style).unwrap_or(st);
                    match win_ansi(c) {
                        Some(code) if face_w(pcs, &styles[cst].font, code) > 0.0 => chars.push(Ch { t: c.to_string(), code: Some(code as u32), w: face_w(pcs, &styles[cst].font, code), kern: 0.0, gap: 0.0, style: cst }),
                        _ => bad.push(c),
                    }
                }
                words.push(chars);
            }
            if bad.is_empty() {
                Ok((styles, words))
            } else {
                Err(Fail::Unsupported(uniq(bad.chars())))
            }
        }
    }
}

fn face_w(pcs: &PcSet, font: &str, code: u8) -> f64 {
    let i: usize = font.trim_start_matches("MjF").parse::<usize>().unwrap_or(1) - 1;
    pcs.faces[i].1.widths[code as usize]
}

// ---------------- breaking ----------------

pub struct LayLine {
    pub x: f64,
    pub words: Vec<usize>,
    pub gap: f64, // extra width per space (justify), pt
}

fn wpt(styles: &[Style], w: &[Ch]) -> f64 {
    w.iter().map(|c| (c.w - c.kern) / 1000.0 * styles[c.style].size).sum()
}

fn sp_pt(styles: &[Style], w: &[Ch]) -> f64 {
    w.last().map(|c| (styles[c.style].sp_w - c.gap) / 1000.0 * styles[c.style].size).unwrap_or(0.0)
}

pub fn typeset(ed: &EditData, styles: &[Style], words: &[Vec<Ch>]) -> Vec<LayLine> {
    let justified = ed.align == Align::Justified;
    let start = |li: usize| match ed.align {
        Align::Left | Align::Justified => if li == 0 { ed.first_x } else { ed.left_x },
        _ => ed.left_x,
    };
    // Lines wholly inside the unchanged start of the text keep Word's own breaks (Word's breaker is not plain first-fit:
    // it shrinks spaces by up to ~17 % on some lines). The last original line only counts when nothing changed at all.
    let text = |w: &[Ch]| w.iter().map(|c| c.t.as_str()).collect::<String>();
    let prefix = ed.words.iter().zip(words.iter()).take_while(|(o, n)| text(o) == text(n)).count();
    let same = prefix == words.len() && prefix == ed.words.len();
    let mut fixed: Vec<usize> = vec![]; // words per fixed line
    let mut acc = 0;
    for (li, &n) in ed.line_words.iter().enumerate() {
        if acc + n > prefix || (li + 1 == ed.nlines && !same) {
            break;
        }
        fixed.push(n);
        acc += n;
    }
    let mut lines: Vec<LayLine> = vec![];
    let mut cur: Vec<usize> = vec![];
    let mut cur_w = 0.0;
    let flush = |lines: &mut Vec<LayLine>, cur: &mut Vec<usize>, cur_w: f64, last: bool| {
        if cur.is_empty() {
            return;
        }
        let li = lines.len();
        let nsp = cur.len() - 1;
        let right = if li + 1 < ed.nlines { ed.line_r.get(li).copied().unwrap_or(ed.right_x) } else { ed.right_x };
        let (x, gap) = match ed.align {
            Align::Left => (start(li), 0.0),
            Align::Justified => (start(li), if !last && nsp > 0 { (right - start(li) - cur_w) / nsp as f64 } else { 0.0 }),
            Align::Center => (ed.centre - cur_w / 2.0, 0.0),
            Align::Right => (ed.right_x - cur_w, 0.0),
        };
        lines.push(LayLine { x, words: std::mem::take(cur), gap });
    };
    let width_of = |ix: &[usize]| -> f64 { ix.iter().map(|&i| wpt(styles, &words[i])).sum::<f64>() + ix.windows(2).map(|w| sp_pt(styles, &words[w[0]])).sum::<f64>() };
    let mut next = 0;
    for (k, &n) in fixed.iter().enumerate() {
        cur = (next..next + n).collect();
        cur_w = width_of(&cur);
        flush(&mut lines, &mut cur, cur_w, same && k + 1 == fixed.len());
        next += n;
    }
    for i in next..words.len() {
        let w = &words[i];
        let ww = wpt(styles, w);
        if cur.is_empty() {
            cur.push(i);
            cur_w = ww;
            continue;
        }
        let sp = sp_pt(styles, &words[*cur.last().unwrap()]);
        let shrink = if justified { 0.15 * sp * cur.len() as f64 } else { 0.0 };
        let li = lines.len();
        let (x0, limit) = match ed.align {
            Align::Center => (ed.centre - (ed.right_x - ed.left_x) / 2.0, ed.right_x),
            _ => (start(li), ed.right_x),
        };
        if x0 + cur_w + sp + ww - shrink > limit + 0.5 {
            flush(&mut lines, &mut cur, cur_w, false);
            cur.push(i);
            cur_w = ww;
        } else {
            cur.push(i);
            cur_w += sp + ww;
        }
    }
    flush(&mut lines, &mut cur, cur_w, true);
    lines
}

/// Horizontal extent (left, right) of a typeset paragraph.
pub fn extent(styles: &[Style], words: &[Vec<Ch>], lines: &[LayLine]) -> (f64, f64) {
    let mut e = (f64::MAX, f64::MIN);
    for ln in lines {
        let w: f64 = ln.words.iter().map(|&i| wpt(styles, &words[i])).sum::<f64>() + ln.words.windows(2).map(|p| sp_pt(styles, &words[p[0]]) + ln.gap).sum::<f64>();
        e = (e.0.min(ln.x), e.1.max(ln.x + w));
    }
    e
}

/// Content bytes (BT..ET per line) for a typeset paragraph; first baseline = base0 - shift.
/// `rinv`: display frame -> content space, so a rotated page gets its new text turned the same way.
pub fn emit(ed: &EditData, styles: &[Style], words: &[Vec<Ch>], lines: &[LayLine], shift: f64, rinv: M) -> Vec<u8> {
    let mut out = String::new();
    for (li, ln) in lines.iter().enumerate() {
        let y = ed.line_base.get(li).copied().unwrap_or_else(|| ed.line_base.last().copied().unwrap_or(ed.base0) - ed.pitch * (li + 1 - ed.line_base.len()) as f64) - shift;
        let tm = mul(tr(ln.x, y), rinv);
        let _ = writeln!(out, "BT
{} {} {} {} {} {} Tm
{} Tw", fmt(tm[0]), fmt(tm[1]), fmt(tm[2]), fmt(tm[3]), fmt(tm[4]), fmt(tm[5]), fmt(ln.gap));
        // flat run list: (style, item) with spaces taking the previous word's style
        enum It {
            S(Vec<u8>),
            N(f64),
        }
        let mut cur_style = usize::MAX;
        let mut items: Vec<It> = vec![];
        let flush = |out: &mut String, items: &mut Vec<It>| {
            if items.is_empty() {
                return;
            }
            out.push('[');
            for it in items.iter() {
                match it {
                    It::S(b) => out.push_str(&esc(b)),
                    It::N(x) => {
                        let _ = write!(out, " {} ", fmt(*x));
                    }
                }
            }
            out.push_str("] TJ\n");
            items.clear();
        };
        let put = |items: &mut Vec<It>, code: u32, nb: u8| {
            let b: Vec<u8> = if nb == 2 { vec![(code >> 8) as u8, code as u8] } else { vec![code as u8] };
            match items.last_mut() {
                Some(It::S(p)) => p.extend(b),
                _ => items.push(It::S(b)),
            }
        };
        let mut seq: Vec<(usize, Option<&Ch>, f64)> = vec![];
        for (k, &wi) in ln.words.iter().enumerate() {
            seq.extend(words[wi].iter().map(|c| (c.style, Some(c), 0.0)));
            if k + 1 < ln.words.len() {
                seq.push((words[wi].last().map(|c| c.style).unwrap_or(0), None, words[wi].last().map(|c| c.gap).unwrap_or(0.0)));
            }
        }
        for (st, ch, gap_k) in seq {
            let s = &styles[st];
            if st != cur_style {
                flush(&mut out, &mut items);
                let fill = if s.fill.is_empty() { "0 g" } else { s.fill.as_str() };
                let _ = writeln!(out, "{fill}\n/{} {} Tf", s.font, fmt(s.size));
                cur_style = st;
            }
            match ch {
                Some(c) => {
                    put(&mut items, c.code.unwrap_or(0), s.nb);
                    if c.kern.abs() > 0.01 {
                        items.push(It::N(c.kern));
                    }
                }
                None => {
                    if s.via_tw {
                        put(&mut items, 32, 1);
                        if gap_k.abs() > 0.01 {
                            items.push(It::N(gap_k));
                        }
                    } else {
                        let adj = gap_k
                            + match s.sp_code {
                                Some(c) => {
                                    put(&mut items, c, s.nb);
                                    -ln.gap * 1000.0 / s.size
                                }
                                None => -(s.sp_w + ln.gap * 1000.0 / s.size),
                            };
                        if adj.abs() > 1e-6 {
                            items.push(It::N(adj));
                        }
                    }
                }
            }
        }
        flush(&mut out, &mut items);
        out.push_str("ET\n");
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcs_keeps_unchanged_words() {
        let old: Vec<String> = "a b c d".split(' ').map(String::from).collect();
        let m = diff_words(&old, &["a", "x", "c", "d", "e"]);
        assert_eq!(m, vec![Some(0), None, Some(2), Some(3), None]);
    }

    #[test]
    fn markers() {
        for t in ["1.", "a)", "\u{2022}", "12.", "-"] {
            assert!(is_marker(t), "{t}");
        }
        for t in ["Hello", "A.B.C.D.E.F", "ab"] {
            assert!(!is_marker(t), "{t}");
        }
    }
}
