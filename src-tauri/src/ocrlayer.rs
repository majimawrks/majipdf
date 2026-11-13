//! Invisible text layer (Accurate OCR): writes recognised line boxes as `Tr 3` Helvetica text onto the ORIGINAL pages.
//! No re-render and no Ghostscript pass, so the file grows by a few KB per page. See _docs/paddle-contract.md.
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};

/// One recognised line, in pixels of the rendered page (origin top-left, y down).
pub struct Word {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub text: String,
}

/// Helvetica AFM advance widths (1/1000 em) for the printable ASCII range 0x20..=0x7E.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // space .. /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 0 .. ?
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // @ .. O
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // P .. _
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // ` .. o
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, // p .. ~
];

/// WinAnsi byte for `c`; anything we can't encode becomes `?`. ponytail: the 0x80..0x9F typographic extras
/// (euro, curly quotes, dashes) are written as `?`; they only matter for searching, not for the layout.
fn winansi(c: char) -> u8 {
    match c as u32 {
        0x20..=0x7E | 0xA0..=0xFF => c as u8,
        _ => b'?',
    }
}

/// Helvetica advance of `s` (WinAnsi bytes) in 1/1000 em. ponytail: non-ASCII bytes count as 556.
fn width1000(s: &[u8]) -> u32 {
    s.iter().map(|&b| if (0x20..=0x7E).contains(&b) { HELV[(b - 0x20) as usize] as u32 } else { 556 }).sum()
}

/// (font size, horizontal scale in percent) so a string drawn in Helvetica fills a `bw` x `bh` point box.
pub fn fit(s: &[u8], bw: f64, bh: f64) -> (f64, f64) {
    let fs = (bh * 0.75).max(1.0);
    let w = width1000(s) as f64 * fs / 1000.0;
    // ponytail: clamp keeps one bad box from producing absurd glyph scaling
    let tz = if w > 0.0 { (100.0 * bw / w).clamp(10.0, 1000.0) } else { 100.0 };
    (fs, tz)
}

fn esc(s: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(s.len() + 2);
    for &b in s {
        if matches!(b, b'(' | b')' | b'\\') {
            o.push(b'\\');
        }
        o.push(b);
    }
    o
}

fn num(o: &Object) -> f64 {
    match o {
        Object::Integer(i) => *i as f64,
        Object::Real(r) => *r as f64,
        _ => 0.0,
    }
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    match o {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(o),
        _ => o,
    }
}

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

/// (x origin, y origin, width, height, rotation) of the page as displayed (crop box, else media box).
fn page_geom(doc: &Document, page: ObjectId) -> (f64, f64, f64, f64, i64) {
    let get = |k: &[u8]| -> Option<[f64; 4]> {
        let a = inherited(doc, page, k)?.as_array().ok()?;
        (a.len() == 4).then(|| [num(deref(doc, &a[0])), num(deref(doc, &a[1])), num(deref(doc, &a[2])), num(deref(doc, &a[3]))])
    };
    let b = get(b"CropBox").or_else(|| get(b"MediaBox")).unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let rot = inherited(doc, page, b"Rotate").map(|o| num(o) as i64).unwrap_or(0).rem_euclid(360);
    (b[0].min(b[2]), b[1].min(b[3]), (b[2] - b[0]).abs(), (b[3] - b[1]).abs(), if rot % 90 == 0 { rot } else { 0 })
}

/// Content for one page. `img_w`/`img_h` are the rendered pixel size; the displayed page is mapped onto them.
fn page_ops(words: &[Word], img_w: f64, img_h: f64, geom: (f64, f64, f64, f64, i64)) -> Vec<u8> {
    let (ox, oy, w, h, rot) = geom;
    let (dw, dh) = if rot == 90 || rot == 270 { (h, w) } else { (w, h) }; // displayed size in points
    let (sx, sy) = (dw / img_w.max(1.0), dh / img_h.max(1.0));
    let mut out = b"q\nBT\n3 Tr\n".to_vec();
    for wd in words {
        let t: Vec<u8> = wd.text.chars().map(winansi).collect();
        let (bw, bh) = ((wd.x1 - wd.x0) as f64 * sx, (wd.y1 - wd.y0) as f64 * sy);
        if t.iter().all(|&b| b == b' ') || bw < 1.0 || bh < 1.0 {
            continue;
        }
        let (fs, tz) = fit(&t, bw, bh);
        // displayed point, y up, baseline a little above the box bottom
        let (dx, dy) = (wd.x0 as f64 * sx, dh - (wd.y1 as f64 * sy - 0.2 * bh));
        // displayed -> unrotated user space, plus the text direction
        let (m, ux, uy) = match rot {
            90 => ([0.0, 1.0, -1.0, 0.0], w - dy, dx),
            180 => ([-1.0, 0.0, 0.0, -1.0], w - dx, h - dy),
            270 => ([0.0, -1.0, 1.0, 0.0], dy, h - dx),
            _ => ([1.0, 0.0, 0.0, 1.0], dx, dy),
        };
        out.extend(format!("/MjOcr {fs:.2} Tf {tz:.1} Tz {} {} {} {} {:.2} {:.2} Tm (", m[0], m[1], m[2], m[3], ux + ox, uy + oy).bytes());
        out.extend(esc(&t));
        out.extend(b") Tj\n");
    }
    out.extend(b"ET\nQ\n");
    out
}

/// Adds the invisible layer to each `(page number from 1, image width px, image height px, words)`.
pub fn add_layers(doc: &mut Document, pages: &[(u32, u32, u32, Vec<Word>)]) -> Result<(), String> {
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let ids = doc.get_pages();
    for (n, iw, ih, words) in pages {
        let Some(&pid) = ids.get(n) else { continue };
        if words.is_empty() {
            continue;
        }
        let ops = page_ops(words, *iw as f64, *ih as f64, page_geom(doc, pid));
        // Resources: a direct copy on the page (inherited or indirect ones may be shared), plus our font.
        let mut res = match inherited(doc, pid, b"Resources").and_then(|o| o.as_dict().ok()) {
            Some(d) => d.clone(),
            None => Dictionary::new(),
        };
        let mut fonts = match res.get(b"Font").ok().map(|o| deref(doc, o)).and_then(|o| o.as_dict().ok()) {
            Some(d) => d.clone(),
            None => Dictionary::new(),
        };
        fonts.set("MjOcr", font);
        res.set("Font", fonts);
        let open = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
        // The closing `Q` first: puts the graphics state back even when the page's own content is unbalanced.
        let mut layer = b"Q\n".to_vec();
        layer.extend(ops);
        let layer = doc.add_object(Stream::new(Dictionary::new(), layer));
        let old: Vec<Object> = match doc.get_dictionary(pid).ok().and_then(|d| d.get(b"Contents").ok()).map(|o| deref(doc, o).clone()) {
            Some(Object::Array(a)) => a,
            Some(Object::Stream(_)) => match doc.get_dictionary(pid).ok().and_then(|d| d.get(b"Contents").ok()) {
                Some(r) => vec![r.clone()],
                None => vec![],
            },
            _ => vec![],
        };
        let mut all = vec![Object::Reference(open)];
        all.extend(old);
        all.push(Object::Reference(layer));
        let page = doc.get_dictionary_mut(pid).map_err(|e| e.to_string())?;
        page.set("Resources", res);
        page.set("Contents", all);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helvetica_width_and_tz() {
        // "Hello": H 722 + e 556 + l 222 + l 222 + o 556 = 2278
        assert_eq!(width1000(b"Hello"), 2278);
        // box 20 pt high -> 15 pt font; natural width 2.278 * 15 = 34.17 pt; a 68.34 pt box doubles it
        let (fs, tz) = fit(b"Hello", 68.34, 20.0);
        assert!((fs - 15.0).abs() < 1e-9);
        assert!((tz - 200.0).abs() < 0.01, "{tz}");
        assert_eq!(fit(b"", 50.0, 10.0).1, 100.0);
        assert_eq!(fit(b"i", 1000.0, 10.0).1, 1000.0); // clamped
        assert_eq!(winansi('\u{4e2d}'), b'?');
        assert_eq!(winansi('\u{e9}'), 0xE9);
        assert_eq!(esc(b"a(b)\\"), b"a\\(b\\)\\\\");
    }

    #[test]
    fn layer_is_added_and_balanced() {
        let mut doc = Document::with_version("1.5");
        let pages = doc.new_object_id();
        let c = doc.add_object(Stream::new(Dictionary::new(), b"q 1 0 0 1 0 0 cm".to_vec())); // deliberately unbalanced
        let p = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()], "Rotate" => 90, "Contents" => c });
        doc.objects.insert(pages, dictionary! { "Type" => "Pages", "Kids" => vec![p.into()], "Count" => 1 }.into());
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", cat);
        let w = vec![Word { x0: 10.0, y0: 10.0, x1: 110.0, y1: 30.0, text: "Hi (x)".into() }];
        add_layers(&mut doc, &[(1, 100, 200, w)]).unwrap();
        let mut buf = vec![];
        doc.save_to(&mut buf).unwrap();
        let back = Document::load_mem(&buf).unwrap();
        let pid = back.get_pages()[&1];
        let text = back.get_page_content(pid).unwrap();
        let s = String::from_utf8_lossy(&text);
        assert!(s.contains("3 Tr") && s.contains("(Hi \\(x\\)) Tj") && s.contains("/MjOcr"), "{s}");
        let fonts = back.get_page_fonts(pid).unwrap();
        assert!(fonts.contains_key(b"MjOcr".as_slice()));
    }
}
