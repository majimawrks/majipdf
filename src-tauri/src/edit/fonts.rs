//! Fonts for Edit PDF: widths, code <-> text maps, glyph availability and PC-font embedding.
//! See _docs/edit-contract.md ("Page analysis" 9, "Glyph availability", "PC-font fallback").
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Kind {
    #[default]
    Simple,
    Composite,
    Type3,
}

#[derive(Clone, Default)]
pub struct Fnt {
    #[allow(dead_code)]
    pub name: String,         // resource name on the page
    pub id: Option<ObjectId>, // font dict object (document-wide key for "codes already shown")
    pub kind: Kind,
    pub base: String, // BaseFont without the subset prefix
    pub bold: bool,
    pub italic: bool,
    pub nb: u8, // bytes per code
    first: i64,
    widths: Vec<f64>,
    missing: f64,
    cw: HashMap<u32, f64>,
    dw: f64,
    pub dec: HashMap<u32, String>, // code -> text
    pub enc: HashMap<char, u32>,   // text -> code (lowest code wins)
    pub file: Option<Vec<u8>>,     // embedded TrueType (outline check)
    pub embedded: bool,
    pub measurable: bool, // we can compute advances of the old text
    pub encodable: bool,  // we can write new text in this font
}

pub fn num(o: &Object) -> f64 {
    match o {
        Object::Integer(i) => *i as f64,
        Object::Real(r) => *r as f64,
        _ => 0.0,
    }
}

pub fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    doc.dereference(o).map(|x| x.1).unwrap_or(o)
}

fn name_of(o: Option<&Object>) -> Option<String> {
    match o {
        Some(Object::Name(n)) => Some(String::from_utf8_lossy(n).to_string()),
        _ => None,
    }
}

/// Resources of a page, walking up /Parent for inherited ones.
pub fn effective_resources(doc: &Document, page: ObjectId) -> Option<&Dictionary> {
    let mut node = doc.get_dictionary(page).ok()?;
    for _ in 0..32 {
        if let Ok(r) = node.get(b"Resources") {
            return deref(doc, r).as_dict().ok();
        }
        let p = node.get(b"Parent").ok()?.as_reference().ok()?;
        node = doc.get_dictionary(p).ok()?;
    }
    None
}

pub fn load_fonts(doc: &Document, page: ObjectId) -> BTreeMap<String, Fnt> {
    let mut out = BTreeMap::new();
    let Some(res) = effective_resources(doc, page) else { return out };
    let Some(Object::Dictionary(fd)) = res.get(b"Font").ok().map(|o| deref(doc, o)) else { return out };
    for (k, v) in fd.iter() {
        let id = v.as_reference().ok();
        if let Ok(d) = deref(doc, v).as_dict() {
            let name = String::from_utf8_lossy(k).to_string();
            out.insert(name.clone(), load_font(doc, name, id, d));
        }
    }
    out
}

// ---------------- encodings ----------------

const CP1252_HI: [u32; 32] = [
    0x20ac, 0, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017d, 0, 0, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x02dc, 0x2122,
    0x0161, 0x203a, 0x0153, 0, 0x017e, 0x0178,
];

pub fn cp1252(code: u8) -> Option<char> {
    match code {
        0x20..=0x7e | 0xa0..=0xff => Some(code as char),
        0x80..=0x9f => char::from_u32(CP1252_HI[(code - 0x80) as usize]).filter(|c| *c != '\0'),
        _ => None,
    }
}

/// WinAnsi code of a character (inverse of `cp1252`).
pub fn win_ansi(c: char) -> Option<u8> {
    match c as u32 {
        0x20..=0x7e | 0xa0..=0xff => Some(c as u8),
        u => CP1252_HI.iter().position(|&x| x == u && x != 0).map(|i| 0x80 + i as u8),
    }
}

fn standard(code: u8) -> Option<char> {
    match code {
        0x27 => Some('\u{2019}'),
        0x60 => Some('\u{2018}'),
        0x20..=0x7e => Some(code as char),
        _ => None,
    }
}

const GLYPHS: &[(&str, u32)] = &[
    ("space", 0x20), ("exclam", 0x21), ("quotedbl", 0x22), ("numbersign", 0x23), ("dollar", 0x24), ("percent", 0x25), ("ampersand", 0x26), ("quotesingle", 0x27), ("parenleft", 0x28),
    ("parenright", 0x29), ("asterisk", 0x2a), ("plus", 0x2b), ("comma", 0x2c), ("hyphen", 0x2d), ("period", 0x2e), ("slash", 0x2f), ("zero", 0x30), ("one", 0x31), ("two", 0x32),
    ("three", 0x33), ("four", 0x34), ("five", 0x35), ("six", 0x36), ("seven", 0x37), ("eight", 0x38), ("nine", 0x39), ("colon", 0x3a), ("semicolon", 0x3b), ("less", 0x3c), ("equal", 0x3d),
    ("greater", 0x3e), ("question", 0x3f), ("at", 0x40), ("bracketleft", 0x5b), ("backslash", 0x5c), ("bracketright", 0x5d), ("asciicircum", 0x5e), ("underscore", 0x5f), ("grave", 0x60),
    ("braceleft", 0x7b), ("bar", 0x7c), ("braceright", 0x7d), ("asciitilde", 0x7e), ("bullet", 0x2022), ("endash", 0x2013), ("emdash", 0x2014), ("quoteleft", 0x2018), ("quoteright", 0x2019),
    ("quotedblleft", 0x201c), ("quotedblright", 0x201d), ("ellipsis", 0x2026), ("Euro", 0x20ac), ("nbspace", 0xa0), ("nonbreakingspace", 0xa0), ("minus", 0x2212), ("fi", 0xfb01),
    ("fl", 0xfb02), ("section", 0xa7), ("degree", 0xb0), ("copyright", 0xa9), ("registered", 0xae), ("trademark", 0x2122),
];

fn glyph_char(n: &str) -> Option<char> {
    if let Some(h) = n.strip_prefix("uni").filter(|h| h.len() == 4).or_else(|| n.strip_prefix('u').filter(|h| (4..=6).contains(&h.len()))) {
        if let Some(c) = u32::from_str_radix(h, 16).ok().and_then(char::from_u32) {
            return Some(c);
        }
    }
    if n.len() == 1 && n.as_bytes()[0].is_ascii_alphabetic() {
        return n.chars().next();
    }
    GLYPHS.iter().find(|g| g.0 == n).and_then(|g| char::from_u32(g.1))
}

fn utf16(h: &str) -> String {
    let units: Vec<u16> = (0..h.len() / 4).filter_map(|i| u16::from_str_radix(&h[i * 4..i * 4 + 4], 16).ok()).collect();
    String::from_utf16_lossy(&units)
}

/// bfchar / bfrange sections of a ToUnicode CMap.
pub fn parse_tounicode(b: &[u8]) -> HashMap<u32, String> {
    let t = String::from_utf8_lossy(b).replace('<', " <").replace('>', "> ").replace('[', " [ ").replace(']', " ] ");
    let toks: Vec<&str> = t.split_whitespace().collect();
    let hex = |s: &str| s.trim_start_matches('<').trim_end_matches('>').to_string();
    let mut m = HashMap::new();
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            "beginbfchar" => {
                i += 1;
                while i + 1 < toks.len() && toks[i] != "endbfchar" {
                    if let Ok(c) = u32::from_str_radix(&hex(toks[i]), 16) {
                        m.insert(c, utf16(&hex(toks[i + 1])));
                    }
                    i += 2;
                }
            }
            "beginbfrange" => {
                i += 1;
                while i + 2 < toks.len() && toks[i] != "endbfrange" {
                    let (lo, hi) = (u32::from_str_radix(&hex(toks[i]), 16).unwrap_or(1), u32::from_str_radix(&hex(toks[i + 1]), 16).unwrap_or(0));
                    if toks[i + 2] == "[" {
                        i += 3;
                        let mut c = lo;
                        while i < toks.len() && toks[i] != "]" {
                            m.insert(c, utf16(&hex(toks[i])));
                            c += 1;
                            i += 1;
                        }
                        i += 1;
                    } else {
                        let d = utf16(&hex(toks[i + 2]));
                        for c in lo..=hi.min(lo + 0xffff) {
                            let mut s = d.clone();
                            if let Some(l) = s.pop() {
                                s.push(char::from_u32(l as u32 + (c - lo)).unwrap_or(l));
                            }
                            m.insert(c, s);
                        }
                        i += 3;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    m
}

// ---------------- loading ----------------

fn stream_bytes(doc: &Document, o: Option<&Object>) -> Option<Vec<u8>> {
    match o.map(|o| deref(doc, o)) {
        Some(Object::Stream(s)) => s.get_plain_content().ok().or_else(|| Some(s.content.clone())),
        _ => None,
    }
}

fn strip_subset(n: &str) -> String {
    match n.split_once('+') {
        Some((p, r)) if p.len() == 6 && p.bytes().all(|b| b.is_ascii_uppercase()) => r.to_string(),
        _ => n.to_string(),
    }
}

fn desc_of<'a>(doc: &'a Document, fd: &'a Dictionary) -> Option<&'a Dictionary> {
    fd.get(b"FontDescriptor").ok().map(|o| deref(doc, o)).and_then(|o| o.as_dict().ok())
}

fn load_font(doc: &Document, name: String, id: Option<ObjectId>, d: &Dictionary) -> Fnt {
    let g = |k: &[u8]| d.get(k).ok().map(|o| deref(doc, o));
    let sub = name_of(d.get(b"Subtype").ok()).unwrap_or_default();
    let mut f = Fnt { name, id, nb: 1, dw: 1000.0, ..Default::default() };
    f.base = strip_subset(&name_of(d.get(b"BaseFont").ok()).unwrap_or_default());
    let lower = f.base.to_lowercase();
    f.bold = ["bold", "black", "heavy"].iter().any(|w| lower.contains(w));
    f.italic = lower.contains("italic") || lower.contains("oblique");
    if sub == "Type3" {
        f.kind = Kind::Type3;
        return f;
    }
    let tu = stream_bytes(doc, d.get(b"ToUnicode").ok()).map(|b| parse_tounicode(&b)).unwrap_or_default();
    let mut desc_flags = 0i64;
    let mut take_desc = |f: &mut Fnt, fd: &Dictionary| {
        let flags = fd.get(b"Flags").map(|o| num(o) as i64).unwrap_or(0);
        desc_flags = flags;
        f.italic |= flags & 64 != 0;
        f.bold |= flags & 262144 != 0;
        f.missing = fd.get(b"MissingWidth").map(num).unwrap_or(0.0);
        f.embedded = fd.has(b"FontFile") || fd.has(b"FontFile2") || fd.has(b"FontFile3");
        f.file = stream_bytes(doc, fd.get(b"FontFile2").ok());
    };
    if sub == "Type0" {
        f.kind = Kind::Composite;
        f.nb = 2;
        let identity = name_of(d.get(b"Encoding").ok()).is_some_and(|n| n == "Identity-H");
        if let Some(Object::Array(a)) = g(b"DescendantFonts") {
            if let Some(Ok(cf)) = a.first().map(|o| deref(doc, o).as_dict()) {
                f.dw = cf.get(b"DW").map(num).unwrap_or(1000.0);
                if let Some(fd) = desc_of(doc, cf) {
                    take_desc(&mut f, fd);
                }
                if let Some(Object::Array(w)) = cf.get(b"W").ok().map(|o| deref(doc, o)) {
                    let w: Vec<&Object> = w.iter().map(|o| deref(doc, o)).collect();
                    let mut i = 0;
                    while i < w.len() {
                        let c = num(w[i]) as u32;
                        match w.get(i + 1) {
                            Some(Object::Array(ws)) => {
                                for (k, x) in ws.iter().enumerate() {
                                    f.cw.insert(c + k as u32, num(deref(doc, x)));
                                }
                                i += 2;
                            }
                            Some(hi) if i + 2 < w.len() => {
                                let (hi, x) = (num(hi) as u32, num(w[i + 2]));
                                for k in c..=hi.min(c + 0xffff) {
                                    f.cw.insert(k, x);
                                }
                                i += 3;
                            }
                            _ => break,
                        }
                    }
                }
                f.measurable = identity;
            }
        }
        f.dec = tu;
    } else {
        if let Some(fd) = desc_of(doc, d) {
            take_desc(&mut f, fd);
        }
        f.first = d.get(b"FirstChar").map(|o| num(deref(doc, o)) as i64).unwrap_or(0);
        if let Some(Object::Array(a)) = g(b"Widths") {
            f.widths = a.iter().map(|o| num(deref(doc, o))).collect();
        }
        f.measurable = !f.widths.is_empty();
        // encoding: base table + /Differences; absent on a symbolic font = unknown (only ToUnicode helps)
        let (base, diffs) = match g(b"Encoding") {
            Some(Object::Name(n)) => (Some(String::from_utf8_lossy(n).to_string()), None),
            Some(Object::Dictionary(ed)) => (name_of(ed.get(b"BaseEncoding").ok()), ed.get(b"Differences").ok().map(|o| deref(doc, o))),
            _ => (None, None),
        };
        let table: Option<fn(u8) -> Option<char>> = match base.as_deref() {
            Some("WinAnsiEncoding") => Some(cp1252),
            Some("StandardEncoding") => Some(standard),
            Some(_) => None, // MacRoman etc.: not supported
            None if desc_flags & 4 == 0 => Some(standard),
            None => None,
        };
        if let Some(t) = table {
            for c in 0..=255u8 {
                if let Some(ch) = t(c) {
                    f.dec.insert(c as u32, ch.to_string());
                }
            }
        }
        if let Some(Object::Array(a)) = diffs {
            let mut code = 0u32;
            for o in a {
                match deref(doc, o) {
                    Object::Integer(i) => code = *i as u32,
                    Object::Name(n) => {
                        match glyph_char(&String::from_utf8_lossy(n)) {
                            Some(c) => f.dec.insert(code, c.to_string()),
                            None => f.dec.remove(&code),
                        };
                        code += 1;
                    }
                    _ => {}
                }
            }
        }
        if f.dec.is_empty() {
            f.dec = tu;
        }
    }
    let mut codes: Vec<u32> = f.dec.keys().copied().collect();
    codes.sort_unstable();
    for c in codes {
        let mut it = f.dec[&c].chars();
        if let (Some(ch), None) = (it.next(), it.next()) {
            if c >= 32 || ch == ' ' {
                f.enc.entry(ch).or_insert(c);
            }
        }
    }
    f.encodable = f.measurable && !f.enc.is_empty();
    f
}

impl Fnt {
    /// Advance of a code in 1/1000 em.
    pub fn w(&self, code: u32) -> f64 {
        match self.kind {
            Kind::Composite => self.cw.get(&code).copied().unwrap_or(self.dw),
            _ => {
                let i = code as i64 - self.first;
                if i >= 0 && (i as usize) < self.widths.len() {
                    self.widths[i as usize]
                } else {
                    self.missing
                }
            }
        }
    }

    #[cfg(test)]
    pub fn set_flat_widths(&mut self, w: f64) {
        self.widths = vec![w; 256];
    }

    pub fn css_font(&self) -> String {
        let l = self.base.to_lowercase();
        let pick = |names: &[&str]| names.iter().any(|n| l.contains(n));
        let s = if pick(&["arial", "helvetica"]) {
            "Arial"
        } else if pick(&["times"]) {
            "Times New Roman"
        } else if pick(&["courier"]) {
            "Courier New"
        } else if pick(&["calibri"]) {
            "Calibri"
        } else if pick(&["cambria"]) {
            "Cambria"
        } else if pick(&["tahoma"]) {
            "Tahoma"
        } else if pick(&["verdana"]) {
            "Verdana"
        } else if pick(&["bookman"]) {
            "Bookman Old Style"
        } else if pick(&["century"]) {
            "Century Schoolbook"
        } else {
            return self.base.split(['-', ',']).next().unwrap_or("Arial").to_string();
        };
        s.into()
    }

    /// The code that writes `c` in this font, when the glyph is really there:
    /// shown before anywhere in the document, or (embedded TrueType) it has an outline, or (not embedded) any encodable char.
    pub fn code_for(&self, c: char, used: &HashSet<u32>) -> Option<u32> {
        let code = *self.enc.get(&c)?;
        if self.w(code) <= 0.0 {
            return None;
        }
        if used.contains(&code) {
            return Some(code);
        }
        if !self.embedded {
            return Some(code);
        }
        let face = ttf_parser::Face::parse(self.file.as_ref()?, 0).ok()?;
        has_outline(&face, c).then_some(code)
    }
}

struct Nb(u32);
impl ttf_parser::OutlineBuilder for Nb {
    fn move_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

fn has_outline(face: &ttf_parser::Face, c: char) -> bool {
    face.glyph_index(c)
        .map(|g| {
            let mut n = Nb(0);
            face.outline_glyph(g, &mut n).is_some() && n.0 > 0
        })
        .unwrap_or(false)
}

// ---------------- PC fonts ----------------

pub struct Pc {
    pub file: PathBuf,
    pub bytes: Vec<u8>,
    pub widths: [f64; 256], // 1/1000 em per WinAnsi code, 0 = no glyph
}

fn font_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        let w = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        vec![w.join("Fonts")]
    } else {
        ["/System/Library/Fonts/Supplemental", "/Library/Fonts", "/usr/share/fonts/truetype/msttcorefonts"].map(PathBuf::from).into()
    }
}

/// [regular, bold, italic, bold italic] candidate files (Windows name, Mac name).
fn family_files(base: &str) -> [[&'static str; 2]; 4] {
    let l = base.to_lowercase();
    let has = |s: &str| l.contains(s);
    if has("times") {
        [["times.ttf", "Times New Roman.ttf"], ["timesbd.ttf", "Times New Roman Bold.ttf"], ["timesi.ttf", "Times New Roman Italic.ttf"], ["timesbi.ttf", "Times New Roman Bold Italic.ttf"]]
    } else if has("calibri") {
        [["calibri.ttf", "Calibri.ttf"], ["calibrib.ttf", "Calibri Bold.ttf"], ["calibrii.ttf", "Calibri Italic.ttf"], ["calibriz.ttf", "Calibri Bold Italic.ttf"]]
    } else if has("tahoma") {
        [["tahoma.ttf", "Tahoma.ttf"], ["tahomabd.ttf", "Tahoma Bold.ttf"], ["tahoma.ttf", "Tahoma.ttf"], ["tahomabd.ttf", "Tahoma Bold.ttf"]]
    } else if has("courier") {
        [["cour.ttf", "Courier New.ttf"], ["courbd.ttf", "Courier New Bold.ttf"], ["couri.ttf", "Courier New Italic.ttf"], ["courbi.ttf", "Courier New Bold Italic.ttf"]]
    } else if has("bookman") {
        [["BOOKOS.TTF", "Bookman Old Style.ttf"], ["BOOKOSB.TTF", "Bookman Old Style Bold.ttf"], ["BOOKOSI.TTF", "Bookman Old Style Italic.ttf"], ["BOOKOSBI.TTF", "Bookman Old Style Bold Italic.ttf"]]
    } else if has("century") {
        [["CENTURY.TTF", "Century.ttf"], ["CENTURY.TTF", "Century.ttf"], ["CENTURY.TTF", "Century.ttf"], ["CENTURY.TTF", "Century.ttf"]]
    } else if has("verdana") {
        [["verdana.ttf", "Verdana.ttf"], ["verdanab.ttf", "Verdana Bold.ttf"], ["verdanai.ttf", "Verdana Italic.ttf"], ["verdanaz.ttf", "Verdana Bold Italic.ttf"]]
    } else {
        [["arial.ttf", "Arial.ttf"], ["arialbd.ttf", "Arial Bold.ttf"], ["ariali.ttf", "Arial Italic.ttf"], ["arialbi.ttf", "Arial Bold Italic.ttf"]]
    }
}

fn find_font(names: &[&str]) -> Option<PathBuf> {
    for dir in font_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        for n in names {
            if let Some(p) = files.iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n))) {
                return Some(p.clone());
            }
        }
    }
    None
}

/// The PC font that stands in for an original font: same family when installed, else Arial.
pub fn pc_font(base: &str, bold: bool, italic: bool) -> Result<Pc, String> {
    let idx = bold as usize + 2 * italic as usize;
    let pick = |b: &str, i: usize| {
        let f = family_files(b);
        find_font(&[f[i][0], f[i][1]]).or_else(|| find_font(&[f[0][0], f[0][1]]))
    };
    let file = pick(base, idx).or_else(|| pick("arial", idx)).ok_or("no_pc_font")?;
    let bytes = std::fs::read(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let face = ttf_parser::Face::parse(&bytes, 0).map_err(|e| format!("bad font {}: {e}", file.display()))?;
    let upem = face.units_per_em() as f64;
    let mut widths = [0.0; 256];
    for code in 32..=255u8 {
        if let Some(g) = cp1252(code).and_then(|c| face.glyph_index(c)) {
            widths[code as usize] = (face.glyph_hor_advance(g).unwrap_or(0) as f64 * 1000.0 / upem).round();
        }
    }
    Ok(Pc { file, bytes, widths })
}

pub const FAMILIES: [&str; 7] = ["Arial", "Times New Roman", "Calibri", "Tahoma", "Verdana", "Courier New", "Bookman Old Style"];

/// PC families whose regular TTF exists on this machine.
pub fn available_families() -> Vec<String> {
    FAMILIES.iter().filter(|f| { let r = family_files(f)[0]; find_font(&[r[0], r[1]]).is_some() }).map(|s| s.to_string()).collect()
}

/// Measuring view of an embedded PC font (resource MjFn), for glyph removal inside our own appended text.
pub fn pc_fnt(name: &str, pc: &Pc) -> Fnt {
    let mut f = Fnt { name: name.into(), nb: 1, dw: 1000.0, first: 32, widths: pc.widths[32..].to_vec(), measurable: true, encodable: true, ..Default::default() };
    for c in 32..=255u8 {
        if let Some(ch) = cp1252(c) {
            f.dec.insert(c as u32, ch.to_string());
            f.enc.entry(ch).or_insert(c as u32);
        }
    }
    f
}

/// Embeds the font as a simple WinAnsi TrueType font; returns the Font dict object.
pub fn embed_pc(doc: &mut Document, pc: &Pc) -> ObjectId {
    let face = ttf_parser::Face::parse(&pc.bytes, 0).expect("checked in pc_font");
    let k = 1000.0 / face.units_per_em() as f64;
    let bb = face.global_bounding_box();
    let stem: String = pc.file.file_stem().map(|s| s.to_string_lossy().chars().filter(|c| c.is_ascii_alphanumeric()).collect()).unwrap_or_else(|| "PcFont".into());
    let mut ff = Stream::new(dictionary! { "Length1" => pc.bytes.len() as i64 }, pc.bytes.clone());
    let _ = ff.compress();
    let ff_id = doc.add_object(ff);
    let desc = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(stem.clone().into_bytes()),
        "Flags" => 32 + if face.is_italic() { 64 } else { 0 },
        "FontBBox" => vec![(bb.x_min as f64 * k).into(), (bb.y_min as f64 * k).into(), (bb.x_max as f64 * k).into(), (bb.y_max as f64 * k).into()],
        "ItalicAngle" => face.italic_angle() as f64,
        "Ascent" => (face.ascender() as f64 * k).round(),
        "Descent" => (face.descender() as f64 * k).round(),
        "CapHeight" => (face.capital_height().unwrap_or(700) as f64 * k).round(),
        "StemV" => 80,
        "FontFile2" => ff_id,
    });
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "TrueType",
        "BaseFont" => Object::Name(stem.into_bytes()),
        "FirstChar" => 32,
        "LastChar" => 255,
        "Widths" => (32..=255usize).map(|c| Object::Integer(pc.widths[c] as i64)).collect::<Vec<_>>(),
        "FontDescriptor" => desc,
        "Encoding" => "WinAnsiEncoding",
    })
}

/// Codes shown with each font object, across the whole document (glyph availability rule (a)).
pub fn used_codes(doc: &Document) -> HashMap<ObjectId, HashSet<u32>> {
    let mut m: HashMap<ObjectId, HashSet<u32>> = HashMap::new();
    for pid in doc.page_iter() {
        let fonts = load_fonts(doc, pid);
        let Ok(joined) = doc.get_page_content(pid) else { continue };
        for (fname, codes) in super::content::codes_by_font(&joined, &fonts) {
            if let Some(id) = fonts.get(&fname).and_then(|f| f.id) {
                m.entry(id).or_default().extend(codes);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings_roundtrip() {
        for c in 0x20..=0xffu8 {
            if let Some(ch) = cp1252(c) {
                assert_eq!(win_ansi(ch), Some(c), "code {c:#x}");
            }
        }
        assert_eq!(win_ansi('\u{3a9}'), None);
        assert_eq!(glyph_char("uni0041"), Some('A'));
        assert_eq!(glyph_char("endash"), Some('\u{2013}'));
    }

    #[test]
    fn tounicode_parse() {
        let cm = b"1 beginbfchar <0003> <0020> endbfchar 2 beginbfrange <0010> <0012> <0041> <0020> <0021> [<0061> <0062>] endbfrange";
        let m = parse_tounicode(cm);
        assert_eq!(m[&3], " ");
        assert_eq!((m[&0x10].as_str(), m[&0x12].as_str()), ("A", "C"));
        assert_eq!((m[&0x20].as_str(), m[&0x21].as_str()), ("a", "b"));
    }
}
