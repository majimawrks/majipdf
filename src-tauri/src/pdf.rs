//! pdfium inspection (pages / encrypted / signed / damaged).
use pdfium_render::prelude::*;
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub(crate) fn pdfium() -> Result<MutexGuard<'static, Pdfium>, String> {
    static P: OnceLock<Result<Mutex<Pdfium>, String>> = OnceLock::new();
    let r = P.get_or_init(|| {
        let dll = crate::runtime::path("pdfium.dll")?;
        Pdfium::bind_to_library(&dll)
            .map(|b| Mutex::new(Pdfium::new(b)))
            .map_err(|e| format!("cannot load {}: {e}", dll.display()))
    });
    match r {
        Ok(m) => Ok(m.lock().unwrap_or_else(|e| e.into_inner())),
        Err(e) => Err(e.clone()),
    }
}

/// %PDF- can be preceded by junk bytes; check within the first 1 KB (per spec).
pub fn looks_like_pdf(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else { return false };
    let mut buf = [0u8; 1024];
    let Ok(n) = file.read(&mut buf) else { return false };
    buf[..n].windows(5).any(|w| w == b"%PDF-")
}

pub enum Opened {
    Ok { pages: u32, signed: bool },
    NeedsPassword,
    Damaged,
}

pub fn open(path: &Path, password: Option<&str>) -> Opened {
    let Ok(p) = pdfium() else { return Opened::Damaged };
    let r = match p.load_pdf_from_file(path, password) {
        Ok(doc) => Opened::Ok { pages: doc.pages().len() as u32, signed: doc.signatures().len() > 0 },
        Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError)) => Opened::NeedsPassword,
        Err(_) => Opened::Damaged,
    };
    r
}

#[derive(Serialize)]
pub struct FileInfo {
    path: String,
    name: String,
    size_bytes: u64,
    is_pdf: bool,
    pages: Option<u32>,
    encrypted: bool,
    signed: bool,
    damaged: bool,
    scanned: bool,
}

pub fn info(path: &Path, shown: String) -> Option<FileInfo> {
    let meta = std::fs::metadata(path).ok()?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| shown.clone());
    let is_pdf = looks_like_pdf(path);
    let (mut pages, mut encrypted, mut signed, mut damaged, mut scanned) = (None, false, false, false, false);
    if is_pdf {
        match open(path, None) {
            Opened::Ok { pages: n, signed: s } => {
                (pages, signed) = (Some(n), s);
                scanned = is_scanned(path, None);
            }
            Opened::NeedsPassword => encrypted = true,
            Opened::Damaged => damaged = true,
        }
    }
    Some(FileInfo { path: shown, name, size_bytes: meta.len(), is_pdf, pages, encrypted, signed, damaged, scanned })
}

/// Symlink / junction / other reparse point: never followed when expanding folders or sweeping temp.
pub(crate) fn is_reparse(md: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::MetadataExt::file_attributes(md) & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        md.file_type().is_symlink()
    }
}

const MAX_DEPTH: usize = 12;
const MAX_PDFS: usize = 2000;

/// Directories expand (iteratively, in name order) to their `.pdf` files; junctions/symlinks are skipped,
/// depth is capped at 12 and `out` at 2,000 files.
// ponytail: hitting a cap just stops silently (no `truncated` flag); add one to FileInfo's caller if users hit it.
pub fn expand(path: &Path, out: &mut Vec<PathBuf>) {
    if !path.is_dir() {
        out.push(path.to_path_buf());
        return;
    }
    // Stack of (path, depth, is_dir); children are pushed reversed so pops come out in name order.
    let mut stack = vec![(path.to_path_buf(), 0usize, true)];
    while let Some((p, depth, is_dir)) = stack.pop() {
        if !is_dir {
            if out.len() >= MAX_PDFS {
                return;
            }
            out.push(p);
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&p) else { continue };
        let mut kids: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        kids.sort();
        for k in kids.into_iter().rev() {
            let Ok(md) = std::fs::symlink_metadata(&k) else { continue };
            if is_reparse(&md) {
                continue;
            }
            if md.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((k, depth + 1, true));
                }
            } else if k.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                stack.push((k, depth, false));
            }
        }
    }
}

/// Image-dominant page = image objects cover >= 80 % of it (also true for scans with an invisible OCR layer).
pub(crate) const IMAGE_DOMINANT_PCT: f64 = 80.0;

/// Scanned when more than half of the sampled pages (first 3) are image-dominant.
pub(crate) fn scanned_from(coverage_pct: &[f64]) -> bool {
    let n = coverage_pct.iter().filter(|c| **c >= IMAGE_DOMINANT_PCT).count();
    2 * n > coverage_pct.len()
}

/// false when the file cannot be opened (encrypted without/with wrong password, damaged).
pub fn is_scanned(path: &Path, password: Option<&str>) -> bool {
    let Ok(p) = pdfium() else { return false };
    let Ok(doc) = p.load_pdf_from_file(path, password) else { return false };
    let cov: Vec<f64> = doc
        .pages()
        .iter()
        .take(3)
        .map(|page| {
            let (w, h) = (page.width().value as f64, page.height().value as f64);
            let area: f64 = page
                .objects()
                .iter()
                .filter(|o| o.object_type() == PdfPageObjectType::Image)
                .filter_map(|o| o.bounds().ok())
                .map(|b| {
                    let r = b.to_rect();
                    let dx = (r.right().value as f64).min(w) - (r.left().value as f64).max(0.0);
                    let dy = (r.top().value as f64).min(h) - (r.bottom().value as f64).max(0.0);
                    dx.max(0.0) * dy.max(0.0)
                })
                .sum();
            (area / (w * h).max(1.0) * 100.0).min(100.0)
        })
        .collect();
    scanned_from(&cov)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanned_classification() {
        assert!(scanned_from(&[100.0, 99.0, 100.0])); // pure scan
        assert!(scanned_from(&[95.0, 90.0, 10.0])); // 2 of 3
        assert!(!scanned_from(&[95.0, 3.0, 0.0])); // digital with a full-page first image
        assert!(!scanned_from(&[0.0, 2.5, 1.0])); // digital
        assert!(scanned_from(&[80.0])); // threshold inclusive
        assert!(!scanned_from(&[79.9]));
        assert!(!scanned_from(&[])); // nothing sampled
    }

    #[test]
    fn expand_caps_and_junctions() {
        let root = std::env::temp_dir().join(format!("majipdf_test_expand_{}", crate::compress::stamp()));
        let sub = root.join("a");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("x.pdf"), b"").unwrap();
        std::fs::write(root.join("note.txt"), b"").unwrap();
        std::fs::write(sub.join("y.PDF"), b"").unwrap();
        let mut v = Vec::new();
        expand(&root, &mut v);
        assert_eq!(v, vec![sub.join("y.PDF"), root.join("x.pdf")]);
        // junction back to the root: must not be followed
        #[cfg(windows)]
        {
            let cmd = crate::compress::system_exe(r"System32\cmd.exe");
            let mut c = std::process::Command::new(cmd);
            std::os::windows::process::CommandExt::raw_arg(&mut c, format!("/C mklink /J \"{}\" \"{}\"", sub.join("loop").display(), root.display()));
            let st = c.output().unwrap();
            assert!(st.status.success(), "mklink failed {:?} -> {:?}: {} {}", sub.join("loop"), root, String::from_utf8_lossy(&st.stdout), String::from_utf8_lossy(&st.stderr));
            let mut v = Vec::new();
            expand(&root, &mut v);
            assert_eq!(v.len(), 2);
            std::fs::remove_dir(sub.join("loop")).unwrap(); // removes only the junction
        }
        // depth cap
        let mut deep = root.join("d");
        for i in 0..15 {
            deep = deep.join(format!("l{i}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.pdf"), b"").unwrap();
        let mut v = Vec::new();
        expand(&root, &mut v);
        assert!(!v.iter().any(|p| p.ends_with("deep.pdf")));
        // count cap
        for i in 0..MAX_PDFS + 50 {
            std::fs::write(sub.join(format!("f{i:05}.pdf")), b"").unwrap();
        }
        let mut v = Vec::new();
        expand(&root, &mut v);
        assert_eq!(v.len(), MAX_PDFS);
        let _ = std::fs::remove_dir_all(&root);
    }
}
