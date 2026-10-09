//! Edit PDF tests (contract section "Tests"). Samples come from `_samples/` and are skipped when absent.
//! `editsample_ringkasan.pdf` holds real names: these tests print counts and booleans only, never page text, and the
//! words they insert are made up. Debug PNGs go to $MAJIPDF_EDIT_OUT (default: the system temp dir), never into the repo.
use super::*;
use content::{analyse, OK};
use fonts::{load_fonts, num};
use image::RgbaImage;

fn out_dir() -> PathBuf {
    let d = std::env::var_os("MAJIPDF_EDIT_OUT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("majipdf_edit_test"));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn sample(prefix: &str) -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            n.starts_with(prefix) && n.ends_with(".pdf") && !["_compressed", "_merged", "_organized", "_edited"].iter().any(|x| n.contains(x))
        })
        .collect();
    v.sort();
    v.into_iter().next()
}

fn open(prefix: &str) -> Option<Session> {
    let p = sample(prefix)?;
    Some(Session::open(&p, None, None).unwrap().0)
}

const RK: &str = "editsample_ringkasan";

fn content(bytes: &[u8], idx: u32) -> Vec<u8> {
    let d = Document::load_mem(bytes).unwrap();
    let pid = d.get_pages()[&(idx + 1)];
    d.get_page_content(pid).unwrap()
}

fn made_up(n: usize) -> String {
    // letters the sample fonts certainly have; the words themselves mean nothing
    let a = b"aeinrtsk";
    (1..=n).map(|i| (0..5).rev().map(|d| a[(i / 8usize.pow(d)) % 8] as char).collect::<String>()).collect::<Vec<_>>().join(" ")
}

/// Made-up words spelled only with letters the paragraph itself uses (subset fonts have no others).
fn made_up_in(text: &str, n: usize) -> String {
    let mut a: Vec<char> = text.chars().filter(|c| c.is_ascii_lowercase()).collect();
    a.sort_unstable();
    a.dedup();
    let k = a.len().max(2);
    (1..=n).map(|i| (0..4).rev().map(|d| a[(i / k.pow(d)) % a.len().max(1)]).collect::<String>()).collect::<Vec<_>>().join(" ")
}

fn shot(bytes: &[u8], pa: &PageAn, dpi: f64) -> RgbaImage {
    render_bytes(bytes, 0, (pa.disp[0] * dpi / 72.0).round() as u32).unwrap()
}

/// (pixels differing outside the box, inside the box, ink pixels of `a` inside the box); box in pt, y up.
fn diff(a: &RgbaImage, b: &RgbaImage, pa: &PageAn, bx: [f64; 4], dpi: f64) -> (u64, u64, u64) {
    let k = dpi / 72.0;
    let tt = pa.to_disp;
    let pts = [(bx[0], bx[1]), (bx[2], bx[1]), (bx[2], bx[3]), (bx[0], bx[3])].map(|(x, y)| (x * tt[0] + y * tt[2] + tt[4], x * tt[1] + y * tt[3] + tt[5]));
    let (dx0, dx1) = (pts.iter().map(|p| p.0).fold(f64::MAX, f64::min), pts.iter().map(|p| p.0).fold(f64::MIN, f64::max));
    let (dy0, dy1) = (pts.iter().map(|p| p.1).fold(f64::MAX, f64::min), pts.iter().map(|p| p.1).fold(f64::MIN, f64::max));
    let (x0, x1) = ((dx0 * k) as i64 - 2, (dx1 * k) as i64 + 2);
    let (y0, y1) = (((pa.disp[1] - dy1) * k) as i64 - 2, ((pa.disp[1] - dy0) * k) as i64 + 2);
    let (mut out, mut inn, mut ink) = (0, 0, 0);
    for (x, y, p) in a.enumerate_pixels() {
        let q = b.get_pixel(x, y);
        let inside = (x as i64) >= x0 && (x as i64) <= x1 && (y as i64) >= y0 && (y as i64) <= y1;
        if inside && p.0[..3].iter().any(|&c| c < 200) {
            ink += 1;
        }
        if (0..3).any(|c| (p[c] as i32 - q[c] as i32).abs() > 16) {
            if inside {
                inn += 1;
            } else {
                out += 1;
            }
        }
    }
    (out, inn, ink)
}

fn rk_para(pa: &PageAn, lines: usize) -> &Para {
    pa.paras.iter().find(|p| p.ed.is_some() && p.nlines == lines && !p.pc_font).expect("paragraph")
}

fn text_of(bytes: &[u8], idx: u32) -> String {
    let p = pdf::pdfium().unwrap();
    let d = p.load_pdf_from_byte_vec(bytes.to_vec(), None).unwrap();
    let t = d.pages().get(idx as PdfPageIndex).unwrap().text().unwrap().all();
    t
}

fn reanalyse(bytes: &[u8], idx: u32) -> PageAn {
    let d = Document::load_mem(bytes).unwrap();
    analyse_page(&d, bytes, idx).unwrap()
}

/// How many paragraphs of `before` show up in `after` with the same text, x extent and top moved by the expected amount.
fn moved_ok(before: &PageAn, after: &PageAn, edited: u32, dy_down: f64) -> (usize, usize, usize, usize) {
    let ed = before.paras[edited as usize].ed.as_ref().unwrap();
    let (mut n, mut ok, mut foot_n, mut foot_ok) = (0, 0, 0, 0);
    for p in &before.paras {
        if p.id == edited || p.locked == Some("form") {
            continue;
        }
        let footer = p.top <= before.foot_y;
        let below = p.top <= ed.bottom + 1.0 && !footer;
        let want = p.top - if below { dy_down } else { 0.0 };
        let hit = after.paras.iter().any(|q| norm(&q.text) == norm(&p.text) && (q.top - want).abs() < 0.05 && (q.l - p.l).abs() < 0.01 && (q.r - p.r).abs() < 0.01 && q.nlines == p.nlines);
        if footer {
            foot_n += 1;
            foot_ok += hit as usize;
        } else {
            n += 1;
            ok += hit as usize;
        }
    }
    (n, ok, foot_n, foot_ok)
}

// 1
#[test]
fn op_mapping_counts() {
    for (prefix, pages, strict) in [(RK, 3, true), ("letter_Lampiran", 1, true), ("table_BonBarangUmum", 1, true), ("signed_", 1, true), ("scan_ocr_96", 3, false)] {
        let Some(mut s) = open(prefix) else { continue };
        for pg in 0..pages.min(s.sizes.len() as u32) {
            let pa = s.page(pg).unwrap();
            let locked = pa.paras.iter().filter(|p| p.locked.is_some()).count();
            println!("{prefix} p{}: pdfium text objects {}, show ops {}, paragraphs {} (locked {}), page lock {:?}, scan {}", pg + 1, pa.pdfium_texts, pa.an.shows.len(), pa.paras.len(), locked, pa.lock, pa.scan);
            if strict {
                assert_ne!(pa.lock, Some("structure"), "{prefix} p{} op count != pdfium text objects", pg + 1);
            }
        }
    }
}

// pdfium must keep the page content when it only re-saves (used for locked input)
#[test]
fn pdfium_resave_keeps_streams() {
    let Some(p) = sample(RK) else { return };
    let raw = std::fs::read(&p).unwrap();
    let (re, _) = to_bytes_via_pdfium(&p);
    println!("re-saved by import: {} pages", Document::load_mem(&re).unwrap().get_pages().len());
    let same = (0..3).all(|i| content(&raw, i) == content(&re, i));
    println!("content streams byte-identical after pdfium re-save (3 pages): {same}");
    assert!(same);
}

fn to_bytes_via_pdfium(p: &Path) -> (Vec<u8>, ()) {
    let pd = pdf::pdfium().unwrap();
    let d = pd.load_pdf_from_file(p, None).unwrap();
    let mut out = pd.create_new_pdf().unwrap();
    out.pages_mut().copy_page_range_from_document(&d, 0..=d.pages().len() - 1, 0).unwrap();
    (out.save_to_bytes().unwrap(), ())
}

// 2
#[test]
fn same_text_rewrap_identity() {
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    let used = used_codes(&s.doc);
    let before = shot(&s.orig, &pa, 100.0);
    let (mut total, mut breaks, mut outside0, mut inside_ok) = (0, 0, 0, 0);
    for p in pa.paras.iter().filter(|p| p.ed.is_some() && p.nlines >= 2 && !p.pc_font) {
        let ed = p.ed.as_ref().unwrap();
        total += 1;
        let (st, w) = retype(ed, &pa.fonts, &used, &norm(&p.text), None).ok().unwrap();
        let lay = typeset(ed, &st, &w);
        let b_ok = lay.iter().map(|l| l.words.len()).collect::<Vec<_>>() == ed.line_words;
        breaks += b_ok as usize;
        let e = Edit { page: 0, para: p.id, text: norm(&p.text), use_pc: false, allow_overlap: false };
        let built = s.build(&[e]).ok().expect("build");
        let after = shot(&built.bytes, &pa, 100.0);
        let (out, inn, ink) = diff(&before, &after, &pa, [p.l, p.bot, p.r, p.top], 100.0);
        if std::env::var_os("MAJIPDF_EDIT_CROPS").is_some() {
            let k = 100.0 / 72.0;
            let (x, y, w, h) = (((p.l - 4.0) * k) as u32, ((pa.page[3] - p.top - 4.0) * k) as u32, (((p.r - p.l) + 8.0) * k) as u32, (((p.top - p.bot) + 8.0) * k) as u32);
            image::imageops::crop_imm(&before, x, y, w, h).to_image().save(out_dir().join(format!("same_{}_before.png", p.id))).unwrap();
            image::imageops::crop_imm(&after, x, y, w, h).to_image().save(out_dir().join(format!("same_{}_after.png", p.id))).unwrap();
        }
        outside0 += (out == 0) as usize;
        inside_ok += (inn as f64 <= 0.4 * ink as f64) as usize;
        println!("  para {}: {} lines, align {:?}, breaks equal {b_ok}, diff outside {out}, inside {inn} of ink {ink}", p.id, p.nlines, ed.align);
    }
    println!("same text: {total} multi-line paragraphs; breaks equal {breaks}; outside diff 0: {outside0}; inside <= 40% ink: {inside_ok}");
    assert!(total > 0);
    assert_eq!((breaks, outside0, inside_ok), (total, total, total));
}

// 3 + 4
#[test]
fn grow_and_shrink() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(0.0); // the sample page is already full to its bottom margin: test the mechanics without the limit
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let ed = p.ed.as_ref().unwrap();
    let words = made_up(24);
    let before = shot(&s.orig, &pa, 100.0);
    before.save(out_dir().join("rk_before.png")).unwrap();
    // grow
    let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), words), use_pc: false, allow_overlap: false };
    let b = s.build(&[e]).unwrap_or_else(|e| panic!("grow: {e:?}"));
    let pa2 = reanalyse(&b.bytes, 0);
    let nl = b.info[&0].lines[&p.id];
    println!("grow: lines {} -> {nl}", p.nlines);
    shot(&b.bytes, &pa, 100.0).save(out_dir().join("rk_grow.png")).unwrap();
    assert_eq!(nl, p.nlines + 2);
    let dy = 2.0 * ed.pitch;
    let (n, ok, fn_, fok) = moved_ok(&pa, &pa2, p.id, dy);
    println!("grow: other paragraphs {ok}/{n} moved as expected, footer-zone {fok}/{fn_} unchanged");
    assert_eq!((ok, fok), (n, fn_));
    let t = text_of(&b.bytes, 0).split_whitespace().collect::<Vec<_>>().join(" ");
    let present = words.split(' ').all(|w| t.contains(w));
    println!("grow: inserted words present in extraction: {present}");
    assert!(present);
    assert_eq!(pa2.paras.iter().find(|q| norm(&q.text).ends_with(&words)).map(|q| q.nlines), Some(p.nlines + 2));
    // shrink: drop the last 12 words
    let ws: Vec<&str> = p.text.split_whitespace().collect();
    let short = ws[..ws.len() - 12].join(" ");
    let e = Edit { page: 0, para: p.id, text: short, use_pc: false, allow_overlap: false };
    let b = s.build(&[e]).ok().expect("shrink builds");
    let pa3 = reanalyse(&b.bytes, 0);
    let nl = b.info[&0].lines[&p.id];
    shot(&b.bytes, &pa, 100.0).save(out_dir().join("rk_shrink.png")).unwrap();
    let dy = (nl as f64 - p.nlines as f64) * ed.pitch;
    let (n, ok, fn_, fok) = moved_ok(&pa, &pa3, p.id, dy);
    println!("shrink: lines {} -> {nl}; other paragraphs {ok}/{n} moved up as expected, footer-zone {fok}/{fn_} unchanged", p.nlines);
    assert!(nl < p.nlines);
    assert_eq!((ok, fok), (n, fn_));
}

// 5
#[test]
fn overflow_is_not_applied() {
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(420)), use_pc_font: false, allow_overlap: false }).unwrap();
    let lines = match &r {
        EditApplyRes::Overflow { lines_over } => Some(*lines_over),
        _ => None,
    };
    println!("30-line insertion: overflow lines_over={lines:?}, history {} entries", s.hist.len());
    assert!(lines.is_some());
    assert!(s.hist.is_empty() && s.built().unwrap().bytes == s.orig);
}

// 6
#[test]
fn missing_chars_and_pc_font() {
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let text = format!("{} Q@#~", norm(&p.text));
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: text.clone(), use_pc_font: false, allow_overlap: false }).unwrap();
    println!("missing chars result: {}", matches!(r, EditApplyRes::MissingChars { .. }));
    assert!(matches!(r, EditApplyRes::MissingChars { .. }));
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text, use_pc_font: true, allow_overlap: false }).unwrap();
    assert!(matches!(r, EditApplyRes::Ok { .. }), "pc font apply");
    let out = std::env::temp_dir().join(format!("majipdf_edit_pc_{}", stamp()));
    let res = s.save(&out).unwrap();
    let orig = std::fs::metadata(sample(RK).unwrap()).unwrap().len();
    let raw = s.built().unwrap().bytes.len() as u64;
    println!("pc font: saved size {} (original {orig}, growth {} B, before the gs pass {raw})", res.size, res.size as i64 - orig as i64);
    assert!(res.size < orig + 150_000);
    let t = text_of(&std::fs::read(&res.output).unwrap(), 0).split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(t.contains("Q@#~"));
    render_bytes(&std::fs::read(&res.output).unwrap(), 0, 827).unwrap().save(out_dir().join("rk_pcfont.png")).unwrap();
    let _ = std::fs::remove_dir_all(&out);
}

// 8
#[test]
fn locked_input_saves_unencrypted() {
    let Some(p) = sample("locked_") else { return };
    let Ok(pw) = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples/password.txt")) else { return };
    let (mut s, doc) = Session::open(&p, Some(pw.trim()), None).unwrap();
    println!("locked sample opened: {} pages", doc.pages);
    let mut done = false;
    for pg in 0..doc.pages.min(4) {
        let pa = s.page(pg).unwrap();
        if let Some(para) = pa.paras.iter().find(|q| q.ed.is_some() && !q.pc_font) {
            let r = s.apply(&EditApplyReq { page: pg, para: para.id, text: format!("{} {}", norm(&para.text), made_up(2)), use_pc_font: false, allow_overlap: false }).unwrap();
            println!("locked sample p{} apply ok: {}", pg + 1, matches!(r, EditApplyRes::Ok { .. }));
            if matches!(r, EditApplyRes::Ok { .. }) {
                done = true;
                break;
            }
        }
    }
    if !done {
        println!("locked sample: no editable paragraph found in the first pages; checked open + decrypt only");
        assert!(!Document::load_mem(&s.orig).unwrap().is_encrypted());
        return;
    }
    let out = std::env::temp_dir().join(format!("majipdf_edit_lock_{}", stamp()));
    let res = s.save(&out).unwrap();
    let bytes = std::fs::read(&res.output).unwrap();
    let enc = Document::load_mem(&bytes).unwrap().is_encrypted();
    let opens = matches!(pdf::open(Path::new(&res.output), None), Opened::Ok { .. });
    println!("locked output: encrypted {enc}, opens without password {opens}");
    assert!(!enc && opens);
    let _ = std::fs::remove_dir_all(&out);
}

// 9
#[test]
fn undo_redo_restore_page_bytes() {
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    s.floor_override = Some(0.0);
    let p = rk_para(&pa, 7);
    let orig_page = content(&s.orig, 0);
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(5)), use_pc_font: false, allow_overlap: false }).unwrap();
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    let edited = content(&s.built().unwrap().bytes, 0);
    let h = s.step(true).unwrap();
    let undone = content(&s.built().unwrap().bytes, 0);
    let h2 = s.step(false).unwrap();
    let redone = content(&s.built().unwrap().bytes, 0);
    println!("undo restores page bytes: {}, redo restores edited page: {}, edited differs: {}, history after undo {}/{}", undone == orig_page, redone == edited, edited != orig_page, h.can_undo, h.can_redo);
    assert!(undone == orig_page && redone == edited && edited != orig_page);
    assert!(!h.can_undo && h.can_redo && h2.can_undo && !h2.can_redo);
}

// 10
#[test]
fn save_never_overwrites() {
    let Some(src) = sample(RK) else { return };
    let dir = std::env::temp_dir().join(format!("majipdf_edit_save_{}", stamp()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("doc.pdf");
    std::fs::copy(&src, &copy).unwrap();
    let before = std::fs::read(&copy).unwrap();
    let (mut s, _) = Session::open(&copy, None, None).unwrap();
    assert_eq!(s.save(&dir).err().as_deref(), Some("nothing_to_save"));
    s.floor_override = Some(0.0);
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(3)), use_pc_font: false, allow_overlap: false }).unwrap();
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    let a = s.save(&dir).unwrap();
    let b = s.save(&dir).unwrap();
    let names: Vec<String> = [&a.output, &b.output].iter().map(|o| Path::new(o).file_name().unwrap().to_string_lossy().to_string()).collect();
    println!("save names: {names:?}, original untouched: {}", std::fs::read(&copy).unwrap() == before);
    assert_eq!(names, vec!["doc_edited.pdf", "doc_edited (2).pdf"]);
    assert_eq!(std::fs::read(&copy).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

// other documents: same text must not change anything outside, edits must not break the file
#[test]
fn other_samples_survive_an_edit() {
    for prefix in ["letter_Lampiran", "table_BonBarangUmum", "signed_"] {
        let Some(mut s) = open(prefix) else { continue };
        let pa = s.page(0).unwrap();
        let direct: Vec<&Para> = pa.paras.iter().filter(|p| p.ed.is_some() && !p.pc_font).collect();
        let mut ok = 0;
        for p in direct.iter().take(6) {
            let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(2)), use_pc: false, allow_overlap: true };
            if let Ok(b) = s.build(&[e]) {
                let t = text_of(&b.bytes, 0);
                ok += made_up(2).split(' ').all(|w| t.contains(w)) as usize;
            }
        }
        println!("{prefix}: {} editable paragraphs, {ok} of the first {} edited with the new words found in extraction", direct.len(), direct.len().min(6));
    }
}


fn lines_for(pa: &PageAn, s: &Session, p: &Para, text: &str) -> usize {
    let ed = p.ed.as_ref().unwrap();
    let used = used_codes(&s.doc);
    let (st, w) = retype(ed, &pa.fonts, &used, text, None).ok().unwrap();
    typeset(ed, &st, &w).len()
}

/// Smallest number of made-up words that makes `p` exactly `extra` lines longer.
fn grow_text(pa: &PageAn, s: &Session, p: &Para, extra: usize) -> Option<String> {
    (1..200).map(|n| format!("{} {}", norm(&p.text), made_up(n))).find(|t| lines_for(pa, s, p, t) == p.nlines + extra)
}

// user decision 2026-10-09: limit = footer zone or 0.5 inch from the page edge; allow_overlap applies when it is exceeded
#[test]
fn growth_with_the_real_limit() {
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let one = grow_text(&pa, &s, p, 1).expect("1-line growth text");
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: one, use_pc_font: false, allow_overlap: false }).unwrap();
    println!("1-line growth with the real limit: ok {}", matches!(r, EditApplyRes::Ok { .. }));
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    s.step(true).unwrap();
    let two = grow_text(&pa, &s, p, 2).expect("2-line growth text");
    let req = |t: &str, allow| EditApplyReq { page: 0, para: p.id, text: t.to_string(), use_pc_font: false, allow_overlap: allow };
    let r = s.apply(&req(&two, false)).unwrap();
    let fits = matches!(r, EditApplyRes::Ok { .. });
    if !fits {
        assert!(matches!(r, EditApplyRes::Overflow { .. }));
        assert!(matches!(s.apply(&req(&two, true)).unwrap(), EditApplyRes::Ok { .. }));
    }
    println!("2-line growth: fits {fits}, otherwise allow_overlap applied");
    let big = format!("{} {}", norm(&p.text), made_up(420));
    let r = s.apply(&req(&big, false)).unwrap();
    assert!(matches!(r, EditApplyRes::Overflow { .. }));
    let r = s.apply(&req(&big, true)).unwrap();
    println!("30-line growth: overflow, then allow_overlap ok {}", matches!(r, EditApplyRes::Ok { .. }));
    assert!(matches!(r, EditApplyRes::Ok { .. }));
}

/// No q/Q/cm while a text object is open, BT/ET balanced and never nested.
fn check_streams(bytes: &[u8], page: u32) -> usize {
    let d = Document::load_mem(bytes).unwrap();
    let pid = d.get_pages()[&(page + 1)];
    let fonts = load_fonts(&d, pid);
    let joined = d.get_page_content(pid).unwrap();
    let an = analyse(&joined, &fonts, &|_| None);
    let mut bad = 0;
    let (mut bt, mut et) = (0, 0);
    for (i, o) in an.ops.iter().enumerate() {
        let inside = i > 0 && an.ops[i - 1].bt;
        let is_cm = o.k == OK::State && joined[o.e.saturating_sub(2)..o.e] == *b"cm";
        if matches!(o.k, OK::Q | OK::EndQ) && inside || is_cm && inside {
            bad += 1;
            if std::env::var_os("MAJIPDF_EDIT_DBG").is_some() {
                println!("  q/Q/cm inside BT at op {i} ({:?})", o.k);
            }
        }
        match o.k {
            OK::Bt => {
                bt += 1;
                bad += inside as usize;
            }
            OK::Et => et += 1,
            _ => {}
        }
    }
    if std::env::var_os("MAJIPDF_EDIT_DBG").is_some() {
        println!("  BT {bt} ET {et} depth_end {} bad {bad}", an.depth_end);
    }
    bad + (bt != et) as usize + (an.depth_end != 0) as usize
}

#[test]
fn pushes_never_put_q_inside_a_text_object() {
    let (mut builds, mut bad) = (0, 0);
    for prefix in [RK, "letter_Lampiran", "table_BonBarangUmum"] {
        let Some(mut s) = open(prefix) else { continue };
        s.floor_override = Some(-1e6);
        let pa = s.page(0).unwrap();
        let ids: Vec<u32> = pa.paras.iter().filter(|p| p.ed.is_some() && !p.pc_font).map(|p| p.id).collect();
        for id in ids {
            let p = &pa.paras[id as usize];
            let ws: Vec<&str> = p.text.split_whitespace().collect();
            let texts = [format!("{} {}", norm(&p.text), made_up(2)), format!("{} {}", norm(&p.text), made_up(30)), if ws.len() > 2 { ws[..ws.len() / 2].join(" ") } else { String::new() }];
            for t in texts {
                let e = Edit { page: 0, para: id, text: t, use_pc: false, allow_overlap: true };
                if let Ok(b) = s.build(&[e]) {
                    builds += 1;
                    bad += check_streams(&b.bytes, 0);
                }
            }
        }
    }
    println!("push builds scanned: {builds}, content streams with q/Q/cm inside BT or unbalanced BT/ET/q: {bad}");
    assert!(builds > 0 && bad == 0);
}

// an edited file must be editable again
#[test]
fn saved_output_can_be_reedited() {
    let Some(src) = sample(RK) else { return };
    let dir = std::env::temp_dir().join(format!("majipdf_edit_re_{}", stamp()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("doc.pdf");
    std::fs::copy(&src, &copy).unwrap();
    let (mut s, _) = Session::open(&copy, None, None).unwrap();
    let pa = s.page(0).unwrap();
    let (a, b) = (rk_para(&pa, 7).id, pa.paras.iter().find(|p| p.ed.is_some() && p.nlines == 6).unwrap().id);
    let at = |s: &mut Session, id: u32, t: String| s.apply(&EditApplyReq { page: 0, para: id, text: t, use_pc_font: false, allow_overlap: true }).unwrap();
    let ta = format!("{} {}", norm(&pa.paras[a as usize].text), made_up(3));
    assert!(matches!(at(&mut s, a, ta), EditApplyRes::Ok { .. }));
    let out1 = s.save(&dir).unwrap().output;
    let (mut s2, _) = Session::open(Path::new(&out1), None, None).unwrap();
    let pa2 = s2.page(0).unwrap();
    println!("reopened output: page lock {:?}, pdfium text objects {}, show ops {}, locked paragraphs {}", pa2.lock, pa2.pdfium_texts, pa2.an.shows.len(), pa2.paras.iter().filter(|p| p.locked.is_some()).count());
    assert_eq!(pa2.lock, None);
    let q = pa2.paras.iter().find(|p| p.ed.is_some() && p.nlines >= 5 && norm(&p.text) != norm(&pa.paras[a as usize].text) && !p.text.contains(&made_up(3))).unwrap();
    let tq = format!("{} {}", norm(&q.text), made_up(2));
    let qid = q.id;
    assert!(matches!(at(&mut s2, qid, tq), EditApplyRes::Ok { .. }));
    let out2 = s2.save(&dir).unwrap();
    println!("second save ok: {} bytes", out2.size);
    let _ = b;
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn centred_heading_stays_one_line() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let cands: Vec<&Para> = pa.paras.iter().filter(|p| p.ed.is_some() && p.nlines == 1 && p.align == Align::Center).collect();
    println!("centred single-line paragraphs on ringkasan p1: {}", cands.len());
    assert!(!cands.is_empty());
    for p in cands {
        let c0 = (p.l + p.r) / 2.0;
        let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(2)), use_pc: false, allow_overlap: false };
        let b = s.build(&[e]).ok().unwrap();
        let nl = b.info[&0].lines[&p.id];
        let pa2 = reanalyse(&b.bytes, 0);
        let Some(q) = pa2.paras.iter().find(|q| q.text.contains(&made_up(2))) else { println!("  wrapped over {nl} lines (wider than the body)"); continue };
        let c1 = (q.l + q.r) / 2.0;
        println!("  centred heading +2 words: {nl} line(s), align {:?}, centre moved {:.2} pt", q.align, (c1 - c0).abs());
        if p.r - p.l < 0.6 * 433.0 {
            assert_eq!(nl, 1);
        }
        if nl == 1 {
            assert!((c1 - c0).abs() < 2.0);
        }
    }
}


// round 3: the paragraph box follows the new layout
#[test]
fn boxes_follow_the_new_layout() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let before = s.model(0).unwrap();
    let head = pa.paras.iter().find(|p| p.ed.is_some() && p.nlines == 1 && p.align == Align::Center).unwrap();
    let body = rk_para(&pa, 7);
    let (hid, bid) = (head.id, body.id);
    let ap = |s: &mut Session, id: u32, t: String| s.apply(&EditApplyReq { page: 0, para: id, text: t, use_pc_font: false, allow_overlap: false }).unwrap();
    assert!(matches!(ap(&mut s, hid, format!("{} {}", norm(&head.text), made_up(3))), EditApplyRes::Ok { .. }));
    let text_b = grow_text(&pa, &s, body, 2).unwrap();
    assert!(matches!(ap(&mut s, bid, text_b), EditApplyRes::Ok { .. }));
    let m = s.model(0).unwrap();
    let built = s.built().unwrap();
    let pa2 = reanalyse(&built.bytes, 0);
    // the edited boxes match what is really on the page now, and untouched paragraphs below moved with the push
    let (mut edited_ok, mut moved_ok_n, mut moved_n) = (0, 0, 0);
    for p in &m.paras {
        let src = &pa.paras[p.id as usize];
        if p.edited {
            let q = pa2.paras.iter().find(|q| norm(&q.text) == norm(&p.text)).unwrap();
            let want = [q.l, pa.page[3] - q.top, q.r - q.l, q.top - q.bot];
            edited_ok += (0..4).all(|i| (p.bbox[i] - want[i]).abs() < 1.5) as usize;
            println!("  edited paragraph {}: model box w {:.1} h {:.1} vs page w {:.1} h {:.1}", p.id, p.bbox[2], p.bbox[3], want[2], want[3]);
        } else if src.ed.is_some() {
            moved_n += 1;
            if let Some(q) = pa2.paras.iter().find(|q| norm(&q.text) == norm(&src.text)) {
                moved_ok_n += ((p.bbox[1] - (pa.page[3] - q.top)).abs() < 1.5) as usize;
            }
        }
    }
    println!("boxes: edited paragraphs matching the page {edited_ok}/2; untouched paragraphs matching the page {moved_ok_n}/{moved_n}");
    assert_eq!(edited_ok, 2);
    assert_eq!(moved_ok_n, moved_n);
    let shifted = m.paras.iter().zip(before.paras.iter()).filter(|(a, b)| !a.edited && (a.bbox[1] - b.bbox[1]).abs() > 1.0).count();
    println!("paragraphs whose box moved down by the push: {shifted}");
    assert!(shifted > 0);
    // after undo the boxes are the old ones again
    s.step(true).unwrap();
    s.step(true).unwrap();
    let m0 = s.model(0).unwrap();
    assert!(m0.paras.iter().zip(before.paras.iter()).all(|(a, b)| (0..4).all(|i| (a.bbox[i] - b.bbox[i]).abs() < 1e-6)));
}

fn r3_dir() -> PathBuf {
    let d = out_dir().join("r3");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Same-text rewrap, then a grow: size, pixels and extraction on a (possibly rotated) page.
fn rotation_checks(s: &mut Session, tag: &str) -> (usize, usize) {
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let sz = s.sizes[0];
    println!("{tag}: displayed size {:.1}x{:.1} (pdfium {:.1}x{:.1}), work-frame turn {}, paragraphs {} (editable {}), page lock {:?}", pa.disp[0], pa.disp[1], sz.0, sz.1, pa.q, pa.paras.len(), pa.paras.iter().filter(|p| p.ed.is_some()).count(), pa.lock);
    assert!((pa.disp[0] - sz.0 as f64).abs() < 0.6 && (pa.disp[1] - sz.1 as f64).abs() < 0.6, "display size equals pdfium's");
    let used = used_codes(&s.doc);
    let before = shot(&s.orig, &pa, 100.0);
    before.save(r3_dir().join(format!("{tag}_before.png"))).unwrap();
    let (mut n, mut ok) = (0, 0);
    let multi = pa.paras.iter().any(|p| p.ed.is_some() && p.nlines >= 2 && !p.pc_font);
    for p in pa.paras.iter().filter(|p| p.ed.is_some() && (p.nlines >= 2 || !multi) && !p.pc_font).take(3) {
        let ed = p.ed.as_ref().unwrap();
        let (st, w) = retype(ed, &pa.fonts, &used, &norm(&p.text), None).ok().unwrap();
        let same_breaks = typeset(ed, &st, &w).iter().map(|l| l.words.len()).collect::<Vec<_>>() == ed.line_words;
        let e = Edit { page: 0, para: p.id, text: norm(&p.text), use_pc: false, allow_overlap: false };
        let b = s.build(&[e]).ok().expect("build");
        let after = shot(&b.bytes, &pa, 100.0);
        let (out, inn, ink) = diff(&before, &after, &pa, [p.l, p.bot, p.r, p.top], 100.0);
        if n == 0 {
            after.save(r3_dir().join(format!("{tag}_same.png"))).unwrap();
        }
        n += 1;
        ok += (same_breaks && out == 0 && inn as f64 <= 0.4 * ink as f64) as usize;
        println!("  same text, para {} ({} lines): breaks equal {same_breaks}, diff outside {out}, inside {inn} of ink {ink}", p.id, p.nlines);
    }
    if let Some(p) = pa.paras.iter().find(|p| p.ed.is_some() && (p.nlines >= 2 || !multi) && !p.pc_font) {
        let words = made_up_in(&p.text, 12);
        let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), words), use_pc: false, allow_overlap: true };
        let b = s.build(&[e]).unwrap_or_else(|e| panic!("grow: {e:?}"));
        let t = text_of(&b.bytes, 0).split_whitespace().collect::<Vec<_>>().join(" ");
        let present = words.split(' ').all(|w| t.contains(w));
        shot(&b.bytes, &pa, 100.0).save(r3_dir().join(format!("{tag}_grow.png"))).unwrap();
        let pa2 = reanalyse(&b.bytes, 0);
        let q = pa2.paras.iter().find(|q| norm(&q.text).ends_with(&words));
        println!("  grow: lines {} -> {}, words extracted {present}, re-analysis finds the paragraph {}", p.nlines, b.info[&0].lines[&p.id], q.is_some());
        n += 1;
        ok += (present && q.is_some()) as usize;
    }
    (n, ok)
}

#[test]
fn rotated_pages_are_editable() {
    let Some(src) = sample(RK) else { return };
    let dir = std::env::temp_dir().join(format!("majipdf_edit_rot_{}", stamp()));
    std::fs::create_dir_all(&dir).unwrap();
    for rot in [90i64, 180, 270] {
        let mut d = Document::load(&src).unwrap();
        let pid = d.get_pages()[&1];
        d.get_object_mut(pid).unwrap().as_dict_mut().unwrap().set("Rotate", rot);
        let f = dir.join(format!("rot{rot}.pdf"));
        d.save(&f).unwrap();
        let (mut s, _) = Session::open(&f, None, None).unwrap();
        let (n, ok) = rotation_checks(&mut s, &format!("rot{rot}"));
        assert!(n >= 2 && ok == n, "rotation {rot}: {ok}/{n}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn locked_sample_rotated_text_is_editable() {
    let Some(p) = sample("locked_") else { return };
    let Ok(pw) = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples/password.txt")) else { return };
    let (mut s, _) = Session::open(&p, Some(pw.trim()), None).unwrap();
    let rot = inherited(&s.doc, s.doc.get_pages()[&1], b"Rotate").map(|o| num(o) as i64);
    println!("locked sample: /Rotate {rot:?}");
    let (n, ok) = rotation_checks(&mut s, "locked");
    assert!(n >= 1 && ok == n, "locked sample {ok}/{n}");
    let pa = s.page(0).unwrap();
    let para = pa.paras.iter().find(|p| p.ed.is_some() && !p.pc_font).expect("an editable paragraph");
    let words = made_up_in(&para.text, 4);
    let r = s.apply(&EditApplyReq { page: 0, para: para.id, text: format!("{} {}", norm(&para.text), words), use_pc_font: false, allow_overlap: true }).unwrap();
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    let out = std::env::temp_dir().join(format!("majipdf_edit_lockrot_{}", stamp()));
    let res = s.save(&out).unwrap();
    let args: Vec<String> = ["-dNOPAUSE", "-dBATCH", "-sDEVICE=nullpage", &res.output].map(String::from).into();
    let gs = run_gs(&args, |_| {}).is_ok();
    let t = text_of(&std::fs::read(&res.output).unwrap(), 0).split_whitespace().collect::<Vec<_>>().join(" ");
    println!("locked sample grow+save: gs opens clean {gs}, new words extracted {}", words.split(' ').all(|w| t.contains(w)));
    assert!(gs && words.split(' ').all(|w| t.contains(w)));
    let _ = std::fs::remove_dir_all(&out);
}

