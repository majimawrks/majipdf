//! White-out support (Edit 2): pixel removal in image XObjects. Images are decoded with pdfium (pixels only) and written with
//! lopdf as new XObjects, so pdfium never regenerates page content. Contract: _docs/edit2-contract.md ("White-out").
use super::content::{inv, mul, M};
use super::fonts::deref;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use pdfium_render::prelude::*;

/// Decoded pixels of one image placement. `mask`: stencil mask, value >= 128 = painted.
pub struct Pix {
    pub w: u32,
    pub h: u32,
    pub gray: bool,
    pub data: Vec<u8>, // gray: w*h, rgb: w*h*3
}

fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    (0..4).all(|i| (a[i] - b[i]).abs() < 2.0)
}

/// For each wanted bounds (user space x0 y0 x1 y1) the pdfium image object with those bounds, decoded to pixels.
pub fn decode_images(orig: &[u8], idx: u32, want: &[[f64; 4]]) -> Result<Vec<Option<Pix>>, String> {
    let p = crate::pdf::pdfium()?;
    let d = p.load_pdf_from_byte_vec(orig.to_vec(), None).map_err(|e| e.to_string())?;
    let pg = d.pages().get(idx as PdfPageIndex).map_err(|e| e.to_string())?;
    let mut used: Vec<bool> = vec![];
    let mut objs = vec![];
    for o in pg.objects().iter() {
        if o.as_image_object().is_some() {
            let b = o.bounds().ok().map(|q| q.to_rect()).map(|r| [r.left().value as f64, r.bottom().value as f64, r.right().value as f64, r.top().value as f64]);
            objs.push((b, o));
            used.push(false);
        }
    }
    let mut out = vec![];
    for w in want {
        let k = (0..objs.len()).find(|&k| !used[k] && objs[k].0.is_some_and(|b| near(b, *w)));
        let pix = k.and_then(|k| {
            used[k] = true;
            let img = objs[k].1.as_image_object()?.get_raw_image().ok()?;
            let (w, h) = (img.width(), img.height());
            let gray = matches!(img.color(), image::ColorType::L8 | image::ColorType::La8 | image::ColorType::L16 | image::ColorType::La16);
            let data = if gray { img.to_luma8().into_raw() } else { img.to_rgb8().into_raw() };
            Some(Pix { w, h, gray, data })
        });
        out.push(pix);
    }
    Ok(out)
}

/// Paints white (stencil: unpainted) every pixel whose centre lands inside one of `rects` (same frame as `ctm`'s output).
/// Returns how many pixels changed.
pub fn whiten(pix: &mut Pix, ctm: M, rects: &[[f64; 4]], mask: bool) -> usize {
    let Some(ic) = inv(ctm) else { return 0 };
    let (w, h) = (pix.w as usize, pix.h as usize);
    let mut n = 0;
    for r in rects {
        let pts = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].map(|(x, y)| (x * ic[0] + y * ic[2] + ic[4], x * ic[1] + y * ic[3] + ic[5]));
        let (s0, s1) = (pts.iter().map(|p| p.0).fold(f64::MAX, f64::min), pts.iter().map(|p| p.0).fold(f64::MIN, f64::max));
        let (t0, t1) = (pts.iter().map(|p| p.1).fold(f64::MAX, f64::min), pts.iter().map(|p| p.1).fold(f64::MIN, f64::max));
        let (u0, u1) = (((s0 * w as f64).floor() as i64 - 1).clamp(0, w as i64), ((s1 * w as f64).ceil() as i64 + 1).clamp(0, w as i64));
        let (v0, v1) = ((((1.0 - t1) * h as f64).floor() as i64 - 1).clamp(0, h as i64), (((1.0 - t0) * h as f64).ceil() as i64 + 1).clamp(0, h as i64));
        for v in v0..v1 {
            for u in u0..u1 {
                let (s, t) = ((u as f64 + 0.5) / w as f64, 1.0 - (v as f64 + 0.5) / h as f64);
                let (x, y) = (s * ctm[0] + t * ctm[2] + ctm[4], s * ctm[1] + t * ctm[3] + ctm[5]);
                if x >= r[0] && x <= r[2] && y >= r[1] && y <= r[3] {
                    let i = v as usize * w + u as usize;
                    if pix.gray {
                        pix.data[i] = if mask { 0 } else { 255 };
                    } else {
                        pix.data[i * 3..i * 3 + 3].fill(255);
                    }
                    n += 1;
                }
            }
        }
    }
    n
}

fn has_filter(d: &Dictionary, name: &[u8]) -> bool {
    match d.get(b"Filter") {
        Ok(Object::Name(n)) => n == name,
        Ok(Object::Array(a)) => a.iter().any(|o| matches!(o, Object::Name(n) if n == name)),
        _ => false,
    }
}

/// The image XObject of a page resource name, with its dictionary facts.
pub struct Src {
    pub mask: bool,
    pub dct: bool,
    pub smask: Option<ObjectId>,
}

pub fn src_of(d: &Dictionary) -> Src {
    Src { mask: matches!(d.get(b"ImageMask"), Ok(Object::Boolean(true))), dct: has_filter(d, b"DCTDecode"), smask: d.get(b"SMask").ok().and_then(|o| o.as_reference().ok()) }
}

/// Makes the new XObject for whitened pixels. Flate, or DCT q90 when the source was DCT; stencil masks stay 1-bit masks.
pub fn new_image(doc: &mut Document, pix: &Pix, src: &Src, smask: Option<ObjectId>) -> Result<ObjectId, String> {
    let mut d = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => pix.w as i64, "Height" => pix.h as i64 };
    let stream = if src.mask && pix.gray {
        let row = (pix.w as usize).div_ceil(8);
        let mut bits = vec![0xffu8; row * pix.h as usize];
        for v in 0..pix.h as usize {
            for u in 0..pix.w as usize {
                if pix.data[v * pix.w as usize + u] >= 128 {
                    bits[v * row + u / 8] &= !(0x80 >> (u % 8)); // 0 = paint
                }
            }
        }
        d.set("ImageMask", true);
        d.set("BitsPerComponent", 1);
        let mut s = Stream::new(d, bits);
        let _ = s.compress();
        s
    } else {
        d.set("ColorSpace", if pix.gray { "DeviceGray" } else { "DeviceRGB" });
        d.set("BitsPerComponent", 8);
        if let Some(sm) = smask {
            d.set("SMask", Object::Reference(sm));
        }
        if src.dct {
            let mut buf = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90)
                .encode(&pix.data, pix.w, pix.h, if pix.gray { image::ExtendedColorType::L8 } else { image::ExtendedColorType::Rgb8 })
                .map_err(|e| e.to_string())?;
            d.set("Filter", "DCTDecode");
            Stream::new(d, buf)
        } else {
            let mut s = Stream::new(d, pix.data.clone());
            let _ = s.compress();
            s
        }
    };
    Ok(doc.add_object(stream))
}

/// A copy of the soft mask with the placement's rects set opaque (8-bit gray masks of any size); None when it can't be read.
pub fn open_smask(doc: &mut Document, id: ObjectId, ctm: M, rects: &[[f64; 4]]) -> Option<ObjectId> {
    let Object::Stream(s) = doc.get_object(id).ok()?.clone() else { return None };
    let (w, h) = (deref(doc, s.dict.get(b"Width").ok()?).as_i64().ok()? as u32, deref(doc, s.dict.get(b"Height").ok()?).as_i64().ok()? as u32);
    if s.dict.get(b"BitsPerComponent").ok().and_then(|o| o.as_i64().ok()) != Some(8) {
        return None;
    }
    let data = s.decompressed_content().ok()?;
    if data.len() != (w * h) as usize {
        return None;
    }
    let mut pix = Pix { w, h, gray: true, data };
    whiten(&mut pix, ctm, rects, false); // 255 = opaque
    let mut d = s.dict.clone();
    d.remove(b"Filter");
    d.remove(b"DecodeParms");
    d.remove(b"Length");
    let mut ns = Stream::new(d, pix.data);
    let _ = ns.compress();
    Some(doc.add_object(ns))
}

/// Four corners of a rect mapped through `m`, as a closed PDF path (so rotated pages work too).
pub fn poly(r: [f64; 4], m: M) -> String {
    use super::content::fmt;
    let pts = [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].map(|(x, y)| (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5]));
    format!("{} {} m {} {} l {} {} l {} {} l h", fmt(pts[0].0), fmt(pts[0].1), fmt(pts[1].0), fmt(pts[1].1), fmt(pts[2].0), fmt(pts[2].1), fmt(pts[3].0), fmt(pts[3].1))
}

/// Local frame of a placed object (top-left origin, y up) in the display frame (y up), for a rect with top-left (x0, ytop) and display size w x h.
pub fn rot_frame(rot: i64, x0: f64, ytop: f64, w: f64, h: f64) -> M {
    match rot {
        90 => [0.0, -1.0, 1.0, 0.0, x0 + w, ytop],
        180 => [-1.0, 0.0, 0.0, -1.0, x0 + w, ytop - h],
        270 => [0.0, 1.0, -1.0, 0.0, x0, ytop - h],
        _ => [1.0, 0.0, 0.0, 1.0, x0, ytop],
    }
}

/// RGB + SMask XObjects for a PNG with alpha. Returns the image object.
pub fn png_xobject(doc: &mut Document, png: &[u8]) -> Result<ObjectId, String> {
    let img = image::load_from_memory(png).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    let (mut rgb, mut a) = (Vec::with_capacity((w * h * 3) as usize), Vec::with_capacity((w * h) as usize));
    for p in img.pixels() {
        rgb.extend_from_slice(&p.0[..3]);
        a.push(p[3]);
    }
    let mut d = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8 };
    if a.iter().any(|&x| x < 255) {
        let mut sm = Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 }, a);
        let _ = sm.compress();
        d.set("SMask", Object::Reference(doc.add_object(sm)));
    }
    let mut s = Stream::new(d, rgb);
    let _ = s.compress();
    Ok(doc.add_object(s))
}

#[allow(dead_code)]
pub fn m_mul(a: M, b: M) -> M {
    mul(a, b)
}
