//! Edit PDF backend: inline text editing by byte-level surgery on the original content streams (lopdf)
//! plus one appended stream per edited page. Contract: _docs/edit-contract.md. Pages are 0-based everywhere.
//! Edits are data and the output is a pure function of (original bytes, effective edits); undo/redo only moves a cursor.
mod content;
mod fonts;
mod layout;

use crate::compress::{self, place, run_gs, size_of, stamp, temp_root, GsErr};
use crate::job::JobGuard;
use crate::pdf::{self, Opened};
use base64::Engine;
use content::{expand_range, restore_ops, rewrite, shift_open, splice, text_restore, zones, Analysis, Mod, OK};
use fonts::{deref, effective_resources, embed_pc, load_fonts, num, used_codes, Fnt};
use layout::{emit, find_paras, retype, typeset, Align, Fail, PageIn, Para, PcSet, Seg};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------- wire types ----------------

#[derive(Deserialize)]
pub struct EditOpenReq {
    path: String,
    password: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct EditDoc {
    pages: u32,
    sizes: Vec<(f32, f32)>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ParaOut {
    id: u32,
    bbox: [f64; 4],
    text: String,
    marker: Option<String>,
    align: Align,
    size: f64,
    pitch: f64,
    first_indent: f64,
    css_font: String,
    bold: bool,
    italic: bool,
    color: String,
    status: &'static str,
    reason: Option<&'static str>,
    edited: bool,
    pc_font_used: bool,
    rot: i64, // clockwise angle at which the text appears on the displayed page
}

#[derive(Serialize, Clone, Debug)]
pub struct PageModel {
    page: u32,
    paras: Vec<ParaOut>,
    scan: bool,
}

#[derive(Deserialize)]
pub struct EditApplyReq {
    page: u32,
    para: u32,
    text: String,
    use_pc_font: bool,
    allow_overlap: bool,
}

#[derive(Serialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditApplyRes {
    Ok { page: PageModel },
    MissingChars { chars: String },
    UnsupportedChars { chars: String },
    Overflow { lines_over: u32 },
    CannotPush,
}

#[derive(Serialize, Debug, Default)]
pub struct EditHistory {
    can_undo: bool,
    can_redo: bool,
    count: u32,
    changed_pages: Vec<u32>,
}

#[derive(Deserialize)]
pub struct EditSaveReq {
    out_mode: String,
}

#[derive(Serialize, Debug)]
pub struct EditSaveRes {
    output: String,
    size: u64,
    seconds: f64,
}

// ---------------- helpers on the lopdf side ----------------

fn inherited<'a>(doc: &'a Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut node = doc.get_dictionary(page).ok()?;
    for _ in 0..32 {
        if let Ok(o) = node.get(key) {
            return Some(deref(doc, o));
        }
        node = doc.get_dictionary(node.get(b"Parent").ok()?.as_reference().ok()?).ok()?;
    }
    None
}

/// [x origin, y origin, width, height] of the crop box (media box when absent).
fn page_box(doc: &Document, page: ObjectId) -> [f64; 4] {
    let get = |k: &[u8]| -> Option<[f64; 4]> {
        let a = inherited(doc, page, k)?.as_array().ok()?;
        (a.len() == 4).then(|| [num(deref(doc, &a[0])), num(deref(doc, &a[1])), num(deref(doc, &a[2])), num(deref(doc, &a[3]))])
    };
    let b = get(b"CropBox").or_else(|| get(b"MediaBox")).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    [b[0].min(b[2]), b[1].min(b[3]), (b[2] - b[0]).abs(), (b[3] - b[1]).abs()]
}

/// Bounding box of an XObject in its own space (images: unit square; forms: BBox through Matrix).
fn xobj_box(doc: &Document, page: ObjectId, name: &str) -> Option<[f64; 4]> {
    let xo = deref(doc, effective_resources(doc, page)?.get(b"XObject").ok()?).as_dict().ok()?;
    let Object::Stream(s) = deref(doc, xo.get(name.as_bytes()).ok()?) else { return None };
    if s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image") {
        return Some([0.0, 0.0, 1.0, 1.0]);
    }
    let a = deref(doc, s.dict.get(b"BBox").ok()?).as_array().ok()?;
    let v: Vec<f64> = a.iter().map(|o| num(deref(doc, o))).collect();
    if v.len() != 4 {
        return None;
    }
    let m: Vec<f64> = s.dict.get(b"Matrix").ok().and_then(|o| deref(doc, o).as_array().ok()).map(|a| a.iter().map(|o| num(deref(doc, o))).collect()).unwrap_or_default();
    if m.len() != 6 {
        return Some([v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])]);
    }
    let pts = [(v[0], v[1]), (v[2], v[1]), (v[2], v[3]), (v[0], v[3])].map(|(x, y)| (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5]));
    Some([
        pts.iter().map(|p| p.0).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.1).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.0).fold(f64::MIN, f64::max),
        pts.iter().map(|p| p.1).fold(f64::MIN, f64::max),
    ])
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------- page analysis ----------------

pub struct PageAn {
    pid: ObjectId,
    page: [f64; 4],
    scan: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    lock: Option<&'static str>,
    #[cfg_attr(not(test), allow(dead_code))]
    pdfium_texts: usize, // pdfium text objects, for the 1:1 check
    joined: Vec<u8>,
    an: Analysis,
    fonts: BTreeMap<String, Fnt>,
    paras: Vec<Para>,
    foot_y: f64,
    head_y: f64,
    rinv: content::M, // work frame -> content space
    disp: [f64; 2],   // displayed page size (the work frame is turned so the dominant text is upright)
    to_disp: content::M, // work frame -> displayed frame
    q: i64, // extra turn of the work frame, degrees
}

/// Quarter turn (0/90/180/270) that makes the page's dominant text direction run left to right.
fn dominant_turn(an: &Analysis) -> i64 {
    let mut w = [0usize; 4];
    for s in an.shows.iter().filter(|s| !s.blank()) {
        let m = content::mul(s.tm, s.ctm);
        let k = if m[0].abs() >= m[1].abs() { if m[0] > 0.0 { 0 } else { 2 } } else if m[1] > 0.0 { 1 } else { 3 };
        w[k] += s.glyphs.len();
    }
    let best = (0..4).max_by_key(|&k| w[k]).unwrap_or(0);
    best as i64 * 90
}

/// User space -> upright display frame (crop origin at 0, y up) for a page rotated clockwise by `rot` degrees.
fn display_matrix(b: [f64; 4], rot: i64) -> content::M {
    let (w, h) = (b[2], b[3]);
    let r = match rot {
        90 => [0.0, -1.0, 1.0, 0.0, 0.0, w],
        180 => [-1.0, 0.0, 0.0, -1.0, w, h],
        270 => [0.0, 1.0, -1.0, 0.0, h, 0.0],
        _ => content::ID,
    };
    content::mul(content::tr(-b[0], -b[1]), r)
}

fn walk(o: &PdfPageObject, form: bool, direct: &mut Vec<String>, forms: &mut Vec<Seg>) {
    if let Some(t) = o.as_text_object() {
        if !form {
            direct.push(t.text());
            return;
        }
        let text = t.text();
        if text.trim().is_empty() {
            return;
        }
        let Ok(q) = t.bounds().map(|q| q.to_rect()) else { return };
        let base = t.matrix().map(|m| m.f() as f64).unwrap_or(q.bottom().value as f64);
        forms.push(Seg {
            show: None,
            x0: q.left().value as f64,
            x1: q.right().value as f64,
            base,
            size: t.scaled_font_size().value as f64,
            text,
            class: 2,
            reason: Some("form"),
            fam: layout::family(&t.font().name()),
        });
    } else if let Some(x) = o.as_x_object_form_object() {
        for c in x.iter() {
            walk(&c, true, direct, forms);
        }
    }
}

fn analyse_page(doc: &Document, orig: &[u8], idx: u32) -> Result<PageAn, String> {
    let pid = *doc.get_pages().get(&(idx + 1)).ok_or("bad page")?;
    let raw_box = page_box(doc, pid);
    let rotate = inherited(doc, pid, b"Rotate").map(|o| num(o) as i64).unwrap_or(0);
    let rot = rotate.rem_euclid(360);
    let mut rmat = display_matrix(raw_box, if rot % 90 == 0 { rot } else { 0 });
    let disp = if rot == 90 || rot == 270 { [raw_box[3], raw_box[2]] } else { [raw_box[2], raw_box[3]] };
    let mut page = [0.0, 0.0, disp[0], disp[1]];
    let mut to_disp = content::ID;
    let fonts = load_fonts(doc, pid);
    let joined = doc.get_page_content(pid).map_err(|e| e.to_string())?;
    let mut an = content::analyse_with(&joined, &fonts, &|n| xobj_box(doc, pid, n), rmat);
    let q = dominant_turn(&an);
    if q != 0 {
        let qm = display_matrix([0.0, 0.0, disp[0], disp[1]], q);
        rmat = content::mul(rmat, qm);
        to_disp = content::inv(qm).unwrap_or(content::ID);
        if q == 90 || q == 270 {
            page = [0.0, 0.0, disp[1], disp[0]];
        }
        an = content::analyse_with(&joined, &fonts, &|n| xobj_box(doc, pid, n), rmat);
    }
    // pdfium side: text objects for the 1:1 check and undecodable codes, form text, image coverage
    let (direct, forms, coverage) = {
        let p = pdf::pdfium()?;
        let d = p.load_pdf_from_byte_vec(orig.to_vec(), None).map_err(|e| e.to_string())?;
        let pg = d.pages().get(idx as PdfPageIndex).map_err(|e| e.to_string())?;
        let (mut direct, mut forms) = (vec![], vec![]);
        for o in pg.objects().iter() {
            walk(&o, false, &mut direct, &mut forms);
        }
        (direct, forms, pdf::image_coverage(&pg))
    };
    // form text comes from pdfium in user space: into the display frame
    let forms: Vec<Seg> = forms
        .into_iter()
        .map(|mut s| {
            let (y0, y1) = (s.base - 0.22 * s.size, s.base + 0.78 * s.size);
            let pts = [(s.x0, y0), (s.x1, y0), (s.x1, y1), (s.x0, y1)].map(|(x, y)| (x * rmat[0] + y * rmat[2] + rmat[4], x * rmat[1] + y * rmat[3] + rmat[5]));
            s.x0 = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            s.x1 = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            s.base = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min) + 0.22 * s.size;
            s
        })
        .collect();
    let lock_all = if rot % 90 != 0 {
        Some("rotated")
    } else if direct.len() != an.shows.len() {
        Some("structure")
    } else {
        None
    };
    if lock_all.is_none() {
        for (s, t) in an.shows.iter_mut().zip(direct.iter()) {
            if s.glyphs.iter().any(|g| g.text.is_none()) {
                let cs: Vec<char> = t.chars().collect();
                let ok = cs.len() == s.glyphs.len();
                for (i, g) in s.glyphs.iter_mut().enumerate() {
                    if g.text.is_none() {
                        g.text = Some(if ok { cs[i].to_string() } else { "\u{fffd}".into() });
                    }
                }
            }
        }
    }
    let (mut paras, pitch) = find_paras(&PageIn { shows: &an.shows, fonts: &fonts, forms, page });
    if let Some(r) = lock_all {
        for p in paras.iter_mut() {
            p.locked = Some(r);
            p.ed = None;
        }
    }
    let (foot_y, head_y) = zones(&an, page, pitch);
    Ok(PageAn { pid, page, lock: lock_all, pdfium_texts: direct.len(), scan: coverage >= pdf::IMAGE_DOMINANT_PCT, joined, an, fonts, paras, foot_y, head_y, rinv: content::inv(rmat).unwrap_or(content::ID), disp, to_disp, q })
}

// ---------------- edits and building ----------------

#[derive(Clone, Debug, PartialEq)]
struct Edit {
    page: u32,
    para: u32,
    text: String,
    use_pc: bool,
    allow_overlap: bool,
}

#[derive(Debug)]
enum BErr {
    Missing(String),
    Unsupported(String),
    Overflow(u32),
    CannotPush,
    Other(String),
}

impl From<String> for BErr {
    fn from(s: String) -> Self {
        BErr::Other(s)
    }
}

#[derive(Default, Clone)]
struct PageInfo {
    shift: Vec<f64>,           // per paragraph: how far it moved down, pt
    lines: HashMap<u32, usize>, // edited paragraph -> new line count
    pc: HashSet<u32>,          // paragraphs retyped in a PC font
    ext: HashMap<u32, (f64, f64)>, // edited paragraph -> (left, right) of its new lines
}

struct Built {
    bytes: Vec<u8>,
    info: HashMap<u32, PageInfo>,
    pc_used: bool,
}

struct Plan<'a> {
    para: &'a Para,
    ed: &'a layout::EditData,
    styles: Vec<layout::Style>,
    words: Vec<Vec<layout::Ch>>,
    lines: Vec<layout::LayLine>,
    dy: f64, // down, page points
    edit: &'a Edit,
}

struct Push {
    lo: usize,
    hi: usize,
    dy: f64,
    moving: HashSet<usize>,
    bottom: f64,
    owner: u32,
}

fn fail(f: Fail) -> BErr {
    match f {
        Fail::Missing(c) => BErr::Missing(c),
        Fail::Unsupported(c) => BErr::Unsupported(c),
        Fail::NoPcFont => BErr::Other("no_pc_font".into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_page(
    doc: &mut Document,
    pa: &PageAn,
    edits: &[&Edit],
    used: &HashMap<ObjectId, HashSet<u32>>,
    floor_override: Option<f64>,
    pcs: &mut PcSet,
    pc_objs: &mut HashMap<usize, ObjectId>,
    old_streams: &mut Vec<ObjectId>,
) -> Result<PageInfo, BErr> {
    let [_, oy, _, _] = pa.page;
    let mut plans: Vec<Plan> = vec![];
    for e in edits {
        let para = pa.paras.get(e.para as usize).ok_or_else(|| BErr::Other("no such paragraph".into()))?;
        let ed = para.ed.as_ref().ok_or_else(|| BErr::Other("locked".into()))?;
        if para.pc_font && !e.use_pc {
            return Err(BErr::Missing(e.text.chars().filter(|c| !c.is_whitespace()).collect()));
        }
        let (styles, words) = retype(ed, &pa.fonts, used, &e.text, if e.use_pc || para.pc_font { Some(&mut *pcs) } else { None }).map_err(fail)?;
        let lines = if words.is_empty() { vec![] } else { typeset(ed, &styles, &words) };
        let dy = (lines.len() as f64 - ed.nlines as f64) * ed.pitch;
        plans.push(Plan { para, ed, styles, words, lines, dy, edit: e });
    }
    let neut: HashSet<usize> = plans.iter().flat_map(|p| p.ed.shows.iter().map(|&s| pa.an.shows[s].op)).collect();
    // ---- pushes
    let mut pushes: Vec<Push> = vec![];
    let ops = &pa.an.ops;
    for p in &plans {
        if p.dy.abs() < 1e-6 {
            continue;
        }
        let moving: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter(|(i, o)| {
                let drawing = match o.k {
                    OK::Show => !pa.an.shows[o.show.unwrap()].blank(),
                    OK::Paint | OK::Do => true,
                    _ => false,
                };
                drawing && !neut.contains(i) && o.bbox.is_some_and(|b| b[3] <= p.ed.bottom + 1.0 && b[3] > pa.foot_y && b[1] < pa.head_y)
            })
            .map(|(i, _)| i)
            .collect();
        let skip = |what: BErr| -> Result<(), BErr> { if p.edit.allow_overlap { Ok(()) } else { Err(what) } };
        let (Some(&first), Some(&last)) = (moving.first(), moving.last()) else { continue };
        let (lo, hi) = expand_range(&pa.an, first, last);
        let mset: HashSet<usize> = moving.iter().copied().collect();
        let bad_op = (lo..=hi).any(|i| {
            let o = &ops[i];
            let drawing = match o.k {
                OK::Show => !pa.an.shows[o.show.unwrap()].blank(),
                OK::Paint | OK::Do => true,
                _ => false,
            };
            (drawing && !mset.contains(&i) && !neut.contains(&i)) || (o.k == OK::Clip && o.depth == ops[lo].depth)
        });
        let before = &pa.an.snaps[if lo == 0 { 0 } else { ops[lo - 1].snap as usize }];
        let after = &pa.an.snaps[ops[hi].snap as usize];
        if bad_op || restore_ops(before, after).is_none() || shift_open(before.ctm, p.dy).is_none() {
            skip(BErr::CannotPush)?;
            continue;
        }
        pushes.push(Push { lo, hi, dy: p.dy, moving: mset, bottom: p.ed.bottom, owner: p.para.id });
    }
    // how far each paragraph ends up moving
    let shift_of = |top: f64, id: u32| -> f64 { pushes.iter().filter(|a| a.owner != id && top <= a.bottom + 1.0 && top > pa.foot_y).map(|a| a.dy).sum() };
    // ---- bottom limit: the footer zone (plus a pitch) or 0.5 inch from the page edge. When the pushed content would
    // go past it the edit is refused; with allow_overlap nothing is pushed at all and the paragraph overlaps what is below.
    if plans.iter().any(|p| p.dy > 0.0) {
        let pitch = plans[0].ed.pitch.max(1.0);
        let floor = floor_override.unwrap_or_else(|| (oy + 36.0).max(if pa.foot_y > f64::MIN / 2.0 { pa.foot_y + pitch } else { f64::MIN }));
        let mut lowest = f64::MAX;
        for a in &pushes {
            for &i in &a.moving {
                let total: f64 = pushes.iter().filter(|x| x.moving.contains(&i)).map(|x| x.dy).sum();
                lowest = lowest.min(ops[i].bbox.unwrap()[1] - total);
            }
        }
        for p in &plans {
            if !p.lines.is_empty() {
                lowest = lowest.min(p.ed.base0 - shift_of(p.ed.top, p.para.id) - p.ed.pitch * (p.lines.len() as f64 - 1.0) - 0.22 * p.ed.size);
            }
        }
        if lowest < floor - 0.5 {
            if plans.iter().any(|p| p.edit.allow_overlap) {
                pushes.clear();
            } else {
                return Err(BErr::Overflow(((floor - lowest) / pitch).ceil().max(1.0) as u32));
            }
        }
    }
    let shift_of = |top: f64, id: u32| -> f64 { pushes.iter().filter(|a| a.owner != id && top <= a.bottom + 1.0 && top > pa.foot_y).map(|a| a.dy).sum() };
    let mut info = PageInfo { shift: pa.paras.iter().map(|p| shift_of(p.top, p.id)).collect(), ..Default::default() };
    for p in &plans {
        info.lines.insert(p.para.id, p.lines.len());
        if !p.lines.is_empty() {
            info.ext.insert(p.para.id, layout::extent(&p.styles, &p.words, &p.lines));
        }
        if p.edit.use_pc || p.para.pc_font {
            info.pc.insert(p.para.id);
        }
    }
    // ---- new content: neutralised ops + wrapped push ranges
    let mut mods: Vec<Mod> = vec![];
    for p in &plans {
        for &sid in &p.ed.shows {
            let s = &pa.an.shows[sid];
            mods.push(Mod { s: s.start, e: s.end, bytes: rewrite(s, &|_| false), prio: 0 });
        }
    }
    let mut order: Vec<usize> = (0..pushes.len()).collect();
    order.sort_by_key(|&i| (pushes[i].lo, std::cmp::Reverse(pushes[i].hi)));
    for (w, &i) in order.iter().enumerate() {
        let a = &pushes[i];
        let before = &pa.an.snaps[if a.lo == 0 { 0 } else { ops[a.lo - 1].snap as usize }];
        let after = &pa.an.snaps[ops[a.hi].snap as usize];
        let open = shift_open(before.ctm, a.dy).unwrap();
        // inside a text object the wrap splits it: ET q cm BT <matrices> ... ET Q BT <matrices>
        let open = if a.lo > 0 && ops[a.lo - 1].bt { format!("ET
{open}BT
{}", text_restore(&pa.an, Some(a.lo - 1))) } else { open };
        mods.push(Mod { s: ops[a.lo].s, e: ops[a.lo].s, bytes: open.into_bytes(), prio: -1000 + w as i32 });
        // inside a text object the close splits it too: ET Q BT <matrices> <state>
        let close = if ops[a.hi].bt {
            format!("
ET
Q
BT
{}{}
", text_restore(&pa.an, Some(a.hi)), restore_ops(before, after).unwrap())
        } else {
            format!("
Q
{}
", restore_ops(before, after).unwrap())
        };
        mods.push(Mod { s: ops[a.hi].e, e: ops[a.hi].e, bytes: close.into_bytes(), prio: -2000 - w as i32 });
    }
    // a stray extra Q in the original (the signed sample has one) would pop our wrapper, so it gets matching q's
    let extra = (-pa.an.depth_min).max(0);
    let mut body = b"q
".repeat(1 + extra as usize);
    body.extend(splice(&pa.joined, mods));
    body.extend(b"\n");
    for _ in 0..(1 + extra + pa.an.depth_end).max(0) {
        body.extend(b"Q\n");
    }
    let mut added = b"q\n0 Tc 100 Tz 0 Ts 0 Tr\n".to_vec();
    for p in &plans {
        if !p.lines.is_empty() {
            added.extend(emit(p.ed, &p.styles, &p.words, &p.lines, shift_of(p.ed.top, p.para.id), pa.rinv));
        }
    }
    added.extend(b"Q\n");
    // ---- fonts of the PC fallback into the page resources
    let pc_names: HashSet<&str> = plans.iter().flat_map(|p| p.styles.iter().map(|s| s.font.as_str())).filter(|n| n.starts_with("MjF")).collect();
    if !pc_names.is_empty() {
        let mut res = effective_resources(doc, pa.pid).cloned().unwrap_or_default();
        let mut fd = res.get(b"Font").ok().and_then(|o| deref(doc, o).as_dict().ok()).cloned().unwrap_or_default();
        for n in pc_names {
            let fi = n.trim_start_matches("MjF").parse::<usize>().unwrap_or(1) - 1;
            let id = *pc_objs.entry(fi).or_insert_with(|| embed_pc(doc, &pcs.faces[fi].1));
            fd.set(n.as_bytes().to_vec(), Object::Reference(id));
        }
        res.set("Font", Object::Dictionary(fd));
        if let Ok(Object::Dictionary(pd)) = doc.get_object_mut(pa.pid) {
            pd.set("Resources", Object::Dictionary(res));
        }
    }
    // ---- contents: two new streams; the old ones are dropped at the end when nothing else uses them
    old_streams.extend(doc.get_page_contents(pa.pid));
    let mut ids = vec![];
    for bytes in [body, added] {
        let mut s = Stream::new(Dictionary::new(), bytes);
        let _ = s.compress();
        ids.push(Object::Reference(doc.add_object(s)));
    }
    if let Ok(Object::Dictionary(pd)) = doc.get_object_mut(pa.pid) {
        pd.set("Contents", Object::Array(ids));
    }
    Ok(info)
}

// ---------------- session ----------------

struct Session {
    orig: Vec<u8>,
    doc: Document,
    path: PathBuf,
    sizes: Vec<(f32, f32)>,
    pages: HashMap<u32, Arc<PageAn>>,
    hist: Vec<Edit>,
    cursor: usize,
    built: Option<Arc<Built>>,
    used: Option<Arc<HashMap<ObjectId, HashSet<u32>>>>,
    floor_override: Option<f64>, // tests only
    _guard: Option<JobGuard>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn with_session<T>(f: impl FnOnce(&mut Session) -> Result<T, String>) -> Result<T, String> {
    let mut g = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    f(g.as_mut().ok_or("no edit session")?)
}

fn to_bytes(path: &Path, pw: Option<&str>, locked: bool) -> Result<(Vec<u8>, Document), String> {
    let raw = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if let Ok(d) = Document::load_mem(&raw).map_err(|_| ()).and_then(|d| if locked { Err(()) } else { Ok(d) }) {
        if !d.is_encrypted() {
            return Ok((raw, d));
        }
    }
    // encrypted or damaged for lopdf: let pdfium write a clean, decrypted copy first
    let bytes = {
        let p = pdf::pdfium()?;
        // pdfium keeps the encryption when it re-saves a document, so the pages go into a fresh one
        let d = p.load_pdf_from_file(path, pw).map_err(|e| e.to_string())?;
        let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
        let n = d.pages().len();
        out.pages_mut().copy_page_range_from_document(&d, 0..=n.saturating_sub(1), 0).map_err(|e| e.to_string())?;
        out.save_to_bytes().map_err(|e| e.to_string())?
    };
    let d = Document::load_mem(&bytes).map_err(|e| format!("cannot parse the PDF: {e}"))?;
    Ok((bytes, d))
}

impl Session {
    fn open(path: &Path, pw: Option<&str>, guard: Option<JobGuard>) -> Result<(Session, EditDoc), String> {
        match pdf::open(path, pw) {
            Opened::Ok { .. } => {}
            Opened::NeedsPassword => return Err("password required or wrong".into()),
            Opened::Damaged => return Err("pdfium could not open the file".into()),
        }
        let (orig, doc) = to_bytes(path, pw, matches!(pdf::open(path, None), Opened::NeedsPassword))?;
        let sizes: Vec<(f32, f32)> = {
            let p = pdf::pdfium()?;
            let d = p.load_pdf_from_byte_vec(orig.clone(), None).map_err(|e| e.to_string())?;
            let v = d.pages().iter().map(|pg| (pg.width().value, pg.height().value)).collect();
            v
        };
        let edoc = EditDoc { pages: sizes.len() as u32, sizes: sizes.clone() };
        let s = Session { orig, doc, path: path.to_path_buf(), sizes, pages: HashMap::new(), hist: vec![], cursor: 0, built: None, used: None, floor_override: None, _guard: guard };
        Ok((s, edoc))
    }

    fn page(&mut self, idx: u32) -> Result<Arc<PageAn>, String> {
        if idx as usize >= self.sizes.len() {
            return Err("bad page".into());
        }
        if let Some(p) = self.pages.get(&idx) {
            return Ok(p.clone());
        }
        let pa = Arc::new(analyse_page(&self.doc, &self.orig, idx)?);
        self.pages.insert(idx, pa.clone());
        Ok(pa)
    }

    /// Last edit per paragraph among the applied history; edits that restore the original text vanish.
    fn effective(&mut self) -> Vec<Edit> {
        let mut m: BTreeMap<(u32, u32), Edit> = BTreeMap::new();
        for e in &self.hist[..self.cursor] {
            m.insert((e.page, e.para), e.clone());
        }
        let v: Vec<Edit> = m.into_values().collect();
        v.into_iter().filter(|e| e.use_pc || self.page(e.page).ok().and_then(|p| p.paras.get(e.para as usize).map(|q| norm(&q.text) != e.text)).unwrap_or(true)).collect()
    }

    fn build(&mut self, edits: &[Edit]) -> Result<Built, BErr> {
        if edits.is_empty() {
            return Ok(Built { bytes: self.orig.clone(), info: HashMap::new(), pc_used: false });
        }
        if self.used.is_none() {
            self.used = Some(Arc::new(used_codes(&self.doc)));
        }
        let used = self.used.clone().unwrap();
        let mut doc = self.doc.clone();
        let (mut pcs, mut pc_objs, mut old) = (PcSet::default(), HashMap::new(), vec![]);
        let mut info = HashMap::new();
        let mut pages: Vec<u32> = edits.iter().map(|e| e.page).collect();
        pages.dedup();
        for pg in pages {
            let pa = self.page(pg)?;
            let es: Vec<&Edit> = edits.iter().filter(|e| e.page == pg).collect();
            info.insert(pg, build_page(&mut doc, &pa, &es, &used, self.floor_override, &mut pcs, &mut pc_objs, &mut old)?);
        }
        let live: HashSet<ObjectId> = doc.page_iter().flat_map(|p| doc.get_page_contents(p)).collect();
        for id in old.into_iter().filter(|i| !live.contains(i)) {
            doc.objects.remove(&id);
        }
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).map_err(|e| BErr::Other(e.to_string()))?;
        Ok(Built { bytes, info, pc_used: !pc_objs.is_empty() })
    }

    fn built(&mut self) -> Result<Arc<Built>, String> {
        if self.built.is_none() {
            let e = self.effective();
            let b = self.build(&e).map_err(|e| match e {
                BErr::Other(s) => s,
                _ => "cannot rebuild the edited document".to_string(),
            })?;
            self.built = Some(Arc::new(b));
        }
        Ok(self.built.clone().unwrap())
    }

    fn model(&mut self, idx: u32) -> Result<PageModel, String> {
        let pa = self.page(idx)?;
        let b = self.built()?;
        let edits = self.effective();
        let info = b.info.get(&idx);
        let paras = pa
            .paras
            .iter()
            .map(|p| {
                let e = edits.iter().find(|e| e.page == idx && e.para == p.id);
                let shift = info.and_then(|i| i.shift.get(p.id as usize)).copied().unwrap_or(0.0);
                let nl = info.and_then(|i| i.lines.get(&p.id)).copied().unwrap_or(p.nlines);
                let (top, bot) = (p.top - shift, p.bot - shift - (nl as f64 - p.nlines as f64) * p.pitch);
                ParaOut {
                    id: p.id,
                    bbox: {
                        let (l, r) = info.and_then(|i| i.ext.get(&p.id)).copied().unwrap_or((p.l, p.r));
                        // work frame -> displayed page, top-left origin
                        let t = pa.to_disp;
                        let pts = [(l, bot), (r, bot), (r, top.max(bot)), (l, top.max(bot))].map(|(x, y)| (x * t[0] + y * t[2] + t[4], x * t[1] + y * t[3] + t[5]));
                        let (x0, x1) = (pts.iter().map(|p| p.0).fold(f64::MAX, f64::min), pts.iter().map(|p| p.0).fold(f64::MIN, f64::max));
                        let (y0, y1) = (pts.iter().map(|p| p.1).fold(f64::MAX, f64::min), pts.iter().map(|p| p.1).fold(f64::MIN, f64::max));
                        [x0, pa.disp[1] - y1, x1 - x0, y1 - y0]
                    },
                    text: e.map(|e| e.text.clone()).unwrap_or_else(|| p.text.clone()),
                    marker: p.marker.clone(),
                    align: p.align,
                    size: p.size,
                    pitch: p.pitch,
                    first_indent: p.first_indent,
                    css_font: p.css_font.clone(),
                    bold: p.bold,
                    italic: p.italic,
                    color: p.color.clone(),
                    status: if p.locked.is_some() { "locked" } else if p.pc_font { "pc_font" } else { "direct" },
                    reason: p.locked,
                    edited: e.is_some(),
                    pc_font_used: info.is_some_and(|i| i.pc.contains(&p.id)),
                    rot: (360 - pa.q) % 360,
                }
            })
            .collect();
        Ok(PageModel { page: idx, paras, scan: pa.scan })
    }

    fn history(&mut self, touched: Vec<u32>) -> EditHistory {
        let mut pages: Vec<u32> = self.effective().iter().map(|e| e.page).chain(touched).collect();
        pages.sort_unstable();
        pages.dedup();
        EditHistory { can_undo: self.cursor > 0, can_redo: self.cursor < self.hist.len(), count: self.effective().len() as u32, changed_pages: pages }
    }

    fn apply(&mut self, req: &EditApplyReq) -> Result<EditApplyRes, String> {
        let pa = self.page(req.page)?;
        let para = pa.paras.get(req.para as usize).ok_or("no such paragraph")?;
        if para.locked.is_some() {
            return Err("locked".into());
        }
        let text = norm(&req.text);
        let e = Edit { page: req.page, para: req.para, text, use_pc: req.use_pc_font, allow_overlap: req.allow_overlap };
        let mut trial: Vec<Edit> = self.effective().into_iter().filter(|x| !(x.page == e.page && x.para == e.para)).collect();
        let noop = !e.use_pc && norm(&para.text) == e.text;
        if !noop {
            trial.push(e.clone());
            trial.sort_by_key(|x| (x.page, x.para));
        }
        let had = self.effective().iter().any(|x| x.page == e.page && x.para == e.para);
        if noop && !had {
            return Ok(EditApplyRes::Ok { page: self.model(req.page)? });
        }
        match self.build(&trial) {
            Ok(b) => {
                self.hist.truncate(self.cursor);
                self.hist.push(e);
                self.cursor += 1;
                self.built = Some(Arc::new(b));
                Ok(EditApplyRes::Ok { page: self.model(req.page)? })
            }
            Err(BErr::Missing(chars)) => Ok(EditApplyRes::MissingChars { chars }),
            Err(BErr::Unsupported(chars)) => Ok(EditApplyRes::UnsupportedChars { chars }),
            Err(BErr::Overflow(n)) => Ok(EditApplyRes::Overflow { lines_over: n }),
            Err(BErr::CannotPush) => Ok(EditApplyRes::CannotPush),
            Err(BErr::Other(s)) => Err(s),
        }
    }

    fn step(&mut self, back: bool) -> Result<EditHistory, String> {
        let touched: Vec<u32> = self.effective().iter().map(|e| e.page).collect();
        let at = if back { self.cursor.checked_sub(1) } else { (self.cursor < self.hist.len()).then_some(self.cursor) };
        let Some(at) = at else { return Ok(self.history(vec![])) };
        let page = self.hist[at].page;
        self.cursor = if back { at } else { at + 1 };
        self.built = None;
        self.built()?;
        Ok(self.history(touched.into_iter().chain([page]).collect()))
    }

    fn render(&mut self, page: u32, width_px: u32) -> Result<String, String> {
        let b = self.built()?;
        let img = render_bytes(&b.bytes, page, width_px.clamp(200, 2400))?;
        let mut buf = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85).encode_image(&image::DynamicImage::ImageRgba8(img).to_rgb8()).map_err(|e| e.to_string())?;
        Ok(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(buf)))
    }
}

fn render_bytes(bytes: &[u8], page: u32, width_px: u32) -> Result<image::RgbaImage, String> {
    let p = pdf::pdfium()?;
    let d = p.load_pdf_from_byte_vec(bytes.to_vec(), None).map_err(|e| e.to_string())?;
    let pg = d.pages().get(page as PdfPageIndex).map_err(|e| e.to_string())?;
    let img = pg.render_with_config(&PdfRenderConfig::new().set_target_width(width_px as i32)).map_err(|e| e.to_string())?.as_image().map_err(|e| e.to_string())?.to_rgba8();
    Ok(img)
}

// ---------------- save ----------------

const GS_FLAGS: [&str; 9] = [
    "-sDEVICE=pdfwrite",
    "-dPassThroughJPEGImages=true",
    "-dDownsampleColorImages=false",
    "-dDownsampleGrayImages=false",
    "-dDownsampleMonoImages=false",
    "-dAutoFilterColorImages=false",
    "-dAutoFilterGrayImages=false",
    "-dCompatibilityLevel=1.6",
    "-dNOPAUSE",
];

/// Whole-page 72 DPI comparison of two files; true when every page differs in at most 0.2 % of its pixels.
fn same_render(a: &Path, b: &Path) -> bool {
    let Ok(p) = pdf::pdfium() else { return false };
    let (Ok(da), Ok(db)) = (p.load_pdf_from_file(a, None), p.load_pdf_from_file(b, None)) else { return false };
    if da.pages().len() != db.pages().len() {
        return false;
    }
    let cfg = PdfRenderConfig::new().scale_page_by_factor(1.0);
    for i in 0..da.pages().len() {
        let (Ok(x), Ok(y)) = (da.pages().get(i), db.pages().get(i)) else { return false };
        let (Ok(x), Ok(y)) = (x.render_with_config(&cfg).and_then(|r| r.as_image()), y.render_with_config(&cfg).and_then(|r| r.as_image())) else { return false };
        let (x, y) = (x.to_rgba8(), y.to_rgba8());
        if x.dimensions() != y.dimensions() {
            return false;
        }
        let diff = x.pixels().zip(y.pixels()).filter(|(p, q)| (0..3).any(|c| (p[c] as i32 - q[c] as i32).abs() > 16)).count();
        if diff as f64 > 0.002 * (x.width() * x.height()) as f64 {
            return false;
        }
    }
    true
}

fn chars_of(s: &str) -> HashMap<char, usize> {
    let mut m = HashMap::new();
    for c in s.chars().filter(|c| !c.is_whitespace()) {
        *m.entry(c).or_default() += 1;
    }
    m
}

/// Reopens the output: same page count, and every character of the new paragraphs is on its page.
fn verify(out: &Path, pages: usize, edits: &[(u32, String)]) -> Result<(), String> {
    let p = pdf::pdfium()?;
    let d = p.load_pdf_from_file(out, None).map_err(|_| "verify".to_string())?;
    if d.pages().len() as usize != pages {
        return Err("verify".into());
    }
    for pg in edits.iter().map(|e| e.0).collect::<HashSet<_>>() {
        let page = d.pages().get(pg as PdfPageIndex).map_err(|_| "verify".to_string())?;
        let text = page.text().map(|t| t.all()).map_err(|_| "verify".to_string())?;
        let have = chars_of(&text);
        let mut want: HashMap<char, usize> = HashMap::new();
        for (_, t) in edits.iter().filter(|e| e.0 == pg) {
            for (c, n) in chars_of(t) {
                *want.entry(c).or_default() += n;
            }
        }
        if want.iter().any(|(c, n)| have.get(c).copied().unwrap_or(0) < *n) {
            return Err("verify".into());
        }
    }
    Ok(())
}

impl Session {
    fn save(&mut self, out_dir: &Path) -> Result<EditSaveRes, String> {
        let start = std::time::Instant::now();
        let edits = self.effective();
        if edits.is_empty() {
            return Err("nothing_to_save".into());
        }
        let b = self.built()?;
        let work = temp_root().join(stamp());
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        let r = (|| {
            let (raw, tmp) = (work.join("raw.pdf"), work.join("out.pdf"));
            std::fs::write(&raw, &b.bytes).map_err(|e| e.to_string())?;
            let mut chosen = raw.clone();
            if b.pc_used {
                // subset the embedded PC fonts; keep the result only when it renders the same
                let gs = work.join("gs.pdf");
                let mut args: Vec<String> = GS_FLAGS.map(String::from).into();
                args.push("-dBATCH".into());
                args.push(format!("-sOutputFile={}", gs.display()));
                args.push(raw.display().to_string());
                match run_gs(&args, |_| {}) {
                    Ok(()) if size_of(&gs) > 0 && same_render(&raw, &gs) => chosen = gs,
                    Err(GsErr::Cancelled) => return Err("cancelled".to_string()),
                    _ => {}
                }
            }
            let pages = self.sizes.len();
            let words: Vec<(u32, String)> = edits.iter().map(|e| (e.page, e.text.clone())).collect();
            verify(&chosen, pages, &words)?;
            std::fs::copy(&chosen, &tmp).map_err(|e| e.to_string())?;
            let stem = self.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
            let dest = place(&tmp, out_dir, &stem, "_edited", "pdf")?;
            Ok(EditSaveRes { output: dest.display().to_string(), size: size_of(&dest), seconds: start.elapsed().as_secs_f64() })
        })();
        let _ = std::fs::remove_dir_all(&work);
        r
    }
}

// ---------------- commands ----------------

#[tauri::command]
pub async fn edit_open(req: EditOpenReq) -> Result<EditDoc, String> {
    drop(SESSION.lock().unwrap_or_else(|e| e.into_inner()).take()); // replaces any earlier session and frees its job slot
    let guard = crate::job::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let pw = req.password.as_deref().filter(|p| !p.is_empty());
        let (s, d) = Session::open(Path::new(&req.path), pw, Some(guard))?;
        *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
        Ok(d)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn edit_page(page: u32) -> Result<PageModel, String> {
    tauri::async_runtime::spawn_blocking(move || with_session(|s| s.model(page))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn edit_render(page: u32, width_px: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || with_session(|s| s.render(page, width_px))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn edit_apply(req: EditApplyReq) -> Result<EditApplyRes, String> {
    tauri::async_runtime::spawn_blocking(move || with_session(|s| s.apply(&req))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn edit_undo() -> Result<EditHistory, String> {
    tauri::async_runtime::spawn_blocking(move || with_session(|s| s.step(true))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn edit_redo() -> Result<EditHistory, String> {
    tauri::async_runtime::spawn_blocking(move || with_session(|s| s.step(false))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn edit_history() -> Result<EditHistory, String> {
    tauri::async_runtime::spawn_blocking(move || with_session(|s| Ok(s.history(vec![])))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn edit_save(app: tauri::AppHandle, req: EditSaveReq) -> Result<EditSaveRes, String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_session(|s| {
            let dir = if req.out_mode == "folder" { compress::folder_dir(&app) } else { s.path.parent().map(Path::to_path_buf).unwrap_or_default() };
            s.save(&dir)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// No-op when there is no session.
#[tauri::command]
pub fn edit_close() {
    drop(SESSION.lock().unwrap_or_else(|e| e.into_inner()).take());
}

#[cfg(test)]
mod tests;
