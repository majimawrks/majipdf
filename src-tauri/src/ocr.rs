//! Scan to text (OCR) backend: Ghostscript `pdfocr24` (Tesseract) on the scan pages only. See _docs/ocr-contract.md.
use crate::compress::{self, place, run_gs, size_of, stamp, temp_root, GsErr, CANCEL};
use crate::pdf::{self, Opened};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::Emitter;

const CANCELLED: &str = "cancelled";

#[derive(Deserialize)]
pub struct OcrRequest {
    path: String,
    password: Option<String>,
    out_mode: String,
}

#[derive(Serialize, Clone)]
pub struct OcrProgress {
    stage: &'static str,
    done: u32,
    total: u32,
}

#[derive(Serialize, Debug)]
pub struct OcrResult {
    output: String,
    size: u64,
    seconds: f64,
    ocr_pages: u32,
    pages: u32,
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
    let pw = req.password.as_deref().filter(|p| !p.is_empty());
    emit(&OcrProgress { stage: "preparing", done: 0, total: 0 });
    match pdf::open(src, pw) {
        Opened::Ok { .. } => {}
        Opened::NeedsPassword => return Err("password required or wrong".into()),
        Opened::Damaged => return Err("pdfium could not open the file".into()),
    }

    // 1-3: classify every page, write the scan pages into an unencrypted scan.pdf
    let (scan_pdf, ocr_pdf, small_pdf, tmp) = (work.join("scan.pdf"), work.join("ocr.pdf"), work.join("ocr_small.pdf"), work.join("out.pdf"));
    let (scan, pages) = {
        let p = pdf::pdfium()?;
        let doc = p.load_pdf_from_file(src, pw).map_err(|e| e.to_string())?;
        let scan: Vec<i32> = doc.pages().iter().enumerate().filter(|(_, pg)| pdf::image_coverage(pg) >= pdf::IMAGE_DOMINANT_PCT).map(|(i, _)| i as i32).collect();
        if scan.is_empty() {
            return Err("no_scan_pages".into());
        }
        let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
        for (k, &i) in scan.iter().enumerate() {
            out.pages_mut().copy_page_range_from_document(&doc, i..=i, k as i32).map_err(|e| e.to_string())?;
        }
        out.save_to_file(&scan_pdf).map_err(|e| e.to_string())?;
        (scan, doc.pages().len() as u32)
    };
    let total = scan.len() as u32;
    check_cancel()?;

    // 4: OCR (progress from gs "Page N" lines)
    emit(&OcrProgress { stage: "reading", done: 0, total });
    let mut args: Vec<String> = ["-dNOPAUSE", "-dBATCH", "-sDEVICE=pdfocr24", "-r300", "-sOCRLanguage=ind+eng"].map(String::from).into();
    args.push(format!("-sOutputFile={}", ocr_pdf.display()));
    args.push(scan_pdf.display().to_string());
    run_gs(&args, |n| emit(&OcrProgress { stage: "reading", done: n.min(total), total })).map_err(gs_err)?;

    // 5: Balanced pass (the invisible text survives)
    emit(&OcrProgress { stage: "saving", done: total, total });
    run_gs(&compress::gs_args("ebook", 150, false, None, &ocr_pdf, &small_pdf), |_| {}).map_err(gs_err)?;

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
    // Commit point: past this check the output is published and returned, never reported as cancelled.
    check_cancel()?;
    if size_of(&tmp) == 0 {
        return Err("OCR produced no output".into());
    }
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let dest = place(&tmp, out_dir, &stem, "_ocr", "pdf")?;
    Ok(OcrResult { output: dest.display().to_string(), size: size_of(&dest), seconds: start.elapsed().as_secs_f64(), ocr_pages: total, pages })
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
        let req = |p: &Path| OcrRequest { path: p.display().to_string(), password: None, out_mode: "next".into() };

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
        let _ = std::fs::remove_dir_all(&out);
    }
}
