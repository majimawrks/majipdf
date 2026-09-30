//! Split tool backend. See _docs/split-contract.md.
use crate::compress::{self, size_of, stamp, temp_root, CANCEL};
use crate::pdf::{self, Opened};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::Emitter;

#[derive(Deserialize)]
pub struct Group {
    from: u32,
    to: u32,
}

#[derive(Deserialize)]
pub struct SplitRequest {
    path: String,
    password: Option<String>,
    groups: Vec<Group>,
    out_mode: String,
}

#[derive(Serialize, Clone)]
pub struct SplitProgress {
    index: usize,
    total: usize,
}

#[derive(Serialize)]
pub struct Output {
    path: String,
    from: u32,
    to: u32,
    size: u64,
}

#[derive(Serialize)]
pub struct SplitResult {
    folder: String,
    outputs: Vec<Output>,
}

const CANCELLED: &str = "cancelled";

fn validate(groups: &[Group], pages: u32) -> Result<(), String> {
    if groups.is_empty() {
        return Err("no groups".into());
    }
    for g in groups {
        if g.from < 1 || g.from > g.to || g.to > pages {
            return Err(format!("invalid range {}-{} (document has {pages} pages)", g.from, g.to));
        }
    }
    Ok(())
}

/// Part paths for `n` parts; the stem gets "", " (2)", " (3)"... until none of them exist.
fn part_paths(dir: &Path, stem: &str, n: usize, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut k = 1;
    loop {
        let suffix = if k == 1 { String::new() } else { format!(" ({k})") };
        let paths: Vec<PathBuf> = (1..=n).map(|i| dir.join(format!("{stem}{suffix}_part{i}.pdf"))).collect();
        if !paths.iter().any(|p| exists(p)) {
            return paths;
        }
        k += 1;
    }
}

/// Removes files this run claimed.
fn rollback(claimed: &[PathBuf]) {
    for p in claimed {
        let _ = std::fs::remove_file(p);
    }
}

fn do_split(req: &SplitRequest, out_dir: &Path, emit: &dyn Fn(&SplitProgress)) -> Result<SplitResult, String> {
    let src_path = Path::new(&req.path);
    let pw = req.password.as_deref().filter(|p| !p.is_empty());
    let pages = match pdf::open(src_path, pw) {
        Opened::Ok { pages, .. } => pages,
        Opened::NeedsPassword => return Err("password required or wrong".into()),
        Opened::Damaged => return Err("pdfium could not open the file".into()),
    };
    validate(&req.groups, pages)?;
    let work = temp_root().join(stamp());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let r = split_in(req, src_path, pw, out_dir, &work, emit);
    let _ = std::fs::remove_dir_all(&work);
    r
}

fn split_in(req: &SplitRequest, src_path: &Path, pw: Option<&str>, out_dir: &Path, work: &Path, emit: &dyn Fn(&SplitProgress)) -> Result<SplitResult, String> {
    let total = req.groups.len();
    let mut temps = Vec::new();
    {
        let p = pdf::pdfium()?;
        let src = p.load_pdf_from_file(src_path, pw).map_err(|e| e.to_string())?;
        for (i, g) in req.groups.iter().enumerate() {
            if CANCEL.load(Ordering::SeqCst) {
                return Err(CANCELLED.into());
            }
            let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
            out.pages_mut().copy_page_range_from_document(&src, (g.from as i32 - 1)..=(g.to as i32 - 1), 0).map_err(|e| e.to_string())?;
            let t = work.join(format!("part{}.pdf", i + 1));
            out.save_to_file(&t).map_err(|e| e.to_string())?;
            temps.push(t);
            emit(&SplitProgress { index: i + 1, total });
        }
    }
    if CANCEL.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    let stem = src_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "split".into());
    // Publish: first claim every destination no-clobber (create_new); a name taken meanwhile releases our
    // claims and retries the set on the next " (n)" suffix. Then fill the claimed files. Cancel is honoured
    // between fills; a cancelled/failed run removes ONLY the files it claimed.
    let mut claimed: Vec<PathBuf> = Vec::new();
    let mut dests = Vec::new();
    for _ in 0..50 {
        dests = part_paths(out_dir, &stem, total, |p| p.exists());
        claimed.clear();
        let mut clash = false;
        for d in &dests {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(d) {
                Ok(_) => claimed.push(d.clone()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    clash = true;
                    break;
                }
                Err(e) => {
                    rollback(&claimed);
                    return Err(format!("cannot write {}: {e}", d.display()));
                }
            }
        }
        if !clash {
            break;
        }
        rollback(&claimed);
        claimed.clear();
    }
    if claimed.len() != total {
        return Err("output names kept colliding".into());
    }
    for (t, d) in temps.iter().zip(&dests) {
        // Commit point is the claim above; from here a cancel still rolls the whole (all-or-nothing) set back.
        if CANCEL.load(Ordering::SeqCst) {
            rollback(&claimed);
            return Err(CANCELLED.into());
        }
        if let Err(e) = std::fs::copy(t, d) {
            rollback(&claimed);
            return Err(format!("cannot write {}: {e}", d.display()));
        }
    }
    let outputs = dests.iter().zip(&req.groups).map(|(d, g)| Output { path: d.display().to_string(), from: g.from, to: g.to, size: size_of(d) }).collect();
    Ok(SplitResult { folder: out_dir.display().to_string(), outputs })
}

#[tauri::command]
pub async fn split(app: tauri::AppHandle, req: SplitRequest) -> Result<SplitResult, String> {
    let guard = crate::job::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard; // released only after do_split (incl. temp cleanup) returns
        let dir = if req.out_mode == "folder" {
            compress::folder_dir(&app)
        } else {
            Path::new(&req.path).parent().map(Path::to_path_buf).unwrap_or_default()
        };
        do_split(&req, &dir, &|p| {
            let _ = app.emit("split-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn split_cancel() {
    compress::compress_cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(from: u32, to: u32) -> Group {
        Group { from, to }
    }

    #[test]
    fn set_wide_collision() {
        let d = Path::new("d");
        let p = part_paths(d, "a", 3, |_| false);
        assert_eq!(p[0], d.join("a_part1.pdf"));
        assert_eq!(p[2], d.join("a_part3.pdf"));
        let taken = [d.join("a_part2.pdf")];
        let p = part_paths(d, "a", 3, |x| taken.contains(&x.to_path_buf()));
        assert_eq!(p[0], d.join("a (2)_part1.pdf"));
        let taken = [d.join("a_part2.pdf"), d.join("a (2)_part3.pdf")];
        let p = part_paths(d, "a", 3, |x| taken.contains(&x.to_path_buf()));
        assert_eq!(p[0], d.join("a (3)_part1.pdf"));
    }

    #[test]
    fn group_validation() {
        assert!(validate(&[g(1, 2), g(3, 3), g(5, 6)], 6).is_ok());
        assert!(validate(&[], 6).is_err());
        assert!(validate(&[g(0, 1)], 6).is_err());
        assert!(validate(&[g(3, 2)], 6).is_err());
        assert!(validate(&[g(5, 7)], 6).is_err());
    }

    /// cargo test split_e2e -- --ignored --nocapture
    #[test]
    #[ignore]
    fn split_e2e() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples");
        let src = std::fs::read_dir(&samples)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                let n = p.file_name().unwrap().to_string_lossy().to_string();
                n.starts_with("signed_") && n.ends_with(".pdf") && !n.contains("_compressed")
            })
            .unwrap();
        let out = std::env::temp_dir().join("majipdf_split_e2e");
        let _ = std::fs::remove_dir_all(&out);
        let req = SplitRequest { path: src.display().to_string(), password: None, groups: vec![g(1, 2), g(3, 3), g(5, 6)], out_mode: "next".into() };
        for run in 1..=2 {
            let r = do_split(&req, &out, &|p| println!("  progress {}/{}", p.index, p.total)).unwrap();
            println!("run {run}: folder={}", r.folder);
            for (o, want) in r.outputs.iter().zip([2, 1, 2]) {
                let Opened::Ok { pages, .. } = pdf::open(Path::new(&o.path), None) else { panic!("reopen failed") };
                println!("  {} size={} pages={} ({}-{})", o.path, o.size, pages, o.from, o.to);
                assert_eq!(pages, want);
            }
        }
    }
}
