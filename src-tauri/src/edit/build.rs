//! Per-page build: paragraph edits (neutralise, retype, push), white-outs (glyph and pixel removal, cover), text boxes and images,
//! all as one rewritten original stream plus one appended stream in list (z) order. Contract: _docs/edit2-contract.md.
use super::content::{self, expand_range, glyph_boxes, glyph_hit, inv, mul, restore_ops, rewrite, shift_open, splice, text_restore, Mod, M, OK};
use super::fonts::{deref, effective_resources, embed_pc, pc_fnt, Fnt, Kind};
use super::layout::{self, box_layout_items, emit, family, line_scales, retype_runs, retype_with, runs_from_words, typeset, Align, BoxItem, Restyle};
use super::whiteout::{self, decode_images, new_image, open_smask, png_xobject, poly, rot_frame, src_of};
use super::*;

pub(super) struct Shared<'a> {
    pub used: &'a HashMap<ObjectId, HashSet<u32>>,
    pub floor_override: Option<f64>,
    pub orig: &'a [u8],
    pub assets: &'a HashMap<String, Arc<Vec<u8>>>,
}

#[derive(Default)]
pub(super) struct Acc {
    pub pcs: PcSet,
    pub pc_objs: HashMap<usize, ObjectId>,
    pub old: Vec<ObjectId>,
    pub asset_objs: HashMap<String, (String, ObjectId)>,
    pub nimg: usize,
    pub prune: bool,
}

struct Plan<'a> {
    item: usize,
    para: &'a Para,
    ed: layout::EditData,
    styles: Vec<layout::Style>,
    words: Vec<Vec<layout::Ch>>,
    lines: Vec<layout::LayLine>,
    dy: f64, // down, page points
    edit: &'a Edit,
    scaled: bool, // line baselines were recomputed (mixed sizes)
}

struct Push {
    lo: usize,
    hi: usize,
    dy: f64,
    moving: HashSet<usize>,
    bottom: f64,
    owner: u32,
}

pub(super) fn fill_of(hex: &str) -> String {
    let h = hex.trim_start_matches('#');
    let v = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).unwrap_or(0) as f64 / 255.0;
    format!("{} {} {} rg", content::fmt((v(0) * 1000.0).round() / 1000.0), content::fmt((v(2) * 1000.0).round() / 1000.0), content::fmt((v(4) * 1000.0).round() / 1000.0))
}

/// Font a paragraph shows as when nothing was changed.
pub(super) fn base_font(p: &Para) -> String {
    if p.pc_font { p.css_font.clone() } else { "orig".into() }
}

/// What a requested style changes relative to the paragraph's original look; the paragraph's layout data adjusted to match.
fn restyle_for(para: &Para, style: Option<&Fmt>) -> (Restyle, layout::EditData) {
    let mut ed = para.ed.clone().expect("editable");
    let mut rs = Restyle::default();
    if let Some(st) = style {
        if st.font != base_font(para) && st.font != "orig" {
            rs.family = Some(st.font.clone());
        }
        if st.size > 0.0 && (st.size - para.size).abs() > 0.01 {
            rs.scale = Some(st.size / para.size);
        }
        if !st.color.eq_ignore_ascii_case(&para.color) {
            rs.fill = Some(fill_of(&st.color));
        }
        if st.bold != para.bold {
            rs.bold = Some(st.bold);
        }
        if st.italic != para.italic {
            rs.italic = Some(st.italic);
        }
        if st.align != para.align {
            ed.align = st.align;
            if st.align == Align::Center {
                ed.centre = (ed.left_x + ed.right_x) / 2.0;
            }
        }
    }
    if let Some(k) = rs.scale {
        ed.pitch *= k;
        ed.size *= k;
        ed.line_base = vec![ed.base0];
    }
    if rs.scale.is_some() || rs.family.is_some() || rs.bold.is_some() || rs.italic.is_some() {
        // widths changed: Word's own breaks no longer hold
        ed.line_words.clear();
        ed.line_r.clear();
    }
    (rs, ed)
}

/// Removes the glyphs of an appended text segment that lie under `rects` (work frame). Returns the new bytes and the count.
fn strip_text(seg: &[u8], fonts: &BTreeMap<String, Fnt>, rmat: M, rects: &[[f64; 4]]) -> (Vec<u8>, usize) {
    let an = content::analyse_with(seg, fonts, &|_| None, rmat);
    let (mut mods, mut n) = (vec![], 0);
    for s in &an.shows {
        let keep: Vec<bool> = glyph_boxes(s).iter().map(|(b, h)| !rects.iter().any(|r| glyph_hit(*b, *h, *r))).collect();
        if keep.iter().all(|k| *k) {
            continue;
        }
        n += keep.iter().zip(&s.glyphs).filter(|(k, g)| !**k && !g.text.as_deref().is_some_and(|t| t.trim().is_empty())).count();
        mods.push(Mod { s: s.start, e: s.end, bytes: rewrite(s, &|i| keep.get(i).copied().unwrap_or(true)), prio: 0 });
    }
    (splice(seg, mods), n)
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn do_name(joined: &[u8], o: &content::Op) -> Option<String> {
    let t = String::from_utf8_lossy(&joined[o.s..o.e]).to_string();
    let n = t.trim().strip_prefix('/')?;
    Some(n.split_whitespace().next()?.to_string())
}

/// Display (y up) -> user space.
fn d2u(pa: &PageAn) -> M {
    mul(inv(pa.to_disp).unwrap_or(content::ID), pa.rinv)
}

/// Display rect [x, y, w, h] (top-left origin) as an AABB in the work frame.
pub(super) fn rect_work(pa: &PageAn, r: [f64; 4]) -> [f64; 4] {
    let (x0, y0, x1, y1) = (r[0], pa.disp[1] - r[1] - r[3], r[0] + r[2], pa.disp[1] - r[1]);
    let w = inv(pa.to_disp).unwrap_or(content::ID);
    let pts = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| (x * w[0] + y * w[2] + w[4], x * w[1] + y * w[3] + w[5]));
    [
        pts.iter().map(|p| p.0).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.1).fold(f64::MAX, f64::min),
        pts.iter().map(|p| p.0).fold(f64::MIN, f64::max),
        pts.iter().map(|p| p.1).fold(f64::MIN, f64::max),
    ]
}

pub(super) fn count_chars(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_page(doc: &mut Document, pa: &PageAn, items: &[&Item], sh: &Shared, acc: &mut Acc) -> Result<PageInfo, BErr> {
    let [_, oy, _, _] = pa.page;
    // ---- paragraph plans
    let mut plans: Vec<Plan> = vec![];
    for (ii, it) in items.iter().enumerate() {
        let Item::Para(e) = it else { continue };
        let para = pa.paras.get(e.para as usize).ok_or_else(|| BErr::Other("no such paragraph".into()))?;
        if para.ed.is_none() {
            return Err(BErr::Other("locked".into()));
        }
        let (styles, words, mut ed) = if let Some(runs) = &e.runs {
            let mut ed = para.ed.clone().unwrap();
            let ro = retype_runs(&ed, para.pc_font, &pa.fonts, sh.used, runs, &mut acc.pcs, e.use_pc).map_err(fail)?;
            if ro.width_changed {
                ed.line_words.clear();
                ed.line_r.clear();
            }
            let al = runs.first().map(|r| r.style.align).unwrap_or(para.align);
            if al != para.align {
                ed.align = al;
                if al == Align::Center {
                    ed.centre = (ed.left_x + ed.right_x) / 2.0;
                }
            }
            (ro.styles, ro.words, ed)
        } else {
            let (rs, ed) = restyle_for(para, e.style.as_ref());
            let use_pc = e.use_pc || rs.family.is_some();
            if para.pc_font && !use_pc {
                return Err(BErr::Missing(e.text.chars().filter(|c| !c.is_whitespace()).collect()));
            }
            let (styles, words) = retype_with(&ed, &pa.fonts, sh.used, &e.text, if use_pc || para.pc_font { Some(&mut acc.pcs) } else { None }, &rs).map_err(fail)?;
            (styles, words, ed)
        };
        let lines = if words.is_empty() { vec![] } else { typeset(&ed, &styles, &words) };
        let old_h = para.ed.as_ref().unwrap().nlines as f64 * para.ed.as_ref().unwrap().pitch;
        let mut new_h = lines.len() as f64 * ed.pitch;
        let mut scaled = false;
        if e.runs.is_some() && !lines.is_empty() {
            // mixed sizes: a line is as high as the pitch times its largest size over the paragraph's own size
            let sc = line_scales(&lines, &words, &styles, para.size);
            if sc.iter().any(|k| (k - 1.0).abs() > 0.005) {
                let mut b = ed.base0;
                let mut v = vec![b];
                for k in &sc[1..] {
                    b -= ed.pitch * k;
                    v.push(b);
                }
                new_h = sc.iter().map(|k| ed.pitch * k).sum();
                ed.line_base = v;
                scaled = true;
            }
        }
        let dy = new_h - old_h;
        plans.push(Plan { item: ii, para, ed, styles, words, lines, dy, edit: e, scaled });
    }
    let neut: HashSet<usize> = plans.iter().flat_map(|p| p.para.ed.as_ref().unwrap().shows.iter().map(|&s| pa.an.shows[s].op)).collect();
    // ---- pushes
    let mut pushes: Vec<Push> = vec![];
    let ops = &pa.an.ops;
    for p in &plans {
        if p.dy.abs() < 1e-6 {
            continue;
        }
        let bottom = p.para.ed.as_ref().unwrap().bottom;
        let moving: Vec<usize> = ops
            .iter()
            .enumerate()
            .filter(|(i, o)| {
                let drawing = match o.k {
                    OK::Show => !pa.an.shows[o.show.unwrap()].blank(),
                    OK::Paint | OK::Do => true,
                    _ => false,
                };
                drawing && !neut.contains(i) && o.bbox.is_some_and(|b| b[3] <= bottom + 1.0 && b[3] > pa.foot_y && b[1] < pa.head_y)
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
        pushes.push(Push { lo, hi, dy: p.dy, moving: mset, bottom, owner: p.para.id });
    }
    let shift_of = |pushes: &[Push], top: f64, id: u32| -> f64 { pushes.iter().filter(|a| a.owner != id && top <= a.bottom + 1.0 && top > pa.foot_y).map(|a| a.dy).sum() };
    // ---- bottom limit (see Edit 1): refuse, or with allow_overlap push nothing
    if plans.iter().any(|p| p.dy > 0.0) {
        let pitch = plans[0].ed.pitch.max(1.0);
        let floor = sh.floor_override.unwrap_or_else(|| (oy + 36.0).max(if pa.foot_y > f64::MIN / 2.0 { pa.foot_y + pitch } else { f64::MIN }));
        let mut lowest = f64::MAX;
        for a in &pushes {
            for &i in &a.moving {
                let total: f64 = pushes.iter().filter(|x| x.moving.contains(&i)).map(|x| x.dy).sum();
                lowest = lowest.min(ops[i].bbox.unwrap()[1] - total);
            }
        }
        for p in &plans {
            if !p.lines.is_empty() {
                let last = if p.scaled { p.ed.line_base[p.lines.len() - 1] } else { p.ed.base0 - p.ed.pitch * (p.lines.len() as f64 - 1.0) };
                lowest = lowest.min(last - shift_of(&pushes, p.ed.top, p.para.id) - 0.22 * p.ed.size);
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
    let mut info = PageInfo { shift: pa.paras.iter().map(|p| shift_of(&pushes, p.top, p.id)).collect(), ..Default::default() };
    for p in &plans {
        info.lines.insert(p.para.id, p.lines.len());
        if !p.lines.is_empty() {
            info.ext.insert(p.para.id, layout::extent(&p.styles, &p.words, &p.lines));
        }
        info.runs.insert(p.para.id, merge_runs(runs_from_words(&p.words, &p.styles, &|st| fmt_of_style(&pa.fonts, Some(&acc.pcs), p.ed.align, st))));
        if p.edit.use_pc || p.para.pc_font || p.styles.iter().any(|s| s.font.starts_with("MjF")) {
            info.pc.insert(p.para.id);
        }
    }
    // ---- mods on the original stream: neutralised paragraphs + wrapped push ranges
    let mut mods: Vec<Mod> = vec![];
    for p in &plans {
        for &sid in &p.para.ed.as_ref().unwrap().shows {
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
        let open = if a.lo > 0 && ops[a.lo - 1].bt { format!("ET\n{open}BT\n{}", text_restore(&pa.an, Some(a.lo - 1))) } else { open };
        mods.push(Mod { s: ops[a.lo].s, e: ops[a.lo].s, bytes: open.into_bytes(), prio: -1000 + w as i32 });
        let close = if ops[a.hi].bt {
            format!("\nET\nQ\nBT\n{}{}\n", text_restore(&pa.an, Some(a.hi)), restore_ops(before, after).unwrap())
        } else {
            format!("\nQ\n{}\n", restore_ops(before, after).unwrap())
        };
        mods.push(Mod { s: ops[a.hi].e, e: ops[a.hi].e, bytes: close.into_bytes(), prio: -2000 - w as i32 });
    }
    // ---- white-outs on the original content (glyphs, image pixels); evaluated where the content is and where a push put it
    let tot_shift = |op: usize| -> f64 { pushes.iter().filter(|x| x.moving.contains(&op)).map(|x| x.dy).sum() };
    let wos: Vec<(usize, &ObjE)> = items.iter().enumerate().filter_map(|(i, it)| if let Item::Obj(o) = it { (o.kind == ObjKind::WhiteOut).then_some((i, o)) } else { None }).collect();
    let wrects: Vec<[f64; 4]> = wos.iter().map(|(_, o)| rect_work(pa, o.rect)).collect();
    let mut res_fonts: Vec<(String, ObjectId)> = vec![];
    let mut res_x: Vec<(String, ObjectId)> = vec![];
    let mut res_rm: Vec<String> = vec![];
    let mut used_pc: BTreeSet<String> = BTreeSet::new();
    let mut partial: Vec<usize> = vec![0; wos.len()];
    let mut partial_img: Vec<bool> = vec![false; wos.len()];
    if !wos.is_empty() {
        for s in &pa.an.shows {
            if neut.contains(&s.op) {
                continue;
            }
            let tot = tot_shift(s.op);
            let removable = pa.fonts.get(&s.font).is_some_and(|f| f.measurable && f.kind != Kind::Type3);
            let boxes = glyph_boxes(s);
            let mut keep = vec![true; boxes.len()];
            for (wi, r) in wrects.iter().enumerate() {
                let rs = [*r, [r[0], r[1] + tot, r[2], r[3] + tot]];
                for (gi, (b, h)) in boxes.iter().enumerate() {
                    if rs.iter().any(|r| glyph_hit(*b, *h, *r)) {
                        if removable {
                            keep[gi] = false;
                        } else if !s.glyphs[gi].text.as_deref().is_some_and(|t| t.trim().is_empty()) {
                            partial[wi] += 1;
                        }
                    }
                }
            }
            if keep.iter().any(|k| !*k) {
                info.removed += keep.iter().zip(&s.glyphs).filter(|(k, g)| !**k && !g.text.as_deref().is_some_and(|t| t.trim().is_empty())).count();
                mods.push(Mod { s: s.start, e: s.end, bytes: rewrite(s, &|i| keep.get(i).copied().unwrap_or(true)), prio: 0 });
            }
        }
        for (wi, r) in wrects.iter().enumerate() {
            for sg in &pa.form_segs {
                let b = [sg.x0, sg.base - 0.22 * sg.size, sg.x1, sg.base + 0.78 * sg.size];
                if glyph_hit(b, true, *r) {
                    partial[wi] += count_chars(&sg.text);
                }
            }
            partial_img[wi] |= pa.form_imgs.iter().any(|b| overlaps(*b, *r));
        }
        // images drawn on the page itself
        struct Hit {
            op: usize,
            name: String,
            ctm: M,
            wis: Vec<usize>,
            rects: Vec<[f64; 4]>,
        }
        let mut hits: Vec<Hit> = vec![];
        let xo = effective_resources(doc, pa.pid).and_then(|r| r.get(b"XObject").ok()).and_then(|o| deref(doc, o).as_dict().ok()).cloned();
        let sub_of = |doc: &Document, name: &str| -> Option<(bool, whiteout::Src)> {
            let Object::Stream(s) = deref(doc, xo.as_ref()?.get(name.as_bytes()).ok()?) else { return None };
            Some((s.dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Image"), src_of(&s.dict)))
        };
        for (i, o) in ops.iter().enumerate() {
            if o.k != OK::Do {
                continue;
            }
            let (Some(b), Some(name)) = (o.bbox, do_name(&pa.joined, o)) else { continue };
            if !sub_of(doc, &name).is_some_and(|x| x.0) {
                continue;
            }
            let tot = tot_shift(i);
            let (mut wis, mut rects) = (vec![], vec![]);
            for (wi, r) in wrects.iter().enumerate() {
                for rr in [*r, [r[0], r[1] + tot, r[2], r[3] + tot]] {
                    if overlaps(b, rr) {
                        if !wis.contains(&wi) {
                            wis.push(wi);
                        }
                        rects.push(rr);
                    }
                }
            }
            if !wis.is_empty() {
                hits.push(Hit { op: i, name, ctm: pa.an.snaps[o.snap as usize].ctm, wis, rects });
            }
        }
        if !hits.is_empty() {
            let u = pa.rinv; // work -> user
            let want: Vec<[f64; 4]> = hits
                .iter()
                .map(|h| {
                    let b = ops[h.op].bbox.unwrap();
                    let pts = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])].map(|(x, y)| (x * u[0] + y * u[2] + u[4], x * u[1] + y * u[3] + u[5]));
                    [pts.iter().map(|p| p.0).fold(f64::MAX, f64::min), pts.iter().map(|p| p.1).fold(f64::MAX, f64::min), pts.iter().map(|p| p.0).fold(f64::MIN, f64::max), pts.iter().map(|p| p.1).fold(f64::MIN, f64::max)]
                })
                .collect();
            let decoded = decode_images(sh.orig, pa.idx, &want).map_err(BErr::Other)?;
            let (mut replaced, mut kept_names) = (HashSet::new(), HashSet::new());
            for (h, pix) in hits.iter().zip(decoded) {
                let Some(mut pix) = pix else {
                    h.wis.iter().for_each(|&w| partial_img[w] = true);
                    kept_names.insert(h.name.clone());
                    continue;
                };
                let (_, src) = sub_of(doc, &h.name).unwrap();
                whiteout::whiten(&mut pix, h.ctm, &h.rects, src.mask);
                let sm = src.smask.map(|id| open_smask(doc, id, h.ctm, &h.rects).unwrap_or(id));
                let id = new_image(doc, &pix, &src, sm).map_err(BErr::Other)?;
                acc.nimg += 1;
                let nm = format!("MjWo{}", acc.nimg);
                res_x.push((nm.clone(), id));
                mods.push(Mod { s: ops[h.op].s, e: ops[h.op].e, bytes: format!("/{nm} Do").into_bytes(), prio: 0 });
                replaced.insert(h.name.clone());
                acc.prune = true;
            }
            for (i, o) in ops.iter().enumerate() {
                if o.k == OK::Do && !hits.iter().any(|h| h.op == i) {
                    if let Some(n) = do_name(&pa.joined, o) {
                        kept_names.insert(n);
                    }
                }
            }
            res_rm.extend(replaced.difference(&kept_names).cloned());
        }
        for (wi, (_, o)) in wos.iter().enumerate() {
            if (partial[wi] > 0 || partial_img[wi]) && !o.cover_only {
                return Err(BErr::Partial(partial[wi]));
            }
        }
    }
    // ---- appended content, in list (z) order
    let rmat = inv(pa.rinv).unwrap_or(content::ID);
    let l2u_page = d2u(pa);
    let mut segs: Vec<(u8, Vec<u8>)> = vec![]; // 0 text, 1 cover/image
    let mut font_map: Option<BTreeMap<String, Fnt>> = None;
    for (ii, it) in items.iter().enumerate() {
        match it {
            Item::Para(_) => {
                let Some(p) = plans.iter().find(|p| p.item == ii) else { continue };
                if !p.lines.is_empty() {
                    let mut b = b"q\n0 Tc 100 Tz 0 Ts 0 Tr\n".to_vec();
                    b.extend(emit(&p.ed, &p.styles, &p.words, &p.lines, shift_of(&pushes, p.ed.top, p.para.id), pa.rinv));
                    b.extend(b"Q\n");
                    segs.push((0, b));
                }
            }
            Item::Obj(o) => match o.kind {
                ObjKind::TextBox => {
                    let (mut bytes, rect, names) = textbox_bytes(pa, o, sh, acc, l2u_page)?;
                    used_pc.extend(names);
                    info.objs.insert(o.id, rect);
                    if !bytes.is_empty() {
                        let mut b = b"q\n0 Tc 100 Tz 0 Ts 0 Tr\n".to_vec();
                        b.append(&mut bytes);
                        b.extend(b"Q\n");
                        segs.push((0, b));
                    }
                }
                ObjKind::WhiteOut => {
                    let r = rect_work(pa, o.rect);
                    if segs.iter().any(|s| s.0 == 0) {
                        if font_map.is_none() {
                            let mut m = pa.fonts.clone();
                            for (i, (_, pc)) in acc.pcs.faces.iter().enumerate() {
                                m.insert(format!("MjF{}", i + 1), pc_fnt(&format!("MjF{}", i + 1), pc));
                            }
                            font_map = Some(m);
                        }
                        for s in segs.iter_mut().filter(|s| s.0 == 0) {
                            let (nb, n) = strip_text(&s.1, font_map.as_ref().unwrap(), rmat, &[r]);
                            s.1 = nb;
                            info.removed += n;
                        }
                    }
                    let (x0, y0, x1, y1) = (o.rect[0], pa.disp[1] - o.rect[1] - o.rect[3], o.rect[0] + o.rect[2], pa.disp[1] - o.rect[1]);
                    segs.push((1, format!("q\n1 g\n{}\nf\nQ\n", poly([x0, y0, x1, y1], l2u_page)).into_bytes()));
                }
                ObjKind::Image => {
                    let png = sh.assets.get(&o.asset).ok_or_else(|| BErr::Other("no such asset".into()))?;
                    if !acc.asset_objs.contains_key(&o.asset) {
                        let n = acc.asset_objs.len() + 1;
                        let id = png_xobject(doc, png).map_err(BErr::Other)?;
                        acc.asset_objs.insert(o.asset.clone(), (format!("MjSg{n}"), id));
                    }
                    let (name, id) = acc.asset_objs[&o.asset].clone();
                    res_x.push((name.clone(), id));
                    let (w, h) = (o.rect[2], o.rect[3]);
                    let (wl, hl) = if o.rot % 180 == 0 { (w, h) } else { (h, w) };
                    let m = mul([wl, 0.0, 0.0, hl, 0.0, -hl], mul(rot_frame(o.rot, o.rect[0], pa.disp[1] - o.rect[1], w, h), l2u_page));
                    let f = content::fmt;
                    segs.push((1, format!("q\n{} {} {} {} {} {} cm\n/{name} Do\nQ\n", f(m[0]), f(m[1]), f(m[2]), f(m[3]), f(m[4]), f(m[5])).into_bytes()));
                }
            },
        }
    }
    // ---- fonts of the PC fallback and new XObjects into the page resources
    for p in &plans {
        used_pc.extend(p.styles.iter().map(|s| s.font.clone()).filter(|n| n.starts_with("MjF")));
    }
    for n in used_pc {
        let fi = n.trim_start_matches("MjF").parse::<usize>().unwrap_or(1) - 1;
        let id = *acc.pc_objs.entry(fi).or_insert_with(|| embed_pc(doc, &acc.pcs.faces[fi].1));
        res_fonts.push((n, id));
    }
    let removed_names = res_rm;
    if !res_fonts.is_empty() || !res_x.is_empty() || !removed_names.is_empty() {
        let mut res = effective_resources(doc, pa.pid).cloned().unwrap_or_default();
        if !res_fonts.is_empty() {
            let mut fd = res.get(b"Font").ok().and_then(|o| deref(doc, o).as_dict().ok()).cloned().unwrap_or_default();
            for (n, id) in res_fonts {
                fd.set(n.as_bytes().to_vec(), Object::Reference(id));
            }
            res.set("Font", Object::Dictionary(fd));
        }
        if !res_x.is_empty() || !removed_names.is_empty() {
            let mut xd = res.get(b"XObject").ok().and_then(|o| deref(doc, o).as_dict().ok()).cloned().unwrap_or_default();
            for (n, id) in res_x {
                xd.set(n.as_bytes().to_vec(), Object::Reference(id));
            }
            for n in removed_names {
                xd.remove(n.as_bytes());
            }
            res.set("XObject", Object::Dictionary(xd));
        }
        if let Ok(Object::Dictionary(pd)) = doc.get_object_mut(pa.pid) {
            pd.set("Resources", Object::Dictionary(res));
        }
    }
    // ---- contents: two new streams; the old ones are dropped at the end when nothing else uses them
    let extra = (-pa.an.depth_min).max(0);
    let mut body = b"q\n".repeat(1 + extra as usize);
    body.extend(splice(&pa.joined, mods));
    body.extend(b"\n");
    for _ in 0..(1 + extra + pa.an.depth_end).max(0) {
        body.extend(b"Q\n");
    }
    let added: Vec<u8> = segs.into_iter().flat_map(|s| s.1).collect();
    acc.old.extend(doc.get_page_contents(pa.pid));
    let mut ids = vec![];
    for bytes in [body, added] {
        let mut s = Stream::new(Dictionary::new(), bytes);
        let _ = s.compress();
        ids.push(Object::Reference(doc.add_object(s)));
    }
    if let Ok(Object::Dictionary(pd)) = doc.get_object_mut(pa.pid) {
        pd.set("Contents", Object::Array(ids));
        // a page thumbnail would still show the original content (and its pixels)
        if pd.remove(b"Thumb").is_some() {
            acc.prune = true;
        }
    }
    Ok(info)
}

/// Typeset bytes of a text box (content space), its display rect with the real typeset size, and the PC font names it uses.
fn textbox_bytes(pa: &PageAn, o: &ObjE, sh: &Shared, acc: &mut Acc, d2u_m: M) -> Result<(Vec<u8>, [f64; 4], Vec<String>), BErr> {
    let runs = o.runs.clone().unwrap_or_else(|| vec![Run { text: o.text.clone(), style: o.style.clone() }]);
    let (w, h) = (o.rect[2], o.rect[3]);
    let wl = if o.rot % 180 == 0 { w } else { h }.max(4.0);
    let none = HashSet::new();
    let dom = pa.dominant();
    let fam_def = dom.as_ref().map(|d| d.base.clone()).unwrap_or_else(|| "Arial".into());
    let mut styles: Vec<layout::Style> = vec![];
    let mut items: Vec<BoxItem> = vec![];
    let (mut missing, mut bad) = (String::new(), String::new());
    for run in &runs {
        let fmt = &run.style;
        let fill = fill_of(&fmt.color);
        let words_chars = || run.text.chars().filter(|c| !c.is_whitespace());
        // the page's own font in the requested bold/italic, when it has every glyph of this run
        let orig = if fmt.font == "orig" {
            dom.as_ref().and_then(|d| {
                let base = pa.fonts.get(&d.font)?;
                pa.fonts.iter().find(|(_, g)| g.encodable && family(&g.base) == family(&base.base) && g.bold == fmt.bold && g.italic == fmt.italic)
            })
        } else {
            None
        };
        let mut use_orig = None;
        if let Some((name, f)) = orig {
            let u = f.id.and_then(|id| sh.used.get(&id)).unwrap_or(&none);
            if words_chars().all(|c| f.code_for(c, u).is_some()) {
                use_orig = Some((name, f, u));
            }
        }
        if fmt.font == "orig" && use_orig.is_none() && !o.use_pc {
            missing.extend(words_chars());
        }
        let (st, widths) = match use_orig {
            Some((name, f, _)) => (layout::make_style(name, f, fmt.size, &fill), None),
            None => {
                let fam = if fmt.font == "orig" { fam_def.clone() } else { fmt.font.clone() };
                let st = acc.pcs.style_fam(&fam, fmt.bold, fmt.italic, fmt.size, &fill).map_err(fail)?;
                let idx = st.font.trim_start_matches("MjF").parse::<usize>().unwrap_or(1).max(1) - 1;
                let wd = acc.pcs.faces[idx].1.widths;
                (st, Some(wd))
            }
        };
        let si = match styles.iter().position(|x| x.font == st.font && (x.size - st.size).abs() < 0.01 && x.fill == st.fill) {
            Some(i) => i,
            None => {
                styles.push(st);
                styles.len() - 1
            }
        };
        for c in run.text.chars() {
            if c == '\n' {
                items.push(BoxItem::Break(fmt.size));
            } else if c.is_whitespace() {
                items.push(BoxItem::Space);
            } else if let Some(wd) = &widths {
                match super::fonts::win_ansi(c).filter(|&k| wd[k as usize] > 0.0) {
                    Some(k) => items.push(BoxItem::Ch(layout::Ch { t: c.to_string(), code: Some(k as u32), w: wd[k as usize], kern: 0.0, gap: 0.0, style: si })),
                    None => bad.push(c),
                }
            } else if let Some((_, f, u)) = use_orig {
                match f.code_for(c, u) {
                    Some(code) => items.push(BoxItem::Ch(layout::Ch { t: c.to_string(), code: Some(code), w: f.w(code), kern: 0.0, gap: 0.0, style: si })),
                    None => missing.push(c),
                }
            }
        }
    }
    if !bad.is_empty() {
        return Err(BErr::Unsupported(bad.chars().collect::<BTreeSet<_>>().into_iter().collect()));
    }
    if !missing.is_empty() {
        return Err(BErr::Missing(missing.chars().collect::<BTreeSet<_>>().into_iter().collect()));
    }
    let default_size = runs.first().map(|r| r.style.size).unwrap_or(11.0);
    let bo = box_layout_items(items, styles, o.style.align, wl, default_size);
    let hl = bo.height.max(1.15 * default_size);
    let frame = rot_frame(o.rot, o.rect[0], pa.disp[1] - o.rect[1], w, h);
    let pts = [(0.0, 0.0), (wl, 0.0), (wl, -hl), (0.0, -hl)].map(|(x, y)| (x * frame[0] + y * frame[2] + frame[4], x * frame[1] + y * frame[3] + frame[5]));
    let (x0, x1) = (pts.iter().map(|p| p.0).fold(f64::MAX, f64::min), pts.iter().map(|p| p.0).fold(f64::MIN, f64::max));
    let (y0, y1) = (pts.iter().map(|p| p.1).fold(f64::MAX, f64::min), pts.iter().map(|p| p.1).fold(f64::MIN, f64::max));
    let rect = [x0, pa.disp[1] - y1, x1 - x0, y1 - y0];
    let bytes = if bo.lines.iter().all(|l| l.words.is_empty()) { vec![] } else { emit(&bo.ed, &bo.styles, &bo.words, &bo.lines, 0.0, mul(frame, d2u_m)) };
    let names = bo.styles.iter().map(|s| s.font.clone()).filter(|n| n.starts_with("MjF")).collect();
    Ok((bytes, rect, names))
}
