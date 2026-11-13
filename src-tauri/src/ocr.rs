//! Scan to text (OCR) backend: Ghostscript `pdfocr24` (Tesseract) on the scan pages only. See _docs/ocr-contract.md.
use crate::compress::{self, place, run_gs, size_of, stamp, temp_root, GsErr, CANCEL};
use crate::pdf::{self, Opened};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::Emitter;

const CANCELLED: &str = "cancelled";

/// OCR engine: Accurate = PaddleOCR (default), Fast = Tesseract through Ghostscript. See _docs/paddle-contract.md.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    #[default]
    Accurate,
    Fast,
}

#[derive(Deserialize)]
pub struct OcrRequest {
    path: String,
    password: Option<String>,
    out_mode: String,
    #[serde(default)]
    ocr_engine: Engine,
}

#[derive(Serialize, Clone)]
pub struct OcrProgress {
    pub(crate) stage: &'static str,
    pub(crate) done: u32,
    pub(crate) total: u32,
}

#[derive(Serialize, Debug)]
pub struct OcrResult {
    output: String,
    size: u64,
    seconds: f64,
    ocr_pages: u32,
    pages: u32,
    /// Accurate was asked for but could not start, so Fast was used.
    ocr_fallback: bool,
}

fn gs_err(e: GsErr) -> String {
    match e {
        GsErr::Cancelled => CANCELLED.into(),
        GsErr::Failed(m) => m,
    }
}

fn check_cancel() -> Result<(), String> {
    if CANCEL.load(Ordering::SeqCst) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

fn ocr(req: &OcrRequest, out_dir: &Path, emit: &dyn Fn(&OcrProgress)) -> Result<OcrResult, String> {
    let start = std::time::Instant::now();
    let work = temp_root().join(stamp());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let r = ocr_in(req, &work, out_dir, emit, start);
    let _ = std::fs::remove_dir_all(&work);
    r
}

fn ocr_in(req: &OcrRequest, work: &Path, out_dir: &Path, emit: &dyn Fn(&OcrProgress), start: std::time::Instant) -> Result<OcrResult, String> {
    let src = Path::new(&req.path);
    let o = ocr_to(src, req.password.as_deref().filter(|p| !p.is_empty()), work, &|p| emit(&p), false, req.ocr_engine)?;
    // Commit point: past this check the output is published and returned, never reported as cancelled.
    check_cancel()?;
    if size_of(&o.path) == 0 {
        return Err("OCR produced no output".into());
    }
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let dest = place(&o.path, out_dir, &stem, "_ocr", "pdf")?;
    Ok(OcrResult { output: dest.display().to_string(), size: size_of(&dest), seconds: start.elapsed().as_secs_f64(), ocr_pages: o.ocr_pages, pages: o.pages, ocr_fallback: o.fallback })
}

/// Result of [`ocr_to`]: the unencrypted OCR'd copy inside the caller's work dir.
pub(crate) struct OcrOut {
    pub path: PathBuf,
    pub ocr_pages: u32,
    pub pages: u32,
    pub fallback: bool,
}

/// OCRs the scan pages of `src` and writes the whole document to `work/ocr_out.pdf` (unencrypted). Used by
/// the OCR tool and by Word/Excel. No `place()`, no job guard and no CANCEL reset: the caller owns those.
/// `clean` (Excel): the scan pages are rendered at 300 DPI and have their table rules erased before Tesseract sees
/// them (it reads rules as "|"); the OCR text layer is then spliced from the raw OCR output (no ebook pass).
/// `engine`: Accurate (PaddleOCR) writes its own invisible text layer onto the original pages and ignores `clean`; if it
/// cannot start (DLL or model load error) the Fast path below runs and `fallback` is set. macOS: Fast, silently.
/// `Err("no_scan_pages")` when nothing qualifies.
pub(crate) fn ocr_to(src: &Path, password: Option<&str>, work: &Path, emit: &dyn Fn(OcrProgress), clean: bool, engine: Engine) -> Result<OcrOut, String> {
    let pw = password;
    emit(OcrProgress { stage: "preparing", done: 0, total: 0 });
    match pdf::open(src, pw) {
        Opened::Ok { .. } => {}
        Opened::NeedsPassword => return Err("password required or wrong".into()),
        Opened::Damaged => return Err("pdfium could not open the file".into()),
    }

    #[allow(unused_mut)]
    let mut fallback = false;
    #[cfg(not(target_os = "macos"))]
    if engine == Engine::Accurate {
        match accurate(src, pw, work, emit)? {
            Some(o) => return Ok(o),
            None => fallback = true,
        }
    }
    let _ = engine;

    // 1-3: classify every page, write the scan pages into an unencrypted scan.pdf
    let (scan_pdf, ocr_pdf, small_pdf, tmp) = (work.join("scan.pdf"), work.join("ocr.pdf"), work.join("ocr_small.pdf"), work.join("ocr_out.pdf"));
    let (scan, pages) = {
        let p = pdf::pdfium()?;
        let doc = p.load_pdf_from_file(src, pw).map_err(|e| e.to_string())?;
        let scan: Vec<i32> = doc.pages().iter().enumerate().filter(|(_, pg)| pdf::image_coverage(pg) >= pdf::IMAGE_DOMINANT_PCT).map(|(i, _)| i as i32).collect();
        if scan.is_empty() {
            return Err("no_scan_pages".into());
        }
        let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
        for (k, &i) in scan.iter().enumerate() {
            if !clean {
                out.pages_mut().copy_page_range_from_document(&doc, i..=i, k as i32).map_err(|e| e.to_string())?;
                continue;
            }
            check_cancel()?;
            emit(OcrProgress { stage: "preparing", done: k as u32, total: scan.len() as u32 });
            let page = doc.pages().get(i).map_err(|e| e.to_string())?;
            let (pw_pt, ph_pt) = (page.width(), page.height());
            // 300 DPI, capped at ~40 MP so an A0 scan doesn't take gigabytes
            let dpi = 300f32.min((40e6 / (pw_pt.value / 72.0 * ph_pt.value / 72.0).max(1.0)).sqrt());
            let cfg = PdfRenderConfig::new().set_target_width((pw_pt.value * dpi / 72.0) as i32);
            let Some(mut gray) = page.render_with_config(&cfg).ok().and_then(|b| b.as_image().ok()).map(|i| i.to_luma8()) else {
                // can't render this page: OCR it as it is
                out.pages_mut().copy_page_range_from_document(&doc, i..=i, k as i32).map_err(|e| e.to_string())?;
                continue;
            };
            let (w, h) = (gray.width() as usize, gray.height() as usize);
            crate::excel::engine::clean_for_ocr(&mut gray, w, h, dpi);
            let mut pg = out.pages_mut().create_page_at_end(PdfPagePaperSize::new_custom(pw_pt, ph_pt)).map_err(|e| e.to_string())?;
            pg.objects_mut()
                .create_image_object(PdfPoints::ZERO, PdfPoints::ZERO, &image::DynamicImage::ImageLuma8(gray), Some(pw_pt), Some(ph_pt))
                .map_err(|e| e.to_string())?;
        }
        if clean {
            emit(OcrProgress { stage: "preparing", done: scan.len() as u32, total: scan.len() as u32 });
        }
        out.save_to_file(&scan_pdf).map_err(|e| e.to_string())?;
        (scan, doc.pages().len() as u32)
    };
    let total = scan.len() as u32;
    check_cancel()?;

    // 4: OCR (progress from gs "Page N" lines)
    emit(OcrProgress { stage: "reading", done: 0, total });
    let mut args: Vec<String> = ["-dNOPAUSE", "-dBATCH", "-sDEVICE=pdfocr24", "-r300", "-sOCRLanguage=ind+eng"].map(String::from).into();
    args.push(format!("-sOutputFile={}", ocr_pdf.display()));
    args.push(scan_pdf.display().to_string());
    run_gs(&args, |n| emit(OcrProgress { stage: "reading", done: n.min(total), total })).map_err(gs_err)?;

    // 5: Balanced pass (the invisible text survives)
    emit(OcrProgress { stage: "saving", done: total, total });
    if !clean {
        run_gs(&compress::gs_args("ebook", 150, false, None, &ocr_pdf, &small_pdf), |_| {}).map_err(gs_err)?;
    }
    let small_pdf = if clean { ocr_pdf } else { small_pdf };

    // 6: original page order, scan pages from the OCR'd copy, the rest untouched
    {
        let p = pdf::pdfium()?;
        let orig = p.load_pdf_from_file(src, pw).map_err(|e| e.to_string())?;
        let small = p.load_pdf_from_file(&small_pdf, None).map_err(|e| e.to_string())?;
        if small.pages().len() as u32 != total {
            return Err(format!("OCR output has {} pages, expected {total}", small.pages().len()));
        }
        let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
        for i in 0..pages as i32 {
            check_cancel()?;
            let r = match scan.iter().position(|&s| s == i) {
                Some(k) => out.pages_mut().copy_page_range_from_document(&small, k as i32..=k as i32, i),
                None => out.pages_mut().copy_page_range_from_document(&orig, i..=i, i),
            };
            r.map_err(|e| e.to_string())?;
        }
        out.save_to_file(&tmp).map_err(|e| e.to_string())?;
    }
    Ok(OcrOut { path: tmp, ocr_pages: total, pages, fallback })
}

/// Accurate path: pdfium render (300 DPI, 40 MP cap) -> Paddle det + rec -> invisible text on the original pages.
/// `Ok(None)` = Paddle could not start or failed on a page (the caller falls back to Fast); `Err("no_scan_pages")` as for Fast.
#[cfg(not(target_os = "macos"))]
fn accurate(src: &Path, pw: Option<&str>, work: &Path, emit: &dyn Fn(OcrProgress)) -> Result<Option<OcrOut>, String> {
    let (base, tmp) = (work.join("ocr_base.pdf"), work.join("ocr_out.pdf"));
    let (scan, pages) = {
        let p = pdf::pdfium()?;
        let doc = p.load_pdf_from_file(src, pw).map_err(|e| e.to_string())?;
        // scan pages that already carry an OCR layer are skipped: a second invisible layer would double search/copy text
        let scan: Vec<i32> = doc
            .pages()
            .iter()
            .enumerate()
            .filter(|(_, pg)| pdf::image_coverage(pg) >= pdf::IMAGE_DOMINANT_PCT && !pdf::has_ocr_text(pg))
            .map(|(i, _)| i as i32)
            .collect();
        (scan, doc.pages().len() as u32)
    };
    if scan.is_empty() {
        return Err("no_scan_pages".into());
    }
    let total = scan.len() as u32;
    emit(OcrProgress { stage: "reading", done: 0, total });
    let mut engine = match crate::paddle::Paddle::load() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("accurate OCR unavailable: {e}");
            return Ok(None);
        }
    };
    let mut layers = Vec::new();
    for (k, &i) in scan.iter().enumerate() {
        check_cancel()?;
        // pdfium is locked per page only, so thumbnails are not starved for the whole run
        let img = {
            let p = pdf::pdfium()?;
            let doc = p.load_pdf_from_file(src, pw).map_err(|e| e.to_string())?;
            let page = doc.pages().get(i).map_err(|e| e.to_string())?;
            let (pw_pt, ph_pt) = (page.width(), page.height());
            let dpi = 300f32.min((40e6 / (pw_pt.value / 72.0 * ph_pt.value / 72.0).max(1.0)).sqrt());
            let cfg = PdfRenderConfig::new().set_target_width((pw_pt.value * dpi / 72.0) as i32);
            page.render_with_config(&cfg).ok().and_then(|b| b.as_image().ok()).map(|i| i.to_rgb8())
        };
        // a page pdfium cannot render, or any Paddle error mid-run: the whole file goes to Fast (cancel stays cancel)
        let Some(img) = img else {
            eprintln!("accurate OCR: page {} could not be rendered", i + 1);
            return Ok(None);
        };
        let words = match engine.read(&img, &check_cancel) {
            Ok(w) => w,
            Err(e) if e == CANCELLED => return Err(e),
            Err(e) => {
                eprintln!("accurate OCR failed: {e}");
                return Ok(None);
            }
        };
        layers.push((i as u32 + 1, img.width(), img.height(), words));
        emit(OcrProgress { stage: "reading", done: k as u32 + 1, total });
    }
    drop(engine);
    check_cancel()?;
    emit(OcrProgress { stage: "saving", done: total, total });
    // unencrypted copy of the whole document (pdfium), then the text layer on top (lopdf)
    {
        let p = pdf::pdfium()?;
        let orig = p.load_pdf_from_file(src, pw).map_err(|e| e.to_string())?;
        let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
        out.pages_mut().copy_page_range_from_document(&orig, 0..=pages as i32 - 1, 0).map_err(|e| e.to_string())?;
        out.save_to_file(&base).map_err(|e| e.to_string())?;
    }
    let mut doc = lopdf::Document::load(&base).map_err(|e| e.to_string())?;
    crate::ocrlayer::add_layers(&mut doc, &layers)?;
    doc.save(&tmp).map_err(|e| e.to_string())?;
    Ok(Some(OcrOut { path: tmp, ocr_pages: total, pages, fallback: false }))
}

// ---------- commands ----------

#[tauri::command]
pub async fn ocr_run(app: tauri::AppHandle, req: OcrRequest) -> Result<OcrResult, String> {
    let guard = crate::job::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard; // held until ocr() has cleaned up
        let dir = if req.out_mode == "folder" {
            compress::folder_dir(&app)
        } else {
            Path::new(&req.path).parent().map(Path::to_path_buf).unwrap_or_default()
        };
        ocr(&req, &dir, &|p| {
            let _ = app.emit("ocr-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn ocr_cancel() {
    compress::compress_cancel();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples");
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy().to_string();
                n.starts_with(name) && n.ends_with(".pdf") && !n.contains("_compressed") && !n.contains("_merged")
            })
            .collect();
        v.sort();
        v.remove(0)
    }

    fn words(p: &Path, page: i32) -> usize {
        let pdfium = pdf::pdfium().unwrap();
        let doc = pdfium.load_pdf_from_file(p, None).unwrap();
        let n = doc.pages().get(page).unwrap().text().unwrap().all().split_whitespace().count();
        n
    }

    /// cargo test ocr_e2e -- --ignored --nocapture
    #[test]
    #[ignore]
    fn ocr_e2e() {
        CANCEL.store(false, Ordering::SeqCst); // tests call ocr() directly, without job::begin()
        let out = std::env::temp_dir().join("majipdf_ocr_e2e");
        let _ = std::fs::remove_dir_all(&out);
        let req = |p: &Path| OcrRequest { path: p.display().to_string(), password: None, out_mode: "next".into(), ocr_engine: Engine::Fast };

        let letter = sample("letter_");
        assert_eq!(ocr(&req(&letter), &out, &|_| {}).unwrap_err(), "no_scan_pages");

        let scan = sample("scanned_bsc");
        let before = words(&scan, 0);
        let r = ocr(&req(&scan), &out, &|p| println!("  {} {}/{}", p.stage, p.done, p.total)).unwrap();
        println!("scanned: pages={} ocr_pages={} size={} seconds={:.1}", r.pages, r.ocr_pages, r.size, r.seconds);
        let out_pdf = Path::new(&r.output);
        assert_eq!((r.pages, r.ocr_pages), (3, 3));
        assert!(matches!(pdf::open(out_pdf, None), Opened::Ok { pages: 3, .. }));
        for i in 0..3 {
            println!("  page {i}: words {} -> {}", if i == 0 { before } else { 0 }, words(out_pdf, i));
            assert!(words(out_pdf, i) > 0, "page {i} has no text");
        }
        assert!(!r.ocr_fallback);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// cargo test ocr_accurate_e2e -- --ignored --nocapture. Counts and sizes only.
    #[cfg(not(target_os = "macos"))]
    #[test]
    #[ignore]
    fn ocr_accurate_e2e() {
        CANCEL.store(false, Ordering::SeqCst);
        let out = std::env::temp_dir().join("majipdf_ocr_accurate_e2e");
        let _ = std::fs::remove_dir_all(&out);
        let req = |p: &Path| OcrRequest { path: p.display().to_string(), password: None, out_mode: "next".into(), ocr_engine: Engine::Accurate };
        assert_eq!(ocr(&req(&sample("letter_")), &out, &|_| {}).unwrap_err(), "no_scan_pages");

        let scan = sample("scanned_bsc");
        let input = size_of(&scan);
        let r = ocr(&req(&scan), &out, &|p| println!("  {} {}/{}", p.stage, p.done, p.total)).unwrap();
        println!("accurate: pages={} ocr_pages={} input={} output={} seconds={:.1}", r.pages, r.ocr_pages, input, r.size, r.seconds);
        let out_pdf = Path::new(&r.output);
        assert!(!r.ocr_fallback, "Paddle did not start");
        assert_eq!((r.pages, r.ocr_pages), (3, 3));
        assert!(matches!(pdf::open(out_pdf, None), Opened::Ok { pages: 3, .. }));
        for i in 0..3 {
            let n = words(out_pdf, i);
            println!("  page {i}: words {n}");
            assert!(n > 0, "page {i} has no text");
        }
        assert!(r.size <= input + 200 * 1024, "output grew by more than 200 KB");
        // already OCR'd (by us or by Fast): no second layer
        assert_eq!(ocr(&req(out_pdf), &out, &|_| {}).unwrap_err(), "no_scan_pages");
        let fast = ocr(&OcrRequest { ocr_engine: Engine::Fast, ..req(&scan) }, &out, &|_| {}).unwrap();
        assert_eq!(ocr(&req(Path::new(&fast.output)), &out, &|_| {}).unwrap_err(), "no_scan_pages");
        let _ = std::fs::remove_dir_all(&out);
    }
}
