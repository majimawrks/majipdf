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

/// run_gs keeps one global child slot: tests that call gs take turns.
static GS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn gs_guard() -> std::sync::MutexGuard<'static, ()> {
    GS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
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
        let e = Edit { page: 0, para: p.id, text: norm(&p.text), use_pc: false, allow_overlap: false, style: None, runs: None };
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
    let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), words), use_pc: false, allow_overlap: false, style: None, runs: None };
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
    let e = Edit { page: 0, para: p.id, text: short, use_pc: false, allow_overlap: false, style: None, runs: None };
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
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(420)), use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
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
    let _g = gs_guard();
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let text = format!("{} Q@#~", norm(&p.text));
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: text.clone(), use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
    println!("missing chars result: {}", matches!(r, EditApplyRes::MissingChars { .. }));
    assert!(matches!(r, EditApplyRes::MissingChars { .. }));
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text, use_pc_font: true, allow_overlap: false, style: None, runs: None }).unwrap();
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
    let _g = gs_guard();
    let Some(p) = sample("locked_") else { return };
    let Ok(pw) = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples/password.txt")) else { return };
    let (mut s, doc) = Session::open(&p, Some(pw.trim()), None).unwrap();
    println!("locked sample opened: {} pages", doc.pages);
    let mut done = false;
    for pg in 0..doc.pages.min(4) {
        let pa = s.page(pg).unwrap();
        if let Some(para) = pa.paras.iter().find(|q| q.ed.is_some() && !q.pc_font) {
            let r = s.apply(&EditApplyReq { page: pg, para: para.id, text: format!("{} {}", norm(&para.text), made_up(2)), use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
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
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(5)), use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
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
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(3)), use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
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
            let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(2)), use_pc: false, allow_overlap: true, style: None, runs: None };
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
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: one, use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
    println!("1-line growth with the real limit: ok {}", matches!(r, EditApplyRes::Ok { .. }));
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    s.step(true).unwrap();
    let two = grow_text(&pa, &s, p, 2).expect("2-line growth text");
    let req = |t: &str, allow| EditApplyReq { page: 0, para: p.id, text: t.to_string(), use_pc_font: false, allow_overlap: allow, style: None, runs: None };
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
    let (mut builds, mut bad, mut opened) = (0, 0, 0);
    for prefix in [RK, "letter_Lampiran", "table_BonBarangUmum"] {
        let Some(mut s) = open(prefix) else { continue };
        opened += 1;
        s.floor_override = Some(-1e6);
        let pa = s.page(0).unwrap();
        let ids: Vec<u32> = pa.paras.iter().filter(|p| p.ed.is_some() && !p.pc_font).map(|p| p.id).collect();
        for id in ids {
            let p = &pa.paras[id as usize];
            let ws: Vec<&str> = p.text.split_whitespace().collect();
            let texts = [format!("{} {}", norm(&p.text), made_up(2)), format!("{} {}", norm(&p.text), made_up(30)), if ws.len() > 2 { ws[..ws.len() / 2].join(" ") } else { String::new() }];
            for t in texts {
                let e = Edit { page: 0, para: id, text: t, use_pc: false, allow_overlap: true, style: None, runs: None };
                if let Ok(b) = s.build(&[e]) {
                    builds += 1;
                    bad += check_streams(&b.bytes, 0);
                }
            }
        }
    }
    println!("push builds scanned: {builds}, content streams with q/Q/cm inside BT or unbalanced BT/ET/q: {bad}");
    // Samples are local-only (not in CI): nothing opened means nothing to check.
    assert!((opened == 0 || builds > 0) && bad == 0);
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
    let at = |s: &mut Session, id: u32, t: String| s.apply(&EditApplyReq { page: 0, para: id, text: t, use_pc_font: false, allow_overlap: true, style: None, runs: None }).unwrap();
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
        let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), made_up(2)), use_pc: false, allow_overlap: false, style: None, runs: None };
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
    let ap = |s: &mut Session, id: u32, t: String| s.apply(&EditApplyReq { page: 0, para: id, text: t, use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
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
        let e = Edit { page: 0, para: p.id, text: norm(&p.text), use_pc: false, allow_overlap: false, style: None, runs: None };
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
        let e = Edit { page: 0, para: p.id, text: format!("{} {}", norm(&p.text), words), use_pc: false, allow_overlap: true, style: None, runs: None };
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
    let _g = gs_guard();
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
    let r = s.apply(&EditApplyReq { page: 0, para: para.id, text: format!("{} {}", norm(&para.text), words), use_pc_font: false, allow_overlap: true, style: None, runs: None }).unwrap();
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


// ---------------- Edit 2 ----------------

fn e2_dir() -> PathBuf {
    let d = out_dir().join("e2");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn req(kind: &str, page: u32, id: Option<u32>, rect: [f64; 4]) -> ObjReq {
    ObjReq { kind: kind.into(), page, id, rect: Some(rect), text: None, style: None, rot: None, asset: None, cover_only: None, runs: None, use_pc_font: None }
}

fn new_id(r: ObjRes) -> u32 {
    match r {
        ObjRes::Ok { id, .. } => id,
        o => panic!("expected ok, got {o:?}"),
    }
}

/// Glyph centres of a page that lie inside a display rect (top-left origin), by pdfium on the output bytes.
fn centres_inside(s: &mut Session, bytes: &[u8], page: u32, r: [f64; 4]) -> usize {
    let pa = s.page(page).unwrap();
    let m = content::mul(content::inv(pa.rinv).unwrap(), pa.to_disp);
    let p = pdf::pdfium().unwrap();
    let d = p.load_pdf_from_byte_vec(bytes.to_vec(), None).unwrap();
    let pg = d.pages().get(page as PdfPageIndex).unwrap();
    let t = pg.text().unwrap();
    let mut n = 0;
    for ch in t.chars().iter() {
        if ch.unicode_char().is_none_or(|c| c.is_whitespace()) {
            continue;
        }
        let Ok(b) = ch.tight_bounds() else { continue };
        let (cx, cy) = (((b.left().value + b.right().value) / 2.0) as f64, ((b.bottom().value + b.top().value) / 2.0) as f64);
        let (x, y) = (cx * m[0] + cy * m[2] + m[4], pa.disp[1] - (cx * m[1] + cy * m[3] + m[5]));
        if x > r[0] && x < r[0] + r[2] && y > r[1] && y < r[1] + r[3] {
            n += 1;
        }
    }
    n
}

/// Display rect (top-left origin) covering glyphs `a..b` of the longest show op of a paragraph.
fn glyph_rect(pa: &PageAn, p: &Para, a: usize, b: usize) -> [f64; 4] {
    let ed = p.ed.as_ref().unwrap();
    let s = ed.shows.iter().map(|&i| &pa.an.shows[i]).max_by_key(|s| s.glyphs.len()).unwrap();
    let bx = content::glyph_boxes(s);
    let (x0, x1) = (bx[a].0[0], bx[b].0[2]);
    let (y0, y1) = (bx[a].0[1], bx[a].0[3]);
    let t = pa.to_disp;
    let pts = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| (x * t[0] + y * t[2] + t[4], x * t[1] + y * t[3] + t[5]));
    let (dx0, dx1) = (pts.iter().map(|p| p.0).fold(f64::MAX, f64::min), pts.iter().map(|p| p.0).fold(f64::MIN, f64::max));
    let (dy0, dy1) = (pts.iter().map(|p| p.1).fold(f64::MAX, f64::min), pts.iter().map(|p| p.1).fold(f64::MIN, f64::max));
    [dx0, pa.disp[1] - dy1, dx1 - dx0, dy1 - dy0]
}

/// (pixels differing by more than `tol` outside / inside the display rect `r` inflated by `grow` px), at `dpi`.
fn diff_rect(a: &RgbaImage, b: &RgbaImage, r: [f64; 4], dpi: f64, grow: i64, tol: i32) -> (u64, u64) {
    let k = dpi / 72.0;
    let (x0, y0, x1, y1) = ((r[0] * k) as i64 - grow, (r[1] * k) as i64 - grow, ((r[0] + r[2]) * k) as i64 + grow, ((r[1] + r[3]) * k) as i64 + grow);
    let (mut out, mut inn) = (0, 0);
    for (x, y, p) in a.enumerate_pixels() {
        let q = b.get_pixel(x, y);
        if (0..3).any(|c| (p[c] as i32 - q[c] as i32).abs() > tol) {
            if (x as i64) >= x0 && (x as i64) <= x1 && (y as i64) >= y0 && (y as i64) <= y1 {
                inn += 1;
            } else {
                out += 1;
            }
        }
    }
    (out, inn)
}

fn gs_clean(bytes: &[u8]) -> bool {
    let f = std::env::temp_dir().join(format!("majipdf_e2_{}.pdf", stamp()));
    std::fs::write(&f, bytes).unwrap();
    let args: Vec<String> = ["-dNOPAUSE", "-dBATCH", "-sDEVICE=nullpage", &f.display().to_string()].map(String::from).into();
    let ok = run_gs(&args, |_| {}).is_ok();
    let _ = std::fs::remove_file(&f);
    ok
}

// E1: white-out over part of a line on original text
#[test]
fn e2_whiteout_original_text() {
    let _g = gs_guard();
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let r = glyph_rect(&pa, p, 4, 12);
    let before = shot(&s.orig, &pa, 100.0);
    let res = s.apply_obj(&req("whiteout", 0, None, r)).unwrap();
    assert!(matches!(res, ObjRes::Ok { .. }));
    let b = s.built().unwrap();
    let removed = b.info[&0].removed;
    let inside = centres_inside(&mut s, &b.bytes, 0, r);
    let after = shot(&b.bytes, &pa, 100.0);
    let (out, inn) = diff_rect(&before, &after, r, 100.0, 2, 16);
    before.save(e2_dir().join("wo_text_before.png")).unwrap();
    after.save(e2_dir().join("wo_text_after.png")).unwrap();
    let gs = gs_clean(&b.bytes);
    println!("white-out text: glyphs removed {removed}, glyph centres inside the rect {inside}, px differing outside {out}, inside {inn}, gs clean {gs}");
    assert!(removed > 0 && inside == 0 && gs && inn > 0);
    assert!(out <= 20, "pixels differing outside the rect: {out}");
    // save passes the output verification
    let dir = std::env::temp_dir().join(format!("majipdf_e2_wo_{}", stamp()));
    s.save(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

// E2: white-out over an edited (pushed) paragraph removes our appended glyphs too
#[test]
fn e2_whiteout_edited_paragraph() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(0.0);
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let text = format!("{} {}", norm(&p.text), made_up(24));
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text, use_pc_font: false, allow_overlap: false, style: None, runs: None }).unwrap();
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    let m = s.model(0).unwrap();
    let bb = m.paras[p.id as usize].bbox;
    let rect = [bb[0] + bb[2] * 0.3, bb[1] + bb[3] * 0.5, bb[2] * 0.3, bb[3] * 0.25];
    let before = s.built().unwrap().bytes.clone();
    let inside0 = centres_inside(&mut s, &before, 0, rect);
    let res = s.apply_obj(&req("whiteout", 0, None, rect)).unwrap();
    assert!(matches!(res, ObjRes::Ok { .. }));
    let b = s.built().unwrap();
    let inside = centres_inside(&mut s, &b.bytes, 0, rect);
    shot(&b.bytes, &pa, 100.0).save(e2_dir().join("wo_edited_after.png")).unwrap();
    println!("white-out over an edited paragraph: glyph centres in the rect before {inside0}, after {inside}, glyphs removed {}", b.info[&0].removed);
    assert!(inside0 > 0 && inside == 0 && b.info[&0].removed > 0);
}

// E3: white-out on a scanned page removes image pixels
#[test]
fn e2_whiteout_scan() {
    let _g = gs_guard();
    let Some(p) = sample("scanned_bsc.pdf") else { return };
    let (mut s, _) = Session::open(&p, None, None).unwrap();
    let pages = s.sizes.len();
    let pa = s.page(0).unwrap();
    let rect = [pa.disp[0] * 0.25, pa.disp[1] * 0.35, pa.disp[0] * 0.3, pa.disp[1] * 0.12];
    let before = shot(&s.orig, &pa, 100.0);
    let res = s.apply_obj(&req("whiteout", 0, None, rect)).unwrap();
    println!("scan white-out result ok: {}", matches!(res, ObjRes::Ok { .. }));
    assert!(matches!(res, ObjRes::Ok { .. }));
    let b = s.built().unwrap();
    let after = shot(&b.bytes, &pa, 100.0);
    before.save(e2_dir().join("wo_scan_before.png")).unwrap();
    after.save(e2_dir().join("wo_scan_after.png")).unwrap();
    let (k, mut dark_before, mut dark_after, mut n) = (100.0 / 72.0, 0, 0, 0);
    for y in (rect[1] * k) as u32 + 2..((rect[1] + rect[3]) * k) as u32 - 2 {
        for x in (rect[0] * k) as u32 + 2..((rect[0] + rect[2]) * k) as u32 - 2 {
            n += 1;
            dark_before += (before.get_pixel(x, y)[0] < 200) as u32;
            dark_after += (after.get_pixel(x, y)[0] < 200) as u32;
        }
    }
    let (out, _) = diff_rect(&before, &after, rect, 100.0, 2, 40);
    let grown = b.bytes.len() as i64 - s.orig.len() as i64;
    // the page's image streams, before
    let d = Document::load_mem(&s.orig).unwrap();
    let img_bytes: usize = d.get_page_images(d.get_pages()[&1]).unwrap().iter().map(|i| i.content.len()).sum();
    println!("scan: non-white px in the rect before {dark_before} / after {dark_after} of {n}; px changed >40 outside {out}; file grew {grown} B (page-1 image streams {img_bytes} B); pages {pages}");
    assert!(dark_before > 0 && dark_after == 0 && out < 200);
    assert!(grown < 2 * img_bytes as i64);
    // other pages untouched, shared images on them unchanged
    let d2 = Document::load_mem(&b.bytes).unwrap();
    let mut same = 0;
    for pg in 2..=pages as u32 {
        let (a, c) = (d.get_pages()[&pg], d2.get_pages()[&pg]);
        let ia: Vec<Vec<u8>> = d.get_page_images(a).unwrap().iter().map(|i| i.content.to_vec()).collect();
        let ib: Vec<Vec<u8>> = d2.get_page_images(c).unwrap().iter().map(|i| i.content.to_vec()).collect();
        same += (ia == ib && d.get_page_content(a).unwrap() == d2.get_page_content(c).unwrap()) as u32;
    }
    println!("other pages with identical content and image streams: {same} of {}", pages.saturating_sub(1));
    assert_eq!(same as usize, pages.saturating_sub(1));
    assert!(gs_clean(&b.bytes));
}

// E4: text box
#[test]
fn e2_textbox() {
    let _g = gs_guard();
    let Some(mut s) = open(RK) else { return };
    let words = "zorbal quendik vimrath tuvok pelmar sunkra deliq woret";
    let style = Fmt { font: "Arial".into(), size: 12.0, color: "#c00000".into(), bold: false, italic: false, align: Align::Left };
    let mut r = req("textbox", 0, None, [72.0, 60.0, 120.0, 14.0]);
    r.text = Some(words.into());
    r.style = Some(style.clone());
    let id = new_id(s.apply_obj(&r).unwrap());
    let m = s.model(0).unwrap();
    let o = m.objs.iter().find(|o| o.id == id).unwrap();
    let nl = (o.rect[3] / (1.15 * 12.0)).round();
    let pitch_ok = (o.rect[3] - nl * 1.15 * 12.0).abs() < 1e-6 && nl >= 2.0;
    let b = s.built().unwrap();
    let t = text_of(&b.bytes, 0).split_whitespace().collect::<Vec<_>>().join(" ");
    let present = words.split(' ').all(|w| t.contains(w));
    shot(&b.bytes, &s.page(0).unwrap(), 100.0).save(e2_dir().join("textbox.png")).unwrap();
    println!("text box: {nl} lines, height = lines x pitch {pitch_ok}, words in extraction {present}, pc_used {}", b.pc_used);
    assert!(pitch_ok && present && b.pc_used);
    // hard line breaks and an empty line
    let mut r2 = req("textbox", 0, Some(id), [72.0, 60.0, 300.0, 14.0]);
    r2.text = Some("kalmir\n\nzentova".into());
    let id2 = new_id(s.apply_obj(&r2).unwrap());
    let m = s.model(0).unwrap();
    let h = m.objs.iter().find(|o| o.id == id2).unwrap().rect[3];
    println!("text box with an empty line: height {:.2} = 3 lines {}", h, (h - 3.0 * 1.15 * 12.0).abs() < 1e-6);
    assert_eq!(id, id2);
    assert!((h - 3.0 * 1.15 * 12.0).abs() < 1e-6);
    // save: gs subset pass keeps the growth small
    let dir = std::env::temp_dir().join(format!("majipdf_e2_tb_{}", stamp()));
    let res = s.save(&dir).unwrap();
    let orig = std::fs::metadata(sample(RK).unwrap()).unwrap().len();
    println!("text box save: size {} (original {orig}, growth {} B)", res.size, res.size as i64 - orig as i64);
    assert!(res.size < orig + 150_000);
    let _ = std::fs::remove_dir_all(&dir);
    // characters outside WinAnsi
    let mut r3 = req("textbox", 0, None, [72.0, 200.0, 100.0, 14.0]);
    r3.text = Some("ab\u{3a9}".into());
    r3.style = Some(style);
    assert!(matches!(s.apply_obj(&r3).unwrap(), ObjRes::UnsupportedChars { .. }));
}

fn alpha_png() -> Vec<u8> {
    let mut img = RgbaImage::new(60, 40);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let corner = x < 6 && y < 6;
        *p = image::Rgba([220, 20, 20, if corner { 0 } else { 255 }]);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

// E5 + E6 (library): image with alpha, shared XObject
#[test]
fn e2_image_and_library() {
    let dir = std::env::temp_dir().join(format!("majipdf_e2_lib_{}", stamp()));
    let lib = sig::Lib { dir: dir.clone() };
    // library: a JPG with dark strokes on a light grey background
    let mut jpg = image::RgbImage::from_pixel(200, 100, image::Rgb([205, 205, 205]));
    for x in 40..160 {
        for y in 45..50 {
            jpg.put_pixel(x, y, image::Rgb([30, 30, 90]));
        }
    }
    let jp = dir.join("src.jpg");
    std::fs::create_dir_all(&dir).unwrap();
    jpg.save(&jp).unwrap();
    let it = lib.import(&jp, "signature").unwrap();
    let png = lib.png(&it.id).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    let corner_clear = img.get_pixel(0, 0)[3] == 0;
    let ink_kept = img.pixels().any(|p| p[3] > 200 && p[2] > p[0]);
    // ink 120 x 5 px plus 4 px padding each side (jpeg ringing may widen the box by a pixel or two)
    let bbox_ok = (img.width() as i32 - 128).abs() <= 4 && (img.height() as i32 - 13).abs() <= 4;
    let again = sig::Lib { dir: dir.clone() };
    let round = again.list().len() == 1 && again.list()[0].name == "src" && again.kind_of(&it.id).as_deref() == Some("signature");
    again.rename(&it.id, "mine").unwrap();
    let renamed = sig::Lib { dir: dir.clone() }.list()[0].name == "mine";
    println!("library: background transparent {corner_clear}, ink colour kept {ink_kept}, crop {}x{} ok {bbox_ok}, index round trip {round}, rename {renamed}, thumbnail <= 240 {}", img.width(), img.height(), it.data_url.len() > 20);
    assert!(corner_clear && ink_kept && bbox_ok && round && renamed);
    again.delete(&it.id).unwrap();
    println!("library: after delete {} items, png file gone {}", again.list().len(), again.png(&it.id).is_none());
    assert!(again.list().is_empty() && again.png(&it.id).is_none());
    // drawn (already transparent): crop only
    let drawn = again.add_drawn(&data_url(&alpha_png()), "drawn").unwrap();
    assert_eq!((drawn.w, drawn.h), (60, 40));
    // place it
    let Some(mut s) = open(RK) else {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    s.lib_dir = Some(dir.clone());
    let mut r = req("image", 0, None, [100.0, 400.0, 90.0, 60.0]);
    r.asset = Some(drawn.id.clone());
    let id1 = new_id(s.apply_obj(&r).unwrap());
    let mut r2 = req("image", 0, None, [300.0, 400.0, 90.0, 60.0]);
    r2.asset = Some(drawn.id.clone());
    let id2 = new_id(s.apply_obj(&r2).unwrap());
    // the library item may go: the placed edits keep working
    again.delete(&drawn.id).unwrap();
    let mut r3 = req("image", 0, Some(id2), [300.0, 420.0, 90.0, 60.0]);
    r3.asset = None;
    s.apply_obj(&r3).unwrap();
    let b = s.built().unwrap();
    let d = Document::load_mem(&b.bytes).unwrap();
    let pid = d.get_pages()[&1];
    let xo = effective_resources(&d, pid).unwrap().get(b"XObject").map(|o| deref(&d, o).as_dict().unwrap().clone()).unwrap();
    let mine: Vec<(&Vec<u8>, &Object)> = xo.iter().filter(|(k, _)| k.starts_with(b"MjSg")).collect();
    let smask = mine.first().map(|(_, o)| matches!(deref(&d, o), Object::Stream(s) if s.dict.has(b"SMask"))).unwrap_or(false);
    let after = shot(&b.bytes, &s.page(0).unwrap(), 100.0);
    let k = 100.0 / 72.0;
    let px = |x: f64, y: f64| after.get_pixel((x * k) as u32, (y * k) as u32).0;
    let centre = px(145.0, 430.0);
    let corner = px(101.0, 401.0);
    after.save(e2_dir().join("image.png")).unwrap();
    println!("image: XObjects named MjSg {}, SMask present {smask}, centre pixel red {}, transparent corner stays page colour {}, ids {id1},{id2}", mine.len(), centre[0] > 180 && centre[1] < 80, corner[0] > 200 && corner[1] > 200);
    assert!(mine.len() == 1 && smask && centre[0] > 180 && centre[1] < 80 && corner[1] > 200);
    let _ = std::fs::remove_dir_all(&dir);
}

// E7: format
#[test]
fn e2_format() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let base = s.model(0).unwrap().paras[p.id as usize].style.clone();
    let fmt = |f: &dyn Fn(&mut Fmt)| {
        let mut x = base.clone();
        f(&mut x);
        x
    };
    let ap = |s: &mut Session, st: Fmt| s.apply(&EditApplyReq { page: 0, para: p.id, text: norm(&p.text), use_pc_font: false, allow_overlap: true, style: Some(st), runs: None }).unwrap();
    // size x1.25: the pitch scales and the paragraph is re-analysed at the new size
    let r = ap(&mut s, fmt(&|x| x.size = (p.size * 1.25 * 2.0).round() / 2.0));
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    let b = s.built().unwrap();
    let pa2 = reanalyse(&b.bytes, 0);
    let q = pa2.paras.iter().find(|q| q.ed.is_some() && norm(&q.text) == norm(&p.text)).unwrap();
    let want = p.size * 1.25;
    println!("size change: size {:.2} -> {:.2}, pitch {:.2} -> {:.2}, lines {} -> {}", p.size, q.size, p.pitch, q.pitch, p.nlines, q.nlines);
    assert!((q.size - want).abs() < 0.6 && (q.pitch / p.pitch - q.size / p.size).abs() < 0.03);
    shot(&b.bytes, &pa, 100.0).save(e2_dir().join("format_size.png")).unwrap();
    // colour
    let r = ap(&mut s, fmt(&|x| x.color = "#C00000".into()));
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    let b = s.built().unwrap();
    let red = content(&b.bytes, 0).windows(8).any(|w| w == b"0.753 0 ");
    println!("colour change: fill operator present {red}");
    assert!(red);
    // bold: the page's own bold resource when it has the glyphs, else the PC-font flow
    let r = ap(&mut s, fmt(&|x| x.bold = true));
    let b = s.built().unwrap();
    match r {
        EditApplyRes::Ok { .. } => {
            let model = s.model(0).unwrap();
            println!("bold: existing bold resource used, pc font used {}", model.paras[p.id as usize].pc_font_used);
            assert!(!b.pc_used);
            shot(&b.bytes, &pa, 100.0).save(e2_dir().join("format_bold.png")).unwrap();
        }
        EditApplyRes::MissingChars { chars } => {
            println!("bold: the page lacks some bold glyphs ({} chars): PC-font flow", chars.chars().count());
            let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: norm(&p.text), use_pc_font: true, allow_overlap: true, style: Some(fmt(&|x| x.bold = true)), runs: None }).unwrap();
            assert!(matches!(r, EditApplyRes::Ok { .. }));
            shot(&s.built().unwrap().bytes, &pa, 100.0).save(e2_dir().join("format_bold.png")).unwrap();
        }
        o => panic!("bold: {o:?}"),
    }
    // align change keeps the text
    let r = ap(&mut s, fmt(&|x| x.align = Align::Right));
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    // a burst of format changes on one paragraph is one undo step
    println!("history entries after 4 format changes of one paragraph: {}", s.hist.len());
    assert_eq!(s.hist.len(), 1);
}

// E8: z-order
#[test]
fn e2_z_order() {
    let _g = gs_guard();
    let Some(mut s) = open(RK) else { return };
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let r = glyph_rect(&pa, p, 4, 12);
    let rect = [r[0] - 5.0, r[1] - 3.0, 190.0, r[3] + 6.0];
    let style = Fmt { font: "Arial".into(), size: 11.0, color: "#000000".into(), bold: false, italic: false, align: Align::Left };
    let tb = |s: &mut Session, w: &str| {
        let mut q = req("textbox", 0, None, [rect[0] + 4.0, rect[1] + 2.0, 180.0, 12.0]);
        q.text = Some(w.into());
        q.style = Some(style.clone());
        new_id(s.apply_obj(&q).unwrap())
    };
    // white-out, then the text box: visible
    s.apply_obj(&req("whiteout", 0, None, rect)).unwrap();
    tb(&mut s, "plorvik");
    let t = text_of(&s.built().unwrap().bytes, 0);
    let vis = t.contains("plorvik");
    // text box, then white-out over it: removed
    let (mut s2, _) = (open(RK).unwrap(), ());
    let mut q = req("textbox", 0, None, [rect[0] + 4.0, rect[1] + 2.0, 180.0, 12.0]);
    q.text = Some("plorvik".into());
    q.style = Some(style.clone());
    s2.apply_obj(&q).unwrap();
    assert!(text_of(&s2.built().unwrap().bytes, 0).contains("plorvik"));
    s2.apply_obj(&req("whiteout", 0, None, rect)).unwrap();
    let gone = !text_of(&s2.built().unwrap().bytes, 0).contains("plorvik");
    println!("z-order: white-out then text box visible {vis}; text box then white-out removed {gone}");
    assert!(vis && gone);
    // and saving such a document passes the output verification
    let dir = std::env::temp_dir().join(format!("majipdf_e2_z_{}", stamp()));
    s2.save(&dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

// E9: undo / redo across mixed edit kinds restores the bytes
#[test]
fn e2_undo_redo_mixed() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(0.0);
    let dir = std::env::temp_dir().join(format!("majipdf_e2_mix_{}", stamp()));
    let lib = sig::Lib { dir: dir.clone() };
    let drawn = lib.add_drawn(&data_url(&alpha_png()), "d").unwrap();
    s.lib_dir = Some(dir.clone());
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let mut snaps: Vec<Vec<u8>> = vec![s.orig.clone()];
    let style = Fmt { font: "Times New Roman".into(), size: 10.0, color: "#1F3864".into(), bold: true, italic: false, align: Align::Center };
    let mut a = req("textbox", 0, None, [100.0, 100.0, 150.0, 12.0]);
    a.text = Some("garvel mintoq".into());
    a.style = Some(style);
    let tid = new_id(s.apply_obj(&a).unwrap());
    snaps.push(s.built().unwrap().bytes.clone());
    let mut i = req("image", 0, None, [300.0, 300.0, 80.0, 50.0]);
    i.asset = Some(drawn.id.clone());
    new_id(s.apply_obj(&i).unwrap());
    snaps.push(s.built().unwrap().bytes.clone());
    let r = glyph_rect(&pa, p, 4, 12);
    let wid = new_id(s.apply_obj(&req("whiteout", 0, None, r)).unwrap());
    snaps.push(s.built().unwrap().bytes.clone());
    assert!(matches!(s.apply(&EditApplyReq { page: 0, para: pa.paras.iter().find(|q| q.ed.is_some() && q.nlines == 6).unwrap().id, text: format!("{} {}", norm(&pa.paras.iter().find(|q| q.ed.is_some() && q.nlines == 6).unwrap().text), made_up(3)), use_pc_font: false, allow_overlap: true, style: None, runs: None }).unwrap(), EditApplyRes::Ok { .. }));
    snaps.push(s.built().unwrap().bytes.clone());
    s.apply_obj(&req("delete", 0, Some(tid), [0.0; 4])).unwrap();
    snaps.push(s.built().unwrap().bytes.clone());
    let _ = wid;
    let n = snaps.len() - 1;
    let mut back = true;
    for k in (0..n).rev() {
        s.step(true).unwrap();
        back &= s.built().unwrap().bytes == snaps[k];
    }
    let mut fwd = true;
    for k in 1..=n {
        s.step(false).unwrap();
        fwd &= s.built().unwrap().bytes == snaps[k];
    }
    let distinct = (1..snaps.len()).all(|k| snaps[k] != snaps[k - 1]);
    println!("mixed history of {n} edits: undo restores every earlier state exactly {back}, redo restores every later state exactly {fwd}, states differ {distinct}");
    assert!(back && fwd && distinct);
    let _ = std::fs::remove_dir_all(&dir);
}

// bold off on a bold heading: the page's own regular resource, no PC font
#[test]
fn e2_variant_resource_is_used() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let Some(p) = pa.paras.iter().find(|p| p.ed.is_some() && !p.pc_font && p.bold && !p.italic) else { return };
    let mut st = s.model(0).unwrap().paras[p.id as usize].style.clone();
    st.bold = false;
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: norm(&p.text), use_pc_font: false, allow_overlap: true, style: Some(st), runs: None }).unwrap();
    let ok = matches!(r, EditApplyRes::Ok { .. });
    let pc = s.built().unwrap().pc_used;
    println!("bold heading -> regular: ok {ok}, pc font used {pc}");
    assert!(ok && !pc);
}

// a page /Thumb shows the old content: it goes, and its image object with it
#[test]
fn e2_thumb_is_dropped() {
    let Some(src) = sample(RK) else { return };
    let dir = std::env::temp_dir().join(format!("majipdf_e2_thumb_{}", stamp()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = Document::load(&src).unwrap();
    let pid = d.get_pages()[&1];
    let th = d.add_object(Stream::new(lopdf::dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 7, "Height" => 5, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 }, vec![0u8; 35]));
    d.get_object_mut(pid).unwrap().as_dict_mut().unwrap().set("Thumb", Object::Reference(th));
    let f = dir.join("t.pdf");
    d.save(&f).unwrap();
    let (mut s, _) = Session::open(&f, None, None).unwrap();
    let mut q = req("whiteout", 0, None, [100.0, 100.0, 50.0, 20.0]);
    q.cover_only = Some(true);
    s.apply_obj(&q).unwrap();
    let out = Document::load_mem(&s.built().unwrap().bytes).unwrap();
    let has_thumb = out.get_dictionary(out.get_pages()[&1]).unwrap().has(b"Thumb");
    let has_obj = out.objects.values().any(|o| matches!(o, Object::Stream(s) if s.dict.get(b"Width").ok().and_then(|w| w.as_i64().ok()) == Some(7)));
    println!("page /Thumb dropped {}, thumbnail object gone {}", !has_thumb, !has_obj);
    assert!(!has_thumb && !has_obj);
    let _ = std::fs::remove_dir_all(&dir);
}

// white ink on a dark background: the ink stays, the background goes
#[test]
fn e2_dark_background_import() {
    let dir = std::env::temp_dir().join(format!("majipdf_e2_dark_{}", stamp()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut im = image::RgbImage::from_pixel(200, 100, image::Rgb([20, 20, 20]));
    for x in 40..160 {
        for y in 45..52 {
            im.put_pixel(x, y, image::Rgb([240, 240, 240]));
        }
    }
    let p = dir.join("dark.png");
    im.save(&p).unwrap();
    let lib = sig::Lib { dir: dir.join("lib") };
    let it = lib.import(&p, "stamp").unwrap();
    let img = image::load_from_memory(&lib.png(&it.id).unwrap()).unwrap().to_rgba8();
    let opaque = img.pixels().filter(|p| p[3] > 200).count() as f64 / (img.width() * img.height()) as f64;
    // the thumbnail is cached on disk after the first listing
    let n = lib.list().len();
    let cached = dir.join("lib").join(format!("{}.thumb.png", it.id)).exists();
    println!("dark import: {}x{} px, opaque share {:.2}, corner transparent {}, list {n}, thumb cached {cached}", img.width(), img.height(), opaque, img.get_pixel(0, 0)[3] == 0);
    assert!(opaque < 0.9 && img.pixels().any(|p| p[3] > 200 && p[0] > 200) && cached);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------- Edit 2: runs (selection formatting) ----------------

/// `base` with `f` applied to the formats of chars `from..to` (char offsets of the concatenated text).
fn restyle_range(base: &[Run], from: usize, to: usize, f: &dyn Fn(&mut Fmt)) -> Vec<Run> {
    let mut v: Vec<Run> = vec![];
    let mut i = 0;
    for r in base {
        for c in r.text.chars() {
            let mut st = r.style.clone();
            if i >= from && i < to {
                f(&mut st);
            }
            match v.last_mut() {
                Some(l) if l.style == st => l.text.push(c),
                _ => v.push(Run { text: c.to_string(), style: st }),
            }
            i += 1;
        }
    }
    v
}

/// Char range of the k-th word of the runs text.
fn word_at(base: &[Run], k: usize) -> (usize, usize) {
    let t: Vec<char> = base.iter().flat_map(|r| r.text.chars()).collect();
    let (mut n, mut i) = (0, 0);
    while i < t.len() {
        if t[i].is_whitespace() {
            i += 1;
            continue;
        }
        let st = i;
        while i < t.len() && !t[i].is_whitespace() {
            i += 1;
        }
        if n == k {
            return (st, i);
        }
        n += 1;
    }
    (0, 0)
}

fn apply_runs(s: &mut Session, p: &Para, runs: Vec<Run>, use_pc: bool) -> EditApplyRes {
    let text = runs.iter().map(|r| r.text.as_str()).collect::<String>();
    s.apply(&EditApplyReq { page: 0, para: p.id, text, use_pc_font: use_pc, allow_overlap: true, style: None, runs: Some(runs) }).unwrap()
}

#[test]
fn e2_runs_selection_format() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let base = s.model(0).unwrap().paras[p.id as usize].runs.clone();
    let base_style = base[0].style.clone();
    println!("direct paragraph: style.font is orig {}, base runs {}", s.model(0).unwrap().paras[p.id as usize].style.font == "orig", base.len());
    assert_eq!(s.model(0).unwrap().paras[p.id as usize].style.font, "orig");
    // colour on two words: no PC font, no missing chars
    let (a, _) = word_at(&base, 5);
    let (_, b) = word_at(&base, 6);
    let red = |x: &mut Fmt| x.color = "#ff0000".into();
    let r = apply_runs(&mut s, p, restyle_range(&base, a, b, &red), false);
    let built = s.built().unwrap();
    let fill = content(&built.bytes, 0).windows(8).any(|w| w == b"1 0 0 rg");
    println!("colour on 2 words: ok {}, pc font {}, red fill present {fill}", matches!(r, EditApplyRes::Ok { .. }), built.pc_used);
    assert!(matches!(r, EditApplyRes::Ok { .. }) && !built.pc_used && fill);
    let m = s.model(0).unwrap().paras[p.id as usize].runs.clone();
    assert!(m.len() == 3 && m[1].style.color == "#ff0000" && m[1].text.split_whitespace().count() == 2);
    // size on two words (+3 pt): the line pitch of those lines grows, text and wrapping stay valid
    let big = |x: &mut Fmt| x.size += 3.0;
    let r = apply_runs(&mut s, p, restyle_range(&base, a, b, &big), false);
    let built = s.built().unwrap();
    let pa2 = reanalyse(&built.bytes, 0);
    let q = pa2.paras.iter().find(|q| q.ed.is_some() && norm(&q.text) == norm(&p.text));
    println!("size on 2 words: ok {}, pc font {}, paragraph re-found with the same text {}", matches!(r, EditApplyRes::Ok { .. }), built.pc_used, q.is_some());
    assert!(matches!(r, EditApplyRes::Ok { .. }) && !built.pc_used);
    shot(&built.bytes, &pa, 100.0).save(e2_dir().join("runs_size.png")).unwrap();
    let sizes: Vec<f64> = s.model(0).unwrap().paras[p.id as usize].runs.iter().map(|r| r.style.size).collect();
    assert!(sizes.iter().any(|z| (z - (base_style.size + 3.0)).abs() < 0.02));
    // bold on a selection that starts inside a word: bold resource when the page has the glyphs, else missing_chars once
    let bold = |x: &mut Fmt| (x.bold, x.italic) = (true, true); // no bold italic resource on the page
    let (w0, _) = word_at(&base, 2);
    let (_, w1) = word_at(&base, 3);
    let runs = restyle_range(&base, w0 + 1, w1 - 1, &bold);
    assert!(runs.len() == 3);
    let r = apply_runs(&mut s, p, runs.clone(), false);
    let missing = match &r {
        EditApplyRes::MissingChars { chars } => chars.chars().count(),
        _ => 0,
    };
    println!("bold inside words: first answer {}", if matches!(r, EditApplyRes::Ok { .. }) { "ok (page bold resource)" } else { "missing_chars" });
    if matches!(r, EditApplyRes::MissingChars { .. }) {
        assert!(missing > 0);
        // sticky PC font: accepted once, only the bold run switches; further applies never ask again
        let r = apply_runs(&mut s, p, runs, true);
        assert!(matches!(r, EditApplyRes::Ok { .. }));
        let built = s.built().unwrap();
        let m = s.model(0).unwrap().paras[p.id as usize].runs.clone();
        let pcruns = m.iter().filter(|r| r.style.italic).count();
        println!("bold with use_pc_font: pc font embedded {}, runs {} of which in a PC family {pcruns}", built.pc_used, m.len());
        assert!(built.pc_used && m.len() == 3 && m[1].style.bold && m[1].style.italic && !m[0].style.bold);
        shot(&built.bytes, &pa, 100.0).save(e2_dir().join("runs_bold.png")).unwrap();
        // more edits on the same paragraph with use_pc_font: colour on other words, and a character the page lacks
        let (c0, _) = word_at(&m, 10);
        let (_, c1) = word_at(&m, 11);
        let r = apply_runs(&mut s, p, restyle_range(&m, c0, c1, &red), true);
        assert!(matches!(r, EditApplyRes::Ok { .. }), "sticky: no new prompt");
        let m2 = s.model(0).unwrap().paras[p.id as usize].runs.clone();
        let mut with_q = m2.clone();
        with_q.last_mut().unwrap().text.push_str(" Q@#");
        let r = apply_runs(&mut s, p, with_q, true);
        println!("sticky use_pc_font: later apply ok {}, even with characters the page lacks", matches!(r, EditApplyRes::Ok { .. }));
        assert!(matches!(r, EditApplyRes::Ok { .. }));
        let t = text_of(&s.built().unwrap().bytes, 0);
        assert!(t.contains("Q@#"));
    }
    // back to the original look: no edit left
    let r = apply_runs(&mut s, p, base.clone(), false);
    assert!(matches!(r, EditApplyRes::Ok { .. }));
    assert_eq!(s.effective().iter().filter(|x| matches!(x, Item::Para(_))).count(), 0);
}

#[test]
fn e2_runs_bold_resource_when_glyphs_exist() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let bold = |x: &mut Fmt| x.bold = true;
    let (mut tried, mut found) = (0, false);
    'o: for p in pa.paras.iter().filter(|p| p.ed.is_some() && !p.pc_font && !p.bold && p.nlines >= 2) {
        let base = s.model(0).unwrap().paras[p.id as usize].runs.clone();
        let n = norm(&p.text).split(' ').count();
        for k in 0..n.saturating_sub(1) {
            tried += 1;
            let (a, _) = word_at(&base, k);
            let (_, b) = word_at(&base, k + 1);
            let r = apply_runs(&mut s, p, restyle_range(&base, a, b, &bold), false);
            if matches!(r, EditApplyRes::Ok { .. }) {
                let built = s.built().unwrap();
                println!("bold on 2 words uses the page bold resource (no PC font): pc font {} after {tried} tries", built.pc_used);
                assert!(!built.pc_used);
                let pa2 = reanalyse(&built.bytes, 0);
                let ok = pa2.paras.iter().any(|q| norm(&q.text) == norm(&p.text));
                shot(&built.bytes, &pa, 100.0).save(e2_dir().join("runs_bold_resource.png")).unwrap();
                println!("line breaks valid, paragraph found again with the same text: {ok}");
                assert!(ok);
                found = true;
                break 'o;
            }
        }
    }
    println!("pairs tried {tried}, found one: {found}");
}

#[test]
fn e2_format_font_name_is_orig_and_size_needs_no_pc() {
    let Some(mut s) = open(RK) else { return };
    s.floor_override = Some(-1e6);
    let pa = s.page(0).unwrap();
    let p = rk_para(&pa, 7);
    let m = s.model(0).unwrap();
    assert!(m.paras.iter().filter(|q| q.status == "direct").all(|q| q.style.font == "orig"));
    // the frontend may send the CSS family of the original font: that still means the original resource
    let mut st = m.paras[p.id as usize].style.clone();
    st.font = p.css_font.clone();
    st.size += 1.0;
    st.color = "#1F3864".into();
    let r = s.apply(&EditApplyReq { page: 0, para: p.id, text: norm(&p.text), use_pc_font: false, allow_overlap: true, style: Some(st), runs: None }).unwrap();
    let built = s.built().unwrap();
    println!("size + colour with the CSS family name as font: ok {}, pc font {}", matches!(r, EditApplyRes::Ok { .. }), built.pc_used);
    assert!(matches!(r, EditApplyRes::Ok { .. }) && !built.pc_used);
}

#[test]
fn e2_textbox_runs() {
    let Some(mut s) = open(RK) else { return };
    let st = |size: f64| Fmt { font: "Arial".into(), size, color: "#000000".into(), bold: false, italic: false, align: Align::Left };
    // mixed sizes on one line, a split inside a word, a hard break and a smaller second line
    let runs = vec![Run { text: "zor".into(), style: st(12.0) }, Run { text: "bal quen".into(), style: st(24.0) }, Run { text: "\nvimrath".into(), style: st(12.0) }];
    let mut r = req("textbox", 0, None, [72.0, 60.0, 400.0, 14.0]);
    r.runs = Some(runs.clone());
    let id = new_id(s.apply_obj(&r).unwrap());
    let m = s.model(0).unwrap();
    let o = m.objs.iter().find(|o| o.id == id).unwrap();
    let want = 1.15 * 24.0 + 1.15 * 12.0;
    let b = s.built().unwrap();
    let t = text_of(&b.bytes, 0).split_whitespace().collect::<Vec<_>>().join(" ");
    shot(&b.bytes, &s.page(0).unwrap(), 100.0).save(e2_dir().join("textbox_runs.png")).unwrap();
    println!("text box runs: height {:.2} (want {:.2}), word split inside zor|bal kept whole in extraction {}, runs echoed {}", o.rect[3], want, t.contains("zorbal"), o.runs.as_ref().map(|r| r.len()).unwrap_or(0));
    assert!((o.rect[3] - want).abs() < 1e-6 && t.contains("zorbal") && o.runs.as_ref().unwrap().len() == 3);
    // a missing bold variant on orig asks once, then use_pc_font answers
    let mut orig_runs = runs.clone();
    orig_runs[0].style.font = "orig".into();
    orig_runs[0].style.bold = true;
    let mut r2 = req("textbox", 0, Some(id), [72.0, 60.0, 400.0, 14.0]);
    r2.runs = Some(orig_runs.clone());
    let first = s.apply_obj(&r2).unwrap();
    println!("text box orig bold: first answer {}", match &first { ObjRes::Ok { .. } => "ok", ObjRes::MissingChars { .. } => "missing_chars", _ => "other" });
    if matches!(first, ObjRes::MissingChars { .. }) {
        r2.use_pc_font = Some(true);
        assert!(matches!(s.apply_obj(&r2).unwrap(), ObjRes::Ok { .. }));
    }
}
