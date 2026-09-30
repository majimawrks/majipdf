//! Merge tool backend + shared page thumbnails. See _docs/merge-contract.md.
use crate::compress::{self, place, run_gs, size_of, stamp, temp_root, GsErr, CANCEL};
use crate::pdf::{self, Opened};
use base64::Engine;
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::Emitter;

#[derive(Deserialize)]
pub struct MergeFile {
    path: String,
    password: Option<String>,
}

#[derive(Deserialize)]
pub struct MergeRequest {
    files: Vec<MergeFile>,
    output_name: String,
    compress: bool,
    out_mode: String,
}

#[derive(Serialize, Clone)]
pub struct MergeProgress {
    stage: &'static str,
    page: u32,
    pages: u32,
}

#[derive(Serialize)]
pub struct MergeResult {
    output: String,
    pages: u32,
    size: u64,
    files: usize,
}

/// Strips `\/:*?"<>|` and control chars, then trailing dots/spaces; empty -> "merged".
fn sanitize(name: &str) -> String {
    let s: String = name.chars().filter(|c| !"\\/:*?\"<>|".contains(*c) && !c.is_control()).collect();
    let s: String = s.trim_start().chars().take(150).collect();
    let s = s.trim_end_matches(['.', ' ']);
    if s.is_empty() {
        return "merged".into();
    }
    // Reserved Windows device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9), also before an extension, get a `_`.
    let base = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| base.strip_prefix(p).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")));
    if reserved { format!("{s}_") } else { s.into() }
}

const CANCELLED: &str = "cancelled";

fn name_of(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.into())
}

fn do_merge(req: &MergeRequest, out_dir: &Path, emit: &dyn Fn(&MergeProgress)) -> Result<MergeResult, String> {
    if req.files.len() < 2 {
        return Err("need at least 2 files".into());
    }
    // Pre-pass: validate every input and learn the total page count.
    let mut total = 0;
    for f in &req.files {
        let pw = f.password.as_deref().filter(|p| !p.is_empty());
        match pdf::open(Path::new(&f.path), pw) {
            Opened::Ok { pages, .. } => total += pages,
            Opened::NeedsPassword => return Err(format!("{}: password required or wrong", name_of(&f.path))),
            Opened::Damaged => return Err(format!("{}: pdfium could not open the file", name_of(&f.path))),
        }
    }
    let work = temp_root().join(stamp());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let r = merge_in(req, out_dir, &work, total, emit);
    let _ = std::fs::remove_dir_all(&work);
    r
}

fn merge_in(req: &MergeRequest, out_dir: &Path, work: &Path, total: u32, emit: &dyn Fn(&MergeProgress)) -> Result<MergeResult, String> {
    let merged = work.join("merged.pdf");
    {
        let p = pdf::pdfium()?;
        let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
        let mut done = 0;
        emit(&MergeProgress { stage: "merging", page: 0, pages: total });
        for f in &req.files {
            if CANCEL.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            let pw = f.password.as_deref().filter(|p| !p.is_empty());
            let src = p.load_pdf_from_file(&f.path, pw).map_err(|e| format!("{}: {e}", name_of(&f.path)))?;
            out.pages_mut().append(&src).map_err(|e| format!("{}: {e}", name_of(&f.path)))?;
            done += src.pages().len() as u32;
            emit(&MergeProgress { stage: "merging", page: done, pages: total });
        }
        if CANCEL.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        out.save_to_file(&merged).map_err(|e| e.to_string())?;
    }
    let mut best = merged.clone();
    if req.compress {
        let small = work.join("small.pdf");
        let (preset, dpi) = compress::preset_of("smallest");
        let args = compress::gs_args(preset, dpi, false, None, &merged, &small);
        emit(&MergeProgress { stage: "compressing", page: 0, pages: total });
        match run_gs(&args, |page| emit(&MergeProgress { stage: "compressing", page, pages: total })) {
            Err(GsErr::Cancelled) => return Err(CANCELLED.into()),
            // ponytail: a failed compress pass just keeps the uncompressed merge.
            Err(GsErr::Failed(_)) => {}
            Ok(()) => {
                let s = size_of(&small);
                if s > 0 && s < size_of(&merged) {
                    best = small;
                }
            }
        }
    }
    // Commit point: past this check the output is published and returned, never reported as cancelled.
    if CANCEL.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    let out = place(&best, out_dir, &sanitize(&req.output_name), "", "pdf")?;
    Ok(MergeResult { output: out.display().to_string(), pages: total, size: size_of(&out), files: req.files.len() })
}

#[tauri::command]
pub async fn merge(app: tauri::AppHandle, req: MergeRequest) -> Result<MergeResult, String> {
    let guard = crate::job::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard; // released only after do_merge (incl. temp cleanup) returns
        let dir = if req.out_mode == "folder" {
            compress::folder_dir(&app)
        } else {
            req.files.first().and_then(|f| Path::new(&f.path).parent().map(Path::to_path_buf)).unwrap_or_default()
        };
        do_merge(&req, &dir, &|p| {
            let _ = app.emit("merge-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn merge_cancel() {
    compress::compress_cancel();
}

/// Page `page` (0-based) as a `data:image/jpeg;base64,` URL, `width` px wide.
#[tauri::command]
pub async fn thumbnail(path: String, password: Option<String>, page: u32, width: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || render_thumb(Path::new(&path), password.as_deref().filter(|p| !p.is_empty()), page, width))
        .await
        .map_err(|e| e.to_string())?
}

fn render_thumb(path: &Path, password: Option<&str>, page: u32, width: u32) -> Result<String, String> {
    let p = pdf::pdfium()?;
    let doc = p.load_pdf_from_file(path, password).map_err(|e| e.to_string())?;
    let pg = doc.pages().get(page.try_into().map_err(|_| "bad page".to_string())?).map_err(|e| e.to_string())?;
    let img = pg
        .render_with_config(&PdfRenderConfig::new().set_target_width(width.clamp(16, 1200) as i32))
        .map_err(|e| e.to_string())?
        .as_image()
        .map_err(|e| e.to_string())?
        .to_rgb8();
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80).encode_image(&img).map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(buf)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize("a/b:c*"), "abc");
        assert_eq!(sanitize("  "), "merged");
        assert_eq!(sanitize("report. . "), "report");
        assert_eq!(sanitize("ok name"), "ok name");
        assert_eq!(sanitize("con"), "con_");
        assert_eq!(sanitize("NUL.txt"), "NUL.txt_");
        assert_eq!(sanitize("Com1"), "Com1_");
        assert_eq!(sanitize("lpt9. "), "lpt9_");
        assert_eq!(sanitize("com10"), "com10");
        assert_eq!(sanitize("console"), "console");
        assert_eq!(sanitize(&"x".repeat(300)).len(), 150);
    }

    #[test]
    fn collision_reuses_helper() {
        let taken = [PathBuf::from("d/m.pdf"), PathBuf::from("d/m (2).pdf")];
        let p = compress::output_name(Path::new("d"), "m", "", "pdf", |p| taken.contains(&p.to_path_buf()));
        assert_eq!(p, PathBuf::from("d/m (3).pdf"));
    }

    /// cargo test merge_e2e -- --ignored --nocapture
    #[test]
    #[ignore]
    fn merge_e2e() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples");
        let pw = std::fs::read_to_string(samples.join("password.txt")).unwrap().trim().to_string();
        let find = |pre: &str| -> String {
            std::fs::read_dir(&samples)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .find(|p| {
                    let n = p.file_name().unwrap().to_string_lossy().to_string();
                    n.starts_with(pre) && n.ends_with(".pdf") && !n.contains("_compressed")
                })
                .unwrap()
                .display()
                .to_string()
        };
        let out = std::env::temp_dir().join("majipdf_merge_e2e");
        let _ = std::fs::remove_dir_all(&out);
        for c in [false, true] {
            let req = MergeRequest {
                files: vec![
                    MergeFile { path: find("letter_"), password: None },
                    MergeFile { path: find("table_Bon"), password: None },
                    MergeFile { path: find("locked_"), password: Some(pw.clone()) },
                ],
                output_name: "merged".into(),
                compress: c,
                out_mode: "next".into(),
            };
            let r = do_merge(&req, &out, &|p| println!("  {} {}/{}", p.stage, p.page, p.pages)).unwrap();
            println!("compress={c}: pages={} size={} files={} output={}", r.pages, r.size, r.files, r.output);
            assert_eq!(r.pages, 3);
        }
        let t = render_thumb(Path::new(&find("signed_")), None, 0, 200).unwrap();
        println!("thumb len={} head={}", t.len(), &t[..40]);
    }
}
