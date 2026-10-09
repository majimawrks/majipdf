//! Content-stream surgery for Edit PDF: lexer with byte offsets, graphics/text state tracker,
//! show-op rewrite (neutralise), splice, push ranges. See _docs/edit-contract.md ("Engine", "Neutralise", "Push").
use super::fonts::Fnt;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write;

pub type M = [f64; 6];
pub const ID: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// a then b (row vectors).
pub fn mul(a: M, b: M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub fn inv(m: M) -> Option<M> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

pub fn tr(x: f64, y: f64) -> M {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

fn apply(m: M, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

// ---------------- lexer ----------------
#[derive(Debug, Clone)]
pub enum El {
    Num(f64),
    Str(Vec<u8>),
}

#[derive(Debug, Clone)]
enum K {
    Num(f64),
    Name(String),
    Str(Vec<u8>),
    Arr(Vec<El>),
    Op(String),
    Other,
}

#[derive(Debug, Clone)]
struct T {
    k: K,
    s: usize,
    e: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}
fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// b[i] == '('; returns the bytes and the index after the closing ')'.
fn lex_string(b: &[u8], mut i: usize) -> (Vec<u8>, usize) {
    let mut depth = 0;
    let mut out = vec![];
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            b'(' => {
                if depth > 0 {
                    out.push(c);
                }
                depth += 1;
            }
            b')' => {
                depth -= 1;
                if depth <= 0 {
                    break;
                }
                out.push(c);
            }
            b'\\' if i < b.len() => {
                let d = b[i];
                i += 1;
                match d {
                    b'n' => out.push(10),
                    b'r' => out.push(13),
                    b't' => out.push(9),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'0'..=b'7' => {
                        let mut v = (d - b'0') as u32;
                        let mut n = 1;
                        while n < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                            v = v * 8 + (b[i] - b'0') as u32;
                            i += 1;
                            n += 1;
                        }
                        out.push(v as u8);
                    }
                    b'\r' => {
                        if i < b.len() && b[i] == b'\n' {
                            i += 1;
                        }
                    }
                    b'\n' => {}
                    o => out.push(o),
                }
            }
            o => out.push(o),
        }
    }
    (out, i)
}

/// b[i] == '<' (not '<<'); returns the decoded bytes and the index after '>'.
fn lex_hex(b: &[u8], mut i: usize) -> (Vec<u8>, usize) {
    i += 1;
    let mut hv: Vec<u8> = vec![];
    while i < b.len() && b[i] != b'>' {
        if b[i].is_ascii_hexdigit() {
            hv.push(b[i]);
        }
        i += 1;
    }
    i = (i + 1).min(b.len());
    if hv.len() % 2 == 1 {
        hv.push(b'0');
    }
    let v = hv.chunks(2).map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap_or("0"), 16).unwrap_or(0)).collect();
    (v, i)
}

fn lex(b: &[u8]) -> Vec<T> {
    let mut v = vec![];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if is_ws(c) {
            i += 1;
            continue;
        }
        let s = i;
        match c {
            b'%' => {
                while i < b.len() && b[i] != 10 && b[i] != 13 {
                    i += 1;
                }
            }
            b'(' => {
                let (st, e) = lex_string(b, i);
                i = e;
                v.push(T { k: K::Str(st), s, e: i });
            }
            b'<' if b.get(i + 1) == Some(&b'<') => {
                let mut d = 0;
                while i < b.len() {
                    if b[i] == b'<' && b.get(i + 1) == Some(&b'<') {
                        d += 1;
                        i += 2;
                    } else if b[i] == b'>' && b.get(i + 1) == Some(&b'>') {
                        d -= 1;
                        i += 2;
                        if d == 0 {
                            break;
                        }
                    } else if b[i] == b'(' {
                        i = lex_string(b, i).1;
                    } else {
                        i += 1;
                    }
                }
                v.push(T { k: K::Other, s, e: i });
            }
            b'<' => {
                let (st, e) = lex_hex(b, i);
                i = e;
                v.push(T { k: K::Str(st), s, e: i });
            }
            b'[' => {
                i += 1;
                let mut els = vec![];
                loop {
                    while i < b.len() && is_ws(b[i]) {
                        i += 1;
                    }
                    if i >= b.len() {
                        break;
                    }
                    if b[i] == b']' {
                        i += 1;
                        break;
                    }
                    if b[i] == b'(' {
                        let (st, e) = lex_string(b, i);
                        i = e;
                        els.push(El::Str(st));
                    } else if b[i] == b'<' {
                        let (st, e) = lex_hex(b, i);
                        i = e;
                        els.push(El::Str(st));
                    } else {
                        let st = i;
                        while i < b.len() && !is_ws(b[i]) && !is_delim(b[i]) {
                            i += 1;
                        }
                        if i == st {
                            i += 1;
                            continue;
                        }
                        els.push(El::Num(std::str::from_utf8(&b[st..i]).ok().and_then(|x| x.parse().ok()).unwrap_or(0.0)));
                    }
                }
                v.push(T { k: K::Arr(els), s, e: i });
            }
            b']' | b'>' | b')' | b'{' | b'}' => {
                i += 1;
                v.push(T { k: K::Other, s, e: i });
            }
            b'/' => {
                i += 1;
                while i < b.len() && !is_ws(b[i]) && !is_delim(b[i]) {
                    i += 1;
                }
                v.push(T { k: K::Name(String::from_utf8_lossy(&b[s + 1..i]).into()), s, e: i });
            }
            _ => {
                while i < b.len() && !is_ws(b[i]) && !is_delim(b[i]) {
                    i += 1;
                }
                let w = std::str::from_utf8(&b[s..i]).unwrap_or("?");
                if let Ok(n) = w.parse::<f64>() {
                    v.push(T { k: K::Num(n), s, e: i });
                } else if w == "BI" {
                    // inline image: skip to "EI" and keep it as one op
                    let mut j = i;
                    loop {
                        if j + 2 >= b.len() {
                            j = b.len();
                            break;
                        }
                        if is_ws(b[j]) && b[j + 1] == b'E' && b[j + 2] == b'I' && (j + 3 >= b.len() || is_ws(b[j + 3])) {
                            j += 3;
                            break;
                        }
                        j += 1;
                    }
                    i = j;
                    v.push(T { k: K::Op("BI".into()), s, e: i });
                } else if matches!(w, "true" | "false" | "null") {
                    v.push(T { k: K::Other, s, e: i });
                } else {
                    v.push(T { k: K::Op(w.into()), s, e: i });
                }
            }
        }
    }
    v
}

// ---------------- state tracker ----------------

#[derive(Clone, PartialEq, Debug)]
pub struct GS {
    pub ctm: M,
    pub tc: f64,
    pub tw: f64,
    pub th: f64,
    pub tl: f64,
    pub rise: f64,
    pub tr: i64,
    pub font: String,
    pub fs: f64,
    pub fill_cs: String,
    pub fill: String,
    pub stroke_cs: String,
    pub stroke: String,
    pub lw: f64,
    pub lw_op: String,
    pub cap: String,
    pub join: String,
    pub miter: String,
    pub dash: String,
    pub gs: String,
}

impl GS {
    fn new() -> GS {
        GS {
            ctm: ID,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            rise: 0.0,
            tr: 0,
            font: String::new(),
            fs: 0.0,
            fill_cs: String::new(),
            fill: String::new(),
            stroke_cs: String::new(),
            stroke: String::new(),
            lw: 1.0,
            lw_op: String::new(),
            cap: String::new(),
            join: String::new(),
            miter: String::new(),
            dash: String::new(),
            gs: String::new(),
        }
    }
}

/// Operators that re-establish `after` right after a `Q` that reset the state to `before`. None = CTM differs (cannot be re-emitted).
pub fn restore_ops(before: &GS, after: &GS) -> Option<String> {
    if before.ctm != after.ctm {
        return None;
    }
    let mut s = String::new();
    if (before.font != after.font || before.fs != after.fs) && !after.font.is_empty() {
        let _ = write!(s, "/{} {} Tf ", after.font, fmt(after.fs));
    }
    for (b, a, op) in [(before.tc, after.tc, "Tc"), (before.tw, after.tw, "Tw"), (before.th, after.th, "Tz"), (before.tl, after.tl, "TL"), (before.rise, after.rise, "Ts")] {
        if b != a {
            let _ = write!(s, "{} {} ", fmt(if op == "Tz" { a * 100.0 } else { a }), op);
        }
    }
    if before.tr != after.tr {
        let _ = write!(s, "{} Tr ", after.tr);
    }
    for (bc, ac, b, a) in [(&before.fill_cs, &after.fill_cs, &before.fill, &after.fill), (&before.stroke_cs, &after.stroke_cs, &before.stroke, &after.stroke)] {
        if bc != ac && !ac.is_empty() {
            let _ = write!(s, "{ac} ");
        }
        if (b != a || bc != ac) && !a.is_empty() {
            let _ = write!(s, "{a} ");
        }
    }
    for (b, a) in [(&before.lw_op, &after.lw_op), (&before.cap, &after.cap), (&before.join, &after.join), (&before.miter, &after.miter), (&before.dash, &after.dash), (&before.gs, &after.gs)] {
        if b != a {
            let _ = write!(s, "{a} ");
        }
    }
    Some(s)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OK {
    Show,
    Paint,
    Do,
    Clip,
    Bt,
    Et,
    Q,
    EndQ,
    State,
    Other,
}

#[derive(Clone, Debug)]
pub struct Op {
    pub s: usize, // first operand (or the operator)
    pub e: usize,
    pub k: OK,
    pub bbox: Option<[f64; 4]>, // page space, x0 y0 x1 y1
    pub depth: i32,             // q depth before the op
    pub show: Option<usize>,
    pub snap: u32, // snapshot index valid after the op
    pub bt: bool,  // inside a text object after the op
    pub mc: i32,   // marked-content depth before the op
    pub tm: M,     // text matrix after the op
    pub tlm: M,    // text line matrix after the op
}

#[derive(Clone, Debug)]
pub struct Glyph {
    pub code: u32,
    pub nb: u8,
    pub text: Option<String>,
    pub x0: f64,
    pub x1: f64,
    pub w: f64,   // font units (1/1000 em)
    pub adv: f64, // text-space advance incl. Tc, Tw, Th
    pub kern: f64, // TJ adjustment after the glyph plus Tc, in 1/1000 em (positive = tighter)
}

#[derive(Clone, Debug)]
pub struct Show {
    pub kind: String,
    pub start: usize,
    pub end: usize,
    pub op: usize,
    pub font: String,
    pub fs: f64,
    pub tc: f64,
    pub tw: f64,
    pub th: f64,
    pub tr: i64,
    pub rise: f64,
    pub tm: M,
    pub ctm: M,
    pub elems: Vec<El>,
    pub glyphs: Vec<Glyph>,
    pub fill: String,
    pub base: f64,
    pub size: f64, // effective size in page space
    pub x0: f64,
    pub x1: f64,
    #[allow(dead_code)]
    pub font_known: bool,
}

impl Show {
    pub fn text(&self) -> String {
        self.glyphs.iter().map(|g| g.text.as_deref().unwrap_or("\u{fffd}")).collect()
    }
    pub fn blank(&self) -> bool {
        self.glyphs.iter().all(|g| g.text.as_deref().is_some_and(|t| t.trim().is_empty()))
    }
    /// Skew, rotation, mirroring or non-uniform scale: cannot be edited.
    pub fn rotated(&self) -> bool {
        let m = mul(self.tm, self.ctm);
        m[1].abs() > 1e-4 * m[3].abs().max(1e-9) || m[2].abs() > 1e-4 * m[3].abs().max(1e-9) || m[0] <= 0.0 || m[3] <= 0.0 || (m[0] - m[3]).abs() > 1e-3 * m[3].abs()
    }
}

pub struct Analysis {
    pub ops: Vec<Op>,
    pub shows: Vec<Show>,
    pub snaps: Vec<GS>,
    pub spans: Vec<(usize, usize, bool)>, // (open op, close op, is_bt)
    pub depth_end: i32,
    pub depth_min: i32,
}

pub fn fmt(x: f64) -> String {
    let s = format!("{:.4}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s
    }
}

/// Bytes of a code string as (code, raw) chunks: 2-byte codes for composite fonts.
pub fn chunks(s: &[u8], nb: u8) -> Vec<(u32, &[u8])> {
    if nb == 2 {
        s.chunks(2).map(|c| (c.iter().fold(0u32, |a, &b| a << 8 | b as u32), c)).collect()
    } else {
        s.iter().enumerate().map(|(i, &b)| (b as u32, &s[i..i + 1])).collect()
    }
}

/// Fonts by resource name -> codes shown, for the document-wide "already shown" rule.
pub fn codes_by_font(b: &[u8], fonts: &BTreeMap<String, Fnt>) -> HashMap<String, HashSet<u32>> {
    let an = analyse(b, fonts, &|_| None);
    let mut m: HashMap<String, HashSet<u32>> = HashMap::new();
    for s in &an.shows {
        m.entry(s.font.clone()).or_default().extend(s.glyphs.iter().map(|g| g.code));
    }
    m
}

/// `xobj(name)` = bbox of an XObject in its own space (images: unit square), for Do ops.
pub fn analyse(b: &[u8], fonts: &BTreeMap<String, Fnt>, xobj: &dyn Fn(&str) -> Option<[f64; 4]>) -> Analysis {
    analyse_with(b, fonts, xobj, ID)
}

/// `init` is the CTM the page starts with: the user-space -> upright-display matrix (page rotation, crop origin),
/// so every position, bbox and shift comes out in the frame the user sees.
pub fn analyse_with(b: &[u8], fonts: &BTreeMap<String, Fnt>, xobj: &dyn Fn(&str) -> Option<[f64; 4]>, init: M) -> Analysis {
    let toks = lex(b);
    let mut gs = GS::new();
    gs.ctm = init;
    let mut stack: Vec<(GS, usize)> = vec![];
    let mut snaps = vec![gs.clone()];
    let (mut tm, mut tlm) = (ID, ID);
    let mut path: Vec<(f64, f64)> = vec![];
    let mut pending_clip = false;
    let mut bt_open: Option<usize> = None;
    let mut mc = 0i32;
    let mut depth = 0i32;
    let mut a = Analysis { ops: vec![], shows: vec![], snaps: vec![], spans: vec![], depth_end: 0, depth_min: 0 };
    let n = |t: &T| if let K::Num(x) = t.k { x } else { 0.0 };
    // second, simpler loop: operands are the tokens since the previous operator
    let mut first = 0usize;
    for (ti, t) in toks.iter().enumerate() {
        let K::Op(op) = &t.k else { continue };
        let args: Vec<&T> = toks[first..ti].iter().collect();
        first = ti + 1;
        let nums: Vec<f64> = args.iter().map(|t| n(t)).collect();
        let s0 = args.first().map(|t| t.s).unwrap_or(t.s);
        let depth_before = depth;
        let mc_before = mc;
        let mut k = OK::Other;
        let mut bbox = None;
        let mut show = None;
        let mut state_changed = false;
        let text_of = |from: usize| String::from_utf8_lossy(&b[from..t.e]).trim().to_string();
        let num6 = |v: &[f64]| -> Option<M> { (v.len() >= 6).then(|| [v[v.len() - 6], v[v.len() - 5], v[v.len() - 4], v[v.len() - 3], v[v.len() - 2], v[v.len() - 1]]) };
        match op.as_str() {
            "q" => {
                stack.push((gs.clone(), a.ops.len()));
                depth += 1;
                k = OK::Q;
                state_changed = true;
            }
            "Q" => {
                if let Some((g, qi)) = stack.pop() {
                    gs = g;
                    a.spans.push((qi, a.ops.len(), false));
                }
                depth -= 1;
                a.depth_min = a.depth_min.min(depth);
                k = OK::EndQ;
                state_changed = true;
            }
            "cm" => {
                if let Some(m) = num6(&nums) {
                    gs.ctm = mul(m, gs.ctm);
                }
                k = OK::State;
                state_changed = true;
            }
            "w" => {
                gs.lw = nums.last().copied().unwrap_or(1.0);
                gs.lw_op = text_of(s0);
                k = OK::State;
                state_changed = true;
            }
            "J" => (gs.cap, k, state_changed) = (text_of(s0), OK::State, true),
            "j" => (gs.join, k, state_changed) = (text_of(s0), OK::State, true),
            "M" => (gs.miter, k, state_changed) = (text_of(s0), OK::State, true),
            "d" => (gs.dash, k, state_changed) = (text_of(s0), OK::State, true),
            "gs" => {
                if !gs.gs.is_empty() {
                    gs.gs.push(' ');
                }
                gs.gs.push_str(&text_of(s0));
                (k, state_changed) = (OK::State, true);
            }
            "g" | "rg" | "k" => (gs.fill_cs, gs.fill, k, state_changed) = (String::new(), text_of(s0), OK::State, true),
            "sc" | "scn" => (gs.fill, k, state_changed) = (text_of(s0), OK::State, true),
            "cs" => (gs.fill_cs, gs.fill, k, state_changed) = (text_of(s0), String::new(), OK::State, true),
            "G" | "RG" | "K" => (gs.stroke_cs, gs.stroke, k, state_changed) = (String::new(), text_of(s0), OK::State, true),
            "SC" | "SCN" => (gs.stroke, k, state_changed) = (text_of(s0), OK::State, true),
            "CS" => (gs.stroke_cs, gs.stroke, k, state_changed) = (text_of(s0), String::new(), OK::State, true),
            "Tc" => (gs.tc, k, state_changed) = (nums.last().copied().unwrap_or(0.0), OK::State, true),
            "Tw" => (gs.tw, k, state_changed) = (nums.last().copied().unwrap_or(0.0), OK::State, true),
            "Tz" => (gs.th, k, state_changed) = (nums.last().copied().unwrap_or(100.0) / 100.0, OK::State, true),
            "TL" => (gs.tl, k, state_changed) = (nums.last().copied().unwrap_or(0.0), OK::State, true),
            "Ts" => (gs.rise, k, state_changed) = (nums.last().copied().unwrap_or(0.0), OK::State, true),
            "Tr" => (gs.tr, k, state_changed) = (nums.last().copied().unwrap_or(0.0) as i64, OK::State, true),
            "Tf" => {
                if let Some(K::Name(nm)) = args.first().map(|t| &t.k) {
                    gs.font = nm.clone();
                }
                gs.fs = nums.get(1).copied().unwrap_or(0.0);
                k = OK::State;
                state_changed = true;
            }
            "BT" => {
                (tm, tlm) = (ID, ID);
                bt_open = Some(a.ops.len());
                k = OK::Bt;
            }
            "ET" => {
                if let Some(o) = bt_open.take() {
                    a.spans.push((o, a.ops.len(), true));
                }
                k = OK::Et;
            }
            "Tm" => {
                if let Some(m) = num6(&nums) {
                    (tm, tlm) = (m, m);
                }
            }
            "Td" | "TD" if nums.len() >= 2 => {
                let (x, y) = (nums[nums.len() - 2], nums[nums.len() - 1]);
                if op == "TD" {
                    gs.tl = -y;
                    state_changed = true;
                }
                tlm = mul(tr(x, y), tlm);
                tm = tlm;
            }
            "T*" => {
                tlm = mul(tr(0.0, -gs.tl), tlm);
                tm = tlm;
            }
            "m" | "l" if nums.len() >= 2 => path.push((nums[nums.len() - 2], nums[nums.len() - 1])),
            "c" if nums.len() >= 6 => path.extend(nums[nums.len() - 6..].chunks(2).map(|p| (p[0], p[1]))),
            "v" | "y" if nums.len() >= 4 => path.extend(nums[nums.len() - 4..].chunks(2).map(|p| (p[0], p[1]))),
            "re" if nums.len() >= 4 => {
                let v = &nums[nums.len() - 4..];
                path.extend([(v[0], v[1]), (v[0] + v[2], v[1]), (v[0] + v[2], v[1] + v[3]), (v[0], v[1] + v[3])]);
            }
            "BDC" | "BMC" => mc += 1,
            "EMC" => mc = (mc - 1).max(0),
            "W" | "W*" => pending_clip = true,
            "n" => {
                if pending_clip {
                    k = OK::Clip;
                }
                pending_clip = false;
                path.clear();
            }
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                k = OK::Paint;
                if !path.is_empty() {
                    let pts: Vec<(f64, f64)> = path.iter().map(|&(x, y)| apply(gs.ctm, x, y)).collect();
                    let pad = if matches!(op.as_str(), "f" | "F" | "f*") { 0.0 } else { gs.lw * 0.5 * gs.ctm[0].abs().max(gs.ctm[3].abs()) };
                    bbox = Some([
                        pts.iter().map(|p| p.0).fold(f64::MAX, f64::min) - pad,
                        pts.iter().map(|p| p.1).fold(f64::MAX, f64::min) - pad,
                        pts.iter().map(|p| p.0).fold(f64::MIN, f64::max) + pad,
                        pts.iter().map(|p| p.1).fold(f64::MIN, f64::max) + pad,
                    ]);
                } else {
                    k = OK::Other;
                }
                pending_clip = false;
                path.clear();
            }
            "sh" => k = OK::Paint, // no bbox: unbounded
            "BI" => {
                k = OK::Paint;
                bbox = Some(unit_box(gs.ctm, [0.0, 0.0, 1.0, 1.0]));
            }
            "Do" => {
                k = OK::Do;
                if let Some(K::Name(nm)) = args.first().map(|t| &t.k) {
                    bbox = xobj(nm).map(|bb| unit_box(gs.ctm, bb));
                }
            }
            "Tj" | "TJ" | "'" | "\"" => {
                let mut elems: Vec<El> = vec![];
                let mut start = t.s;
                let mut aw_ac = None;
                match op.as_str() {
                    "TJ" => {
                        if let Some(x) = args.last() {
                            if let K::Arr(e) = &x.k {
                                elems = e.clone();
                                start = x.s;
                            }
                        }
                    }
                    _ => {
                        if let Some(x) = args.last() {
                            if let K::Str(s) = &x.k {
                                elems.push(El::Str(s.clone()));
                                start = x.s;
                            }
                        }
                    }
                }
                if op == "\"" && nums.len() >= 3 {
                    aw_ac = Some((n(args[args.len() - 3]), n(args[args.len() - 2])));
                    start = args[args.len() - 3].s;
                }
                if op == "'" || op == "\"" {
                    tlm = mul(tr(0.0, -gs.tl), tlm);
                    tm = tlm;
                }
                if let Some((aw, ac)) = aw_ac {
                    gs.tw = aw;
                    gs.tc = ac;
                    state_changed = true;
                }
                let f = fonts.get(&gs.font);
                let m = mul(tm, gs.ctm);
                let mut glyphs = vec![];
                let mut xt = 0.0f64;
                for e in &elems {
                    match e {
                        El::Str(s) => {
                            for (code, raw) in chunks(s, f.map(|f| f.nb).unwrap_or(1)) {
                                let w = f.map(|f| f.w(code)).unwrap_or(0.0);
                                let adv = (w / 1000.0 * gs.fs + gs.tc + if raw.len() == 1 && code == 32 { gs.tw } else { 0.0 }) * gs.th;
                                glyphs.push(Glyph {
                                    code,
                                    nb: raw.len() as u8,
                                    text: f.and_then(|f| f.dec.get(&code).cloned()),
                                    x0: m[0] * xt + m[4],
                                    x1: m[0] * (xt + adv) + m[4],
                                    w,
                                    adv,
                                    kern: if gs.fs != 0.0 { -gs.tc * 1000.0 / gs.fs } else { 0.0 },
                                });
                                xt += adv;
                            }
                        }
                        El::Num(x) => {
                            xt += -x / 1000.0 * gs.fs * gs.th;
                            if let Some(g) = glyphs.last_mut() {
                                g.kern += x;
                            }
                        }
                    }
                }
                let size = gs.fs * m[2].hypot(m[3]);
                let (x0, x1) = (glyphs.first().map(|g| g.x0).unwrap_or(m[4]), glyphs.last().map(|g| g.x1).unwrap_or(m[4]).max(m[4] + m[0] * xt.max(0.0)));
                bbox = Some([x0.min(x1), m[5] - 0.22 * size, x0.max(x1), m[5] + 0.78 * size]);
                // number-only TJ (neutralised ops, text_restore) draws no glyph and pdfium makes no text object for it
                let draws = elems.iter().any(|e| matches!(e, El::Str(_)));
                if draws {
                show = Some(a.shows.len());
                k = OK::Show;
                a.shows.push(Show {
                    kind: op.clone(),
                    start,
                    end: t.e,
                    op: a.ops.len(),
                    font: gs.font.clone(),
                    fs: gs.fs,
                    tc: gs.tc,
                    tw: gs.tw,
                    th: gs.th,
                    tr: gs.tr,
                    rise: gs.rise,
                    tm,
                    ctm: gs.ctm,
                    elems,
                    glyphs,
                    fill: format!("{} {}", gs.fill_cs, gs.fill).trim().to_string(),
                    base: m[5],
                    size,
                    x0,
                    x1,
                    font_known: f.is_some(),
                });
                } else {
                    bbox = None;
                }
                tm = mul(tr(xt, 0.0), tm);
            }
            _ => {}
        }
        if state_changed {
            snaps.push(gs.clone());
        }
        a.ops.push(Op { s: s0, e: t.e, k, bbox, depth: depth_before, show, snap: snaps.len() as u32 - 1, bt: bt_open.is_some(), mc: mc_before, tm, tlm });
    }
    a.snaps = snaps;
    a.depth_end = depth;
    a
}

fn unit_box(ctm: M, b: [f64; 4]) -> [f64; 4] {
    let pts = [apply(ctm, b[0], b[1]), apply(ctm, b[2], b[1]), apply(ctm, b[2], b[3]), apply(ctm, b[0], b[3])];
    [
        pts.iter().map(|p| p.0).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.1).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.0).fold(f64::MIN, f64::max),
        pts.iter().map(|p| p.1).fold(f64::MIN, f64::max),
    ]
}

// ---------------- rewrite / splice ----------------

pub fn esc(b: &[u8]) -> String {
    let mut o = String::from("(");
    for &c in b {
        match c {
            b'(' | b')' | b'\\' => {
                o.push('\\');
                o.push(c as char);
            }
            32..=126 => o.push(c as char),
            _ => o.push_str(&format!("\\{:03o}", c)),
        }
    }
    o.push(')');
    o
}

/// Replacement for one show op: glyphs for which `keep(glyph index)` is false become a TJ adjustment of the same advance.
/// `'` and `"` keep their implicit T* / word + char spacing through explicit operators.
pub fn rewrite(s: &Show, keep: &dyn Fn(usize) -> bool) -> Vec<u8> {
    enum It {
        S(Vec<u8>),
        N(f64),
    }
    let mut items: Vec<It> = vec![];
    let push_n = |items: &mut Vec<It>, x: f64| {
        if let Some(It::N(p)) = items.last_mut() {
            *p += x;
        } else {
            items.push(It::N(x));
        }
    };
    let nb = s.glyphs.first().map(|g| g.nb).unwrap_or(1);
    let denom = s.fs * s.th;
    let mut gi = 0;
    for e in &s.elems {
        match e {
            El::Num(x) => push_n(&mut items, *x),
            El::Str(st) => {
                for (_, raw) in chunks(st, if nb == 2 { 2 } else { 1 }) {
                    let g = &s.glyphs[gi];
                    if keep(gi) {
                        if let Some(It::S(p)) = items.last_mut() {
                            p.extend_from_slice(raw);
                        } else {
                            items.push(It::S(raw.to_vec()));
                        }
                    } else if denom.abs() > 1e-9 {
                        push_n(&mut items, -g.adv * 1000.0 / denom);
                    }
                    gi += 1;
                }
            }
        }
    }
    let mut out = String::new();
    if s.kind == "'" {
        out.push_str("T* ");
    } else if s.kind == "\"" {
        let _ = write!(out, "{} Tw {} Tc T* ", fmt(s.tw), fmt(s.tc));
    }
    out.push('[');
    for it in &items {
        match it {
            It::S(b) => out.push_str(&esc(b)),
            It::N(x) if x.abs() > 1e-9 => {
                let _ = write!(out, " {} ", fmt(*x));
            }
            _ => {}
        }
    }
    out.push_str("] TJ");
    out.into_bytes()
}

/// Per glyph of a show op: its box in the analysis frame ([x0, y0, x1, y1]) and whether the text runs horizontally there.
pub fn glyph_boxes(s: &Show) -> Vec<([f64; 4], bool)> {
    let m = mul(s.tm, s.ctm);
    let horiz = m[0].abs() >= m[1].abs();
    let nb = s.glyphs.first().map(|g| g.nb).unwrap_or(1);
    let (mut xt, mut gi) = (0.0f64, 0usize);
    let mut out = vec![];
    let fs = s.fs.abs();
    for e in &s.elems {
        match e {
            El::Num(x) => xt += -x / 1000.0 * s.fs * s.th,
            El::Str(st) => {
                for _ in chunks(st, if nb == 2 { 2 } else { 1 }) {
                    let adv = s.glyphs[gi].adv;
                    let (y0, y1) = (s.rise - 0.15 * fs, s.rise + 0.75 * fs);
                    let pts = [(xt, y0), (xt + adv, y0), (xt + adv, y1), (xt, y1)].map(|(x, y)| apply(m, x, y));
                    out.push(([
                        pts.iter().map(|p| p.0).fold(f64::MAX, f64::min),
                        pts.iter().map(|p| p.1).fold(f64::MAX, f64::min),
                        pts.iter().map(|p| p.0).fold(f64::MIN, f64::max),
                        pts.iter().map(|p| p.1).fold(f64::MIN, f64::max),
                    ], horiz));
                    xt += adv;
                    gi += 1;
                }
            }
        }
    }
    out
}

fn overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

/// White-out rule (spike r2): more than half of the glyph's advance lies inside the rect and its line's y-range overlaps it.
pub fn glyph_hit(b: [f64; 4], horiz: bool, r: [f64; 4]) -> bool {
    let (ox, oy) = (overlap(b[0], b[2], r[0], r[2]), overlap(b[1], b[3], r[1], r[3]));
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    if horiz { w > 1e-6 && ox >= 0.5 * w && oy > 0.3 } else { h > 1e-6 && oy >= 0.5 * h && ox > 0.3 }
}

pub struct Mod {
    pub s: usize,
    pub e: usize,
    pub bytes: Vec<u8>,
    pub prio: i32,
}

/// Replaces byte ranges (and inserts at zero-length ranges) of `orig`; at equal start inserts go first, ordered by prio.
pub fn splice(orig: &[u8], mut mods: Vec<Mod>) -> Vec<u8> {
    mods.sort_by_key(|m| (m.s, m.prio));
    let mut out = Vec::with_capacity(orig.len() + 256);
    let mut p = 0;
    for m in mods {
        if m.s < p {
            continue; // overlapping replacement: the first one wins
        }
        out.extend_from_slice(&orig[p..m.s]);
        out.extend_from_slice(&m.bytes);
        p = m.e.max(m.s);
    }
    out.extend_from_slice(&orig[p..]);
    out
}

// ---------------- push helpers ----------------

/// Grows [lo, hi] (op indices) to whole q..Q groups and to marked-content boundaries, so `q ... Q` can be wrapped around it.
/// (A wrap inside a text object is done by splitting it: see `text_restore`.)
pub fn expand_range(an: &Analysis, mut lo: usize, mut hi: usize) -> (usize, usize) {
    loop {
        let mut changed = false;
        for &(s, e, bt) in &an.spans {
            if bt {
                continue;
            }
            let (in_lo, in_hi) = (s <= lo && lo <= e, s <= hi && hi <= e);
            if in_lo && !in_hi && s < lo {
                lo = s;
                changed = true;
            } else if in_hi && !in_lo && e > hi {
                hi = e;
                changed = true;
            }
        }
        while lo > 0 && an.ops[lo].mc != 0 {
            lo -= 1;
            changed = true;
        }
        while hi + 1 < an.ops.len() && an.ops[hi + 1].mc != 0 {
            hi += 1;
            changed = true;
        }
        if !changed {
            return (lo, hi);
        }
    }
}

/// Operators that re-establish the text matrices after a new `BT` that continues the text object ended at op `after`:
/// Tm := Tlm, then the advance since the line start as an empty TJ adjustment.
pub fn text_restore(an: &Analysis, after: Option<usize>) -> String {
    let (tm, tlm, gs) = match after {
        Some(i) => (an.ops[i].tm, an.ops[i].tlm, &an.snaps[an.ops[i].snap as usize]),
        None => (ID, ID, &an.snaps[0]),
    };
    let mut s = format!("{} {} {} {} {} {} Tm
", fmt(tlm[0]), fmt(tlm[1]), fmt(tlm[2]), fmt(tlm[3]), fmt(tlm[4]), fmt(tlm[5]));
    let n2 = tlm[0] * tlm[0] + tlm[1] * tlm[1];
    let adv = if n2 > 1e-12 { ((tm[4] - tlm[4]) * tlm[0] + (tm[5] - tlm[5]) * tlm[1]) / n2 } else { 0.0 };
    if adv.abs() > 1e-9 && gs.fs * gs.th != 0.0 && !gs.font.is_empty() {
        let _ = writeln!(s, "[{}] TJ", fmt(-adv * 1000.0 / (gs.fs * gs.th)));
    }
    s
}

/// `q 1 0 0 1 tx ty cm` that moves everything after it down by `dy` page points, whatever the CTM there is.
pub fn shift_open(ctm: M, dy: f64) -> Option<String> {
    let det = ctm[0] * ctm[3] - ctm[1] * ctm[2];
    if det.abs() < 1e-12 {
        return None;
    }
    let (vx, vy) = (dy * ctm[2] / det, -dy * ctm[0] / det);
    Some(format!("q 1 0 0 1 {} {} cm\n", fmt(vx), fmt(vy)))
}

/// Header / footer zones: ops whose bbox lies in the bottom (top) 15 % of the page and sit >= 2 pitches away from the content above (below).
/// Returns (foot_y, head_y): footer ops have top <= foot_y, header ops have bottom >= head_y.
pub fn zones(an: &Analysis, page: [f64; 4], pitch: f64) -> (f64, f64) {
    let (ox, oy, w, h) = (page[0], page[1], page[2], page[3]);
    let _ = ox;
    let boxes: Vec<[f64; 4]> = an
        .ops
        .iter()
        .filter_map(|o| o.bbox)
        .filter(|b| (b[3] - b[1]) < 0.4 * h && !((b[2] - b[0]) > 0.9 * w && (b[3] - b[1]) > 0.5 * h) && b[2] > b[0] - 1.0)
        .collect();
    let mut foot = f64::MIN;
    let mut by_top = boxes.clone();
    by_top.sort_by(|a, b| b[3].total_cmp(&a[3]));
    let mut low = f64::MAX;
    for b in &by_top {
        if b[3] <= oy + 0.15 * h && low != f64::MAX && low - b[3] >= 2.0 * pitch {
            foot = b[3];
            break;
        }
        low = low.min(b[1]);
    }
    let mut head = f64::MAX;
    let mut by_bot = boxes;
    by_bot.sort_by(|a, b| a[1].total_cmp(&b[1]));
    let mut high = f64::MIN;
    for b in &by_bot {
        if b[1] >= oy + 0.85 * h && high != f64::MIN && b[1] - high >= 2.0 * pitch {
            head = b[1];
            break;
        }
        high = high.max(b[3]);
    }
    (foot, head)
}

pub fn fill_css(fill: &str) -> String {
    let t: Vec<&str> = fill.split_whitespace().collect();
    let v = |i: usize| t.get(i).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0).clamp(0.0, 1.0);
    let (r, g, b) = match t.last().copied() {
        Some("g") if t.len() == 2 => (v(0), v(0), v(0)),
        Some("rg") if t.len() == 4 => (v(0), v(1), v(2)),
        Some("k") if t.len() == 5 => ((1.0 - v(0)) * (1.0 - v(3)), (1.0 - v(1)) * (1.0 - v(3)), (1.0 - v(2)) * (1.0 - v(3))),
        _ => (0.0, 0.0, 0.0),
    };
    format!("#{:02x}{:02x}{:02x}", (r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> BTreeMap<String, Fnt> {
        // synthetic simple font: every code is 500 units wide
        let mut f = Fnt::default();
        f.name = "F1".into();
        f.nb = 1;
        for c in 0..=255u32 {
            f.dec.insert(c, (c as u8 as char).to_string());
        }
        f.set_flat_widths(500.0);
        BTreeMap::from([("F1".to_string(), f)])
    }

    #[test]
    fn neutralise_keeps_advance_for_all_show_ops() {
        // a probe show after the neutralised op must land exactly where it landed before; the neutralised op itself draws no glyph
        for (orig, name) in [
            ("BT /F1 10 Tf 12 TL 1 0 0 1 50 700 Tm 2 Tc 3 Tw (ab c) Tj (Z) Tj ET", "Tj"),
            ("BT /F1 10 Tf 12 TL 1 0 0 1 50 700 Tm 2 Tc 3 Tw [(ab) -120 ( c)] TJ (Z) Tj ET", "TJ"),
            ("BT /F1 10 Tf 12 TL 1 0 0 1 50 700 Tm 2 Tc 3 Tw (ab c) ' (Z) Tj ET", "quote"),
            ("BT /F1 10 Tf 12 TL 1 0 0 1 50 700 Tm 5 1 (ab c) \" (Z) Tj ET", "dquote"),
        ] {
            let an = analyse(orig.as_bytes(), &fonts(), &|_| None);
            let s = &an.shows[0];
            let spliced = splice(orig.as_bytes(), vec![Mod { s: s.start, e: s.end, bytes: rewrite(s, &|_| false), prio: 0 }]);
            let an2 = analyse(&spliced, &fonts(), &|_| None);
            assert_eq!(an2.shows.len(), 1, "{name}: only the probe remains a show op");
            let (p, q) = (an.shows.last().unwrap(), an2.shows.last().unwrap());
            let (a, b) = (mul(p.tm, p.ctm), mul(q.tm, q.ctm));
            assert!((a[4] - b[4]).abs() < 1e-3 && (a[5] - b[5]).abs() < 1e-9, "{name}: {} vs {}", a[4], b[4]);
        }
    }

    #[test]
    fn lexer_survives_truncated_input() {
        for c in ["BT (abc", "BT [(abc) 12", "BT <4142", "<< /A (x", "BI /W 1 ID xx", "(a\\"] {
            let _ = analyse(c.as_bytes(), &fonts(), &|_| None);
        }
    }

    #[test]
    fn shift_matrix_follows_ctm() {
        // flipped, scaled coordinate system: a page-space move down by 10 pt is +10/0.1 in user space
        let m = shift_open([0.1, 0.0, 0.0, -0.1, 0.0, 800.0], 10.0).unwrap();
        assert_eq!(m, "q 1 0 0 1 0 100 cm\n");
        assert_eq!(shift_open(ID, 10.0).unwrap(), "q 1 0 0 1 0 -10 cm\n");
    }
}
