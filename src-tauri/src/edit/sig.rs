//! Signature and stamp library (Edit 2): PNGs with alpha in `<user data>/majipdf/signatures/`, metadata in `index.json`.
//! Nothing leaves the computer. Contract: _docs/edit2-contract.md ("Signatures & stamps").
use crate::pdf;
use base64::Engine;
use image::{imageops::FilterType, DynamicImage, GenericImageView, RgbaImage};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_ITEMS: usize = 50;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Entry {
    id: String,
    name: String,
    kind: String,
    added: u64, // unix seconds
}

#[derive(Serialize, Clone, Debug)]
pub struct SigItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub w: u32,
    pub h: u32,
    pub data_url: String,
}

pub struct Lib {
    pub dir: PathBuf,
}

/// `%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS, XDG data dir elsewhere.
pub fn default_dir() -> Result<PathBuf, String> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    Ok(base.ok_or("no user data folder")?.join("majipdf").join("signatures"))
}

fn png_bytes(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img.clone()).write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Otsu threshold of the luminance histogram.
fn otsu(hist: &[u64; 256]) -> u8 {
    let total: u64 = hist.iter().sum();
    let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut wb, mut sb, mut best, mut thr) = (0u64, 0.0f64, -1.0f64, 128u8);
    for (t, &c) in hist.iter().enumerate() {
        wb += c;
        if wb == 0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0 {
            break;
        }
        sb += t as f64 * c as f64;
        let (mb, mf) = (sb / wb as f64, (sum - sb) / wf as f64);
        let v = wb as f64 * wf as f64 * (mb - mf) * (mb - mf);
        if v > best {
            best = v;
            thr = t as u8;
        }
    }
    thr
}

fn lum(p: &image::Rgba<u8>) -> f64 {
    0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
}

/// Lighter than the threshold (clamped 170..235) becomes transparent, with a 15-level soft edge; the ink colour stays.
pub fn remove_background(img: &mut RgbaImage) {
    let mut hist = [0u64; 256];
    img.pixels().filter(|p| p[3] > 0).for_each(|p| hist[lum(p) as usize] += 1);
    let raw = otsu(&hist);
    let thr = raw.clamp(170, 235) as f64;
    let (mut dark, mut all) = (0u64, 0u64);
    for p in img.pixels().filter(|p| p[3] > 0) {
        all += 1;
        dark += (lum(p) < thr) as u64;
    }
    if dark * 2 > all {
        // dark background (white ink on black, a stamp on dark paper): the lighter side is the ink
        let thr = raw.clamp(20, 235) as f64;
        for p in img.pixels_mut() {
            let a = ((lum(p) - thr) / 15.0).clamp(0.0, 1.0);
            p[3] = (p[3] as f64 * a).round() as u8;
        }
        return;
    }
    for p in img.pixels_mut() {
        let a = ((thr - lum(p)) / 15.0).clamp(0.0, 1.0);
        p[3] = (p[3] as f64 * a).round() as u8;
    }
}

/// Crops to the ink bbox plus 4 px and scales the longest side down to 1200 px.
pub fn crop_and_fit(img: &RgbaImage) -> Result<RgbaImage, String> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 8 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 == u32::MAX {
        return Err("empty".into());
    }
    let (x0, y0) = (x0.saturating_sub(4), y0.saturating_sub(4));
    let (x1, y1) = ((x1 + 5).min(img.width()), (y1 + 5).min(img.height()));
    let mut c = image::imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image();
    let long = c.width().max(c.height());
    if long > 1200 {
        let k = 1200.0 / long as f64;
        c = image::imageops::resize(&c, ((c.width() as f64 * k).round() as u32).max(1), ((c.height() as f64 * k).round() as u32).max(1), FilterType::Lanczos3);
    }
    Ok(c)
}

fn load_source(path: &Path) -> Result<RgbaImage, String> {
    let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        let p = pdf::pdfium()?;
        let d = p.load_pdf_from_file(path, None).map_err(|e| e.to_string())?;
        let pg = d.pages().get(0).map_err(|e| e.to_string())?;
        return Ok(pg.render_with_config(&PdfRenderConfig::new().scale_page_by_factor(300.0 / 72.0)).map_err(|e| e.to_string())?.as_image().map_err(|e| e.to_string())?.to_rgba8());
    }
    Ok(image::open(path).map_err(|e| format!("cannot read the image: {e}"))?.to_rgba8())
}

fn thumb(png: &[u8]) -> Result<(u32, u32, String), String> {
    let img = image::load_from_memory(png).map_err(|e| e.to_string())?;
    let (w, h) = img.dimensions();
    let t = if w.max(h) > 240 { img.resize(240, 240, FilterType::Triangle) } else { img };
    let mut out = std::io::Cursor::new(Vec::new());
    t.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok((w, h, format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out.into_inner()))))
}

impl Lib {
    fn index(&self) -> Vec<Entry> {
        std::fs::read(self.dir.join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save_index(&self, v: &[Entry]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let tmp = self.dir.join("index.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.dir.join("index.json")).map_err(|e| e.to_string())
    }

    pub fn png(&self, id: &str) -> Option<Vec<u8>> {
        (id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| std::fs::read(self.dir.join(format!("{id}.png"))).ok()).flatten()
    }

    /// Thumbnail data URL and original size; the thumbnail is written once (<id>.thumb.png) and read back after that.
    fn item(&self, e: &Entry) -> Option<SigItem> {
        let tp = self.dir.join(format!("{}.thumb.png", e.id));
        let png = self.png(&e.id)?;
        let (w, h) = image::ImageReader::new(std::io::Cursor::new(&png)).with_guessed_format().ok()?.into_dimensions().ok()?;
        let data_url = match std::fs::read(&tp) {
            Ok(t) => format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(t)),
            Err(_) => {
                let (_, _, u) = thumb(&png).ok()?;
                if let Some(b) = u.split_once(',').and_then(|x| base64::engine::general_purpose::STANDARD.decode(x.1).ok()) {
                    let _ = std::fs::write(&tp, b);
                }
                u
            }
        };
        Some(SigItem { id: e.id.clone(), name: e.name.clone(), kind: e.kind.clone(), w, h, data_url })
    }

    pub fn list(&self) -> Vec<SigItem> {
        self.index().iter().filter_map(|e| self.item(e)).collect()
    }

    pub fn kind_of(&self, id: &str) -> Option<String> {
        self.index().into_iter().find(|e| e.id == id).map(|e| e.kind)
    }

    fn add(&self, img: &RgbaImage, name: &str, kind: &str) -> Result<SigItem, String> {
        let bytes = png_bytes(img)?;
        let id: String = Sha256::digest(&bytes).iter().take(8).map(|b| format!("{b:02x}")).collect();
        let mut idx = self.index();
        if let Some(e) = idx.iter().find(|e| e.id == id) {
            return self.item(e).ok_or_else(|| "library file missing".to_string());
        }
        if idx.len() >= MAX_ITEMS {
            return Err("library_full".into());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join(format!("{id}.png")), &bytes).map_err(|e| e.to_string())?;
        let kind = if kind == "stamp" { "stamp" } else { "signature" };
        let name = name.trim();
        let e = Entry {
            id,
            name: if name.is_empty() { "Signature".into() } else { name.chars().take(60).collect() },
            kind: kind.into(),
            added: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        };
        idx.push(e.clone());
        self.save_index(&idx)?;
        self.item(&e).ok_or_else(|| "library file missing".to_string())
    }

    pub fn import(&self, path: &Path, kind: &str) -> Result<SigItem, String> {
        let mut img = load_source(path)?;
        remove_background(&mut img);
        let img = crop_and_fit(&img)?;
        let name = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        self.add(&img, &name, kind)
    }

    pub fn add_drawn(&self, data_url: &str, name: &str) -> Result<SigItem, String> {
        let b64 = data_url.split_once(',').map(|x| x.1).unwrap_or(data_url);
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| e.to_string())?;
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
        self.add(&crop_and_fit(&img)?, name, "signature")
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let mut idx = self.index();
        let e = idx.iter_mut().find(|e| e.id == id).ok_or("no such item")?;
        e.name = name.trim().chars().take(60).collect();
        self.save_index(&idx)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut idx = self.index();
        idx.retain(|e| e.id != id);
        self.save_index(&idx)?;
        if self.png(id).is_some() {
            let _ = std::fs::remove_file(self.dir.join(format!("{id}.png")));
            let _ = std::fs::remove_file(self.dir.join(format!("{id}.thumb.png")));
        }
        Ok(())
    }
}

pub fn lib() -> Result<Lib, String> {
    Ok(Lib { dir: default_dir()? })
}

#[tauri::command]
pub async fn sig_list() -> Result<Vec<SigItem>, String> {
    tauri::async_runtime::spawn_blocking(|| Ok(lib()?.list())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn sig_import(path: String, kind: String) -> Result<SigItem, String> {
    tauri::async_runtime::spawn_blocking(move || lib()?.import(Path::new(&path), &kind)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn sig_add_drawn(data_url: String, name: String) -> Result<SigItem, String> {
    tauri::async_runtime::spawn_blocking(move || lib()?.add_drawn(&data_url, &name)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn sig_rename(id: String, name: String) -> Result<(), String> {
    lib()?.rename(&id, &name)
}

#[tauri::command]
pub fn sig_delete(id: String) -> Result<(), String> {
    lib()?.delete(&id)
}
