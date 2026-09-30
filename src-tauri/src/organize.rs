//! Organize pages backend. See _docs/organize-contract.md.
use crate::compress::{self, place, size_of, stamp, temp_root, CANCEL};
use crate::pdf::{self, Opened};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::Emitter;

#[derive(Deserialize)]
pub struct Source {
    path: String,
    password: Option<String>,
}

#[derive(Deserialize)]
pub struct PageReq {
    src: usize,
    page: u32,
    rotation: u32,
}

#[derive(Deserialize)]
pub struct OrganizeRequest {
    sources: Vec<Source>,
    pages: Vec<PageReq>,
    out_mode: String,
}

#[derive(Serialize, Clone)]
pub struct OrganizeProgress {
    page: usize,
    pages: usize,
}

#[derive(Serialize)]
pub struct OrganizeResult {
    output: String,
    pages: usize,
    size: u64,
}

const CANCELLED: &str = "cancelled";

fn validate(pages: &[PageReq], counts: &[u32]) -> Result<(), String> {
    if pages.is_empty() {
        return Err("no pages".into());
    }
    for p in pages {
        let n = *counts.get(p.src).ok_or_else(|| format!("invalid source {}", p.src))?;
        if p.page >= n {
            return Err(format!("page {} out of range for source {} ({n} pages)", p.page, p.src));
        }
        if !matches!(p.rotation, 0 | 90 | 180 | 270) {
            return Err(format!("invalid rotation {}", p.rotation));
        }
    }
    Ok(())
}

fn add_rotation(existing: u32, extra: u32) -> u32 {
    (existing + extra) % 360
}

fn to_rot(deg: u32) -> PdfPageRenderRotation {
    match deg {
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => PdfPageRenderRotation::None,
    }
}

fn do_organize(req: &OrganizeRequest, out_dir: &Path, emit: &dyn Fn(&OrganizeProgress)) -> Result<OrganizeResult, String> {
    let first = req.sources.first().ok_or("no sources")?;
    let mut counts = Vec::new();
    for s in &req.sources {
        let pw = s.password.as_deref().filter(|p| !p.is_empty());
        match pdf::open(Path::new(&s.path), pw) {
            Opened::Ok { pages, .. } => counts.push(pages),
            Opened::NeedsPassword => return Err(format!("password required or wrong: {}", s.path)),
            Opened::Damaged => return Err(format!("pdfium could not open {}", s.path)),
        }
    }
    validate(&req.pages, &counts)?;
    let work = temp_root().join(stamp());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let r = organize_in(req, &work, out_dir, first, emit);
    let _ = std::fs::remove_dir_all(&work);
    r
}

fn organize_in(req: &OrganizeRequest, work: &Path, out_dir: &Path, first: &Source, emit: &dyn Fn(&OrganizeProgress)) -> Result<OrganizeResult, String> {
    let total = req.pages.len();
    let tmp = work.join("organized.pdf");
    {
        let p = pdf::pdfium()?;
        let docs = req
            .sources
            .iter()
            .map(|s| p.load_pdf_from_file(&s.path, s.password.as_deref().filter(|p| !p.is_empty())).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
        for (i, r) in req.pages.iter().enumerate() {
            if CANCEL.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            let idx = i as i32;
            out.pages_mut().copy_page_range_from_document(&docs[r.src], r.page as i32..=r.page as i32, idx).map_err(|e| e.to_string())?;
            let mut page = out.pages().get(idx).map_err(|e| e.to_string())?;
            let existing = page.rotation().map_err(|e| e.to_string())?.as_degrees() as u32;
            page.set_rotation(to_rot(add_rotation(existing, r.rotation)));
            if (i + 1) % 10 == 0 || i + 1 == total {
                emit(&OrganizeProgress { page: i + 1, pages: total });
            }
        }
        out.save_to_file(&tmp).map_err(|e| e.to_string())?;
    }
    // Commit point: past this check the output is published and returned, never reported as cancelled.
    if CANCEL.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    let stem = Path::new(&first.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "organized".into());
    let dest = place(&tmp, out_dir, &stem, "_organized", "pdf")?;
    Ok(OrganizeResult { output: dest.display().to_string(), pages: total, size: size_of(&dest) })
}

#[tauri::command]
pub async fn organize(app: tauri::AppHandle, req: OrganizeRequest) -> Result<OrganizeResult, String> {
    let guard = crate::job::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard; // released only after do_organize (incl. temp cleanup) returns
        let first = req.sources.first().ok_or("no sources")?;
        let dir = if req.out_mode == "folder" {
            compress::folder_dir(&app)
        } else {
            Path::new(&first.path).parent().map(Path::to_path_buf).unwrap_or_default()
        };
        do_organize(&req, &dir, &|p| {
            let _ = app.emit("organize-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn organize_cancel() {
    compress::compress_cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(src: usize, page: u32, rotation: u32) -> PageReq {
        PageReq { src, page, rotation }
    }

    #[test]
    fn rotation_arithmetic() {
        assert_eq!(add_rotation(270, 180), 90);
        assert_eq!(add_rotation(0, 0), 0);
        assert_eq!(add_rotation(90, 270), 0);
    }

    #[test]
    fn page_validation() {
        assert!(validate(&[pr(0, 5, 0), pr(1, 0, 90)], &[6, 2]).is_ok());
        assert!(validate(&[], &[6]).is_err());
        assert!(validate(&[pr(2, 0, 0)], &[6, 2]).is_err());
        assert!(validate(&[pr(0, 6, 0)], &[6]).is_err());
        assert!(validate(&[pr(0, 0, 45)], &[6]).is_err());
    }

    fn find(prefix: &str) -> String {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples");
        std::fs::read_dir(samples)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                let n = p.file_name().unwrap().to_string_lossy().to_string();
                n.starts_with(prefix) && n.ends_with(".pdf") && !n.contains("_compressed")
            })
            .unwrap()
            .display()
            .to_string()
    }

    /// cargo test organize_e2e -- --ignored --nocapture
    #[test]
    #[ignore]
    fn organize_e2e() {
        let s = |path: String| Source { path, password: None };
        let req = OrganizeRequest {
            sources: vec![s(find("signed_")), s(find("table_Bon"))],
            pages: vec![pr(0, 5, 0), pr(1, 0, 90), pr(0, 0, 180), pr(0, 2, 0)],
            out_mode: "next".into(),
        };
        let out = std::env::temp_dir().join("majipdf_organize_e2e");
        let _ = std::fs::remove_dir_all(&out);
        let r = do_organize(&req, &out, &|p| println!("  progress {}/{}", p.page, p.pages)).unwrap();
        println!("output={} size={}", r.output, r.size);
        let p = pdf::pdfium().unwrap();
        let doc = p.load_pdf_from_file(&r.output, None).unwrap();
        println!("pages={}", doc.pages().len());
        assert_eq!(doc.pages().len(), 4);
        for (i, pg) in doc.pages().iter().enumerate() {
            println!("  page {i}: rot={} {}x{}", pg.rotation().unwrap().as_degrees(), pg.width().value, pg.height().value);
        }
    }
}
