//! Compress tool backend: Ghostscript subprocess, presets, size-limit ladder. See _docs/compress-contract.md.
use crate::pdf::{self, Opened};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Deserialize)]
pub struct FileReq {
    path: String,
    password: Option<String>,
}

#[derive(Deserialize)]
pub struct CompressRequest {
    files: Vec<FileReq>,
    mode: String,
    preset: String,
    target_mb: f64,
    grayscale: bool,
    out_mode: String,
}

#[derive(Serialize, Clone, Default)]
pub struct CompressProgress {
    index: usize,
    state: &'static str,
    page: u32,
    pages: u32,
    attempt: Option<u32>,
    dpi: Option<u32>,
    current_size: Option<u64>,
}

#[derive(Serialize)]
pub struct CompressResult {
    path: String,
    before: u64,
    after: Option<u64>,
    output: Option<String>,
    status: &'static str,
    temp: Option<String>,
    error: Option<String>,
}

// ---------- pure logic ----------

/// `<stem><suffix>.<ext>`, then ` (2)`, ` (3)`... while `exists` says the name is taken.
pub fn output_name(dir: &Path, stem: &str, suffix: &str, ext: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let mut p = dir.join(format!("{stem}{suffix}.{ext}"));
    let mut n = 2;
    while exists(&p) {
        p = dir.join(format!("{stem}{suffix} ({n}).{ext}"));
        n += 1;
    }
    p
}

const LADDER: [u32; 6] = [150, 120, 100, 85, 72, 60];
const DPI_72: usize = 4;

/// After attempt `i` produced `size` bytes: index of the next attempt, or None to stop.
pub fn next_attempt(i: usize, size: u64, original: u64, target: u64) -> Option<usize> {
    if size <= target || i + 1 >= LADDER.len() {
        None
    } else if size >= original && i < DPI_72 {
        // ponytail: bigger than the original means a low-DPI/JPEG2000 scan that grows at higher DPI (and takes ages);
        // jump straight to 72 DPI instead of walking the ladder.
        Some(DPI_72)
    } else {
        Some(i + 1)
    }
}

/// Final status given the smallest attempt.
pub fn decide(best: u64, original: u64, target: u64) -> &'static str {
    if best >= original {
        "no_reduction"
    } else if best <= target {
        "done"
    } else {
        "target_missed"
    }
}

pub(crate) fn preset_of(name: &str) -> (&'static str, u32) {
    match name {
        "balanced" => ("ebook", 150),
        "print" => ("printer", 300),
        _ => ("screen", 72),
    }
}

// ---------- gs ----------

pub(crate) static CANCEL: AtomicBool = AtomicBool::new(false);
static CHILD: Mutex<Option<Child>> = Mutex::new(None);

pub(crate) enum GsErr {
    Cancelled,
    Failed(String),
}

pub(crate) fn gs_args(preset: &str, dpi: u32, gray: bool, pw: Option<&str>, input: &Path, output: &Path) -> Vec<String> {
    let mut a: Vec<String> = ["-sDEVICE=pdfwrite", "-dCompatibilityLevel=1.4", "-dNOPAUSE", "-dBATCH"]
        .map(String::from)
        .into();
    a.push(format!("-dPDFSETTINGS=/{preset}"));
    for k in ["Color", "Gray", "Mono"] {
        a.push(format!("-d{k}ImageDownsampleType=/Bicubic"));
        a.push(format!("-d{k}ImageResolution={dpi}"));
        a.push(format!("-dDownsample{k}Images=true"));
        a.push(format!("-d{k}ImageDownsampleThreshold=1.0"));
    }
    a.extend(["-dCompressPages=true", "-dDetectDuplicateImages=true", "-dSubsetFonts=true", "-dEmbedAllFonts=true"].map(String::from));
    if gray {
        a.extend(["-sColorConversionStrategy=Gray", "-dProcessColorModel=/DeviceGray"].map(String::from));
    }
    if let Some(pw) = pw {
        a.push(format!("-sPDFPassword={pw}"));
    }
    a.push(format!("-sOutputFile={}", output.display()));
    a.push(input.display().to_string());
    a
}

/// Runs gs to completion; calls `on_page(n)` for each "Page N" line.
pub(crate) fn run_gs(args: &[String], mut on_page: impl FnMut(u32)) -> Result<(), GsErr> {
    let exe = crate::runtime::path(crate::runtime::GS).map_err(GsErr::Failed)?;
    let mut cmd = Command::new(&exe);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir); // gsdll64.dll resolves from the exe folder
    }
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().map_err(|e| GsErr::Failed(format!("cannot start {}: {e}", exe.display())))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    *CHILD.lock().unwrap() = Some(child);
    if CANCEL.load(Ordering::SeqCst) {
        kill_child(); // cancel arrived between the flag check and spawn
    }
    let err_thread = std::thread::spawn(move || {
        let mut lines: Vec<String> = BufReader::new(stderr).lines().map_while(Result::ok).collect();
        let keep = lines.len().saturating_sub(10);
        lines.split_off(keep).join("\n")
    });
    let mut tail: Vec<String> = Vec::new();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(n) = line.strip_prefix("Page ").and_then(|s| s.trim().parse().ok()) {
            on_page(n);
        } else {
            tail.push(line);
            if tail.len() > 10 {
                tail.remove(0);
            }
        }
    }
    let status = CHILD.lock().unwrap().take().map(|mut c| c.wait());
    let stderr_tail = err_thread.join().unwrap_or_default();
    if CANCEL.load(Ordering::SeqCst) {
        return Err(GsErr::Cancelled);
    }
    match status {
        Some(Ok(s)) if s.success() => Ok(()),
        Some(Ok(s)) => Err(GsErr::Failed(format!("gs exit code {:?}\n{}\n{}", s.code(), tail.join("\n"), stderr_tail))),
        other => Err(GsErr::Failed(format!("gs wait failed: {other:?}"))),
    }
}

fn kill_child() {
    if let Some(c) = CHILD.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}

// ---------- per-file work ----------

pub(crate) fn size_of(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Publishes `from` at `to` and never replaces an existing file: `to` is claimed with `create_new`
/// (fails with AlreadyExists if anything is there), the content is copied in, then `from` is removed.
/// On any failure the claimed (partial) `to` is removed again; it is ours, we just created it.
// ponytail: always copies (no same-volume rename): std::fs::rename replaces on Windows. Add MoveFileExW
// without MOVEFILE_REPLACE_EXISTING if copying huge outputs ever hurts.
pub(crate) fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    if let Err(e) = std::io::copy(&mut src, &mut dst).and_then(|_| dst.sync_all()) {
        drop(dst);
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    drop(dst);
    drop(src);
    std::fs::remove_file(from)
}

pub(crate) fn temp_root() -> PathBuf {
    std::env::temp_dir().join("majipdf")
}

/// `<pid>` of a temp entry: work dirs `<pid>_<nanos>`, results `miss_<pid>_<nanos>.pdf`.
fn owner_pid(name: &str) -> Option<u32> {
    let s = name.strip_prefix("miss_").map(|s| s.strip_suffix(".pdf").unwrap_or(s)).unwrap_or(name);
    let (pid, nanos) = s.split_once('_')?;
    nanos.parse::<u128>().ok()?;
    pid.parse().ok()
}

/// Sweep rule: owner known -> gone AND older than 1 h; unparseable -> older than 24 h.
fn sweepable(name: &str, age: std::time::Duration, live: &std::collections::HashSet<u32>) -> bool {
    const H: u64 = 3600;
    match owner_pid(name) {
        Some(pid) => !live.contains(&pid) && age.as_secs() > H,
        None => age.as_secs() > 24 * H,
    }
}

#[cfg(windows)]
pub(crate) fn system_exe(rel: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into())).join(rel)
}

/// Pids of running processes; None if it cannot be determined (then nothing is swept).
fn live_pids() -> Option<std::collections::HashSet<u32>> {
    #[cfg(windows)]
    let (mut cmd, pid_of) = {
        let mut c = Command::new(system_exe(r"System32\tasklist.exe"));
        c.args(["/FO", "CSV", "/NH"]);
        std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000); // CREATE_NO_WINDOW
        (c, (|l: &str| l.split("\",\"").nth(1)?.trim_matches('"').parse().ok()) as fn(&str) -> Option<u32>)
    };
    #[cfg(not(windows))]
    let (mut cmd, pid_of) = {
        let mut c = Command::new("/bin/ps");
        c.args(["-A", "-o", "pid="]);
        (c, (|l: &str| l.trim().parse().ok()) as fn(&str) -> Option<u32>)
    };
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let mut set: std::collections::HashSet<u32> = String::from_utf8_lossy(&out.stdout).lines().filter_map(pid_of).collect();
    set.insert(std::process::id());
    (set.len() > 1).then_some(set)
}

/// Deletes entries in `temp_root()` whose owning majipdf process is gone (crash/close mid-run work
/// dirs, Compress `miss_*.pdf` never kept or discarded) and that are older than 1 h. Reparse points are skipped.
pub(crate) fn sweep_temp() {
    let Ok(entries) = std::fs::read_dir(temp_root()) else { return };
    let Some(live) = live_pids() else { return };
    for e in entries.flatten() {
        let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
        if pdf::is_reparse(&md) {
            continue;
        }
        let age = md.modified().ok().and_then(|t| t.elapsed().ok()).unwrap_or_default();
        if !sweepable(&e.file_name().to_string_lossy(), age, &live) {
            continue;
        }
        let p = e.path();
        let _ = if md.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
}

pub(crate) fn stamp() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}_{n}", std::process::id())
}

/// Moves `work_file` to a free name in `out_dir` without ever replacing an existing file.
pub(crate) fn place(work_file: &Path, out_dir: &Path, stem: &str, suffix: &str, ext: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    for _ in 0..1000 {
        let out = output_name(out_dir, stem, suffix, ext, |p| p.exists());
        match move_file(work_file, &out) {
            Ok(()) => return Ok(out),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue, // lost a race: next number
            Err(e) => return Err(format!("cannot write {}: {e}", out.display())),
        }
    }
    Err(format!("no free file name in {}", out_dir.display()))
}

/// Err(()) = cancelled. Everything else is reported inside the result.
fn process_file(
    index: usize,
    f: &FileReq,
    req: &CompressRequest,
    out_dir: &Path,
    work: &Path,
    emit: &dyn Fn(&CompressProgress),
) -> Result<CompressResult, ()> {
    let input = Path::new(&f.path);
    let before = size_of(input);
    let mut res = CompressResult { path: f.path.clone(), before, after: None, output: None, status: "failed", temp: None, error: None };
    let pw = f.password.as_deref().filter(|p| !p.is_empty());
    let pages = match pdf::open(input, pw) {
        Opened::Ok { pages, .. } => pages,
        Opened::NeedsPassword => {
            res.error = Some("password required or wrong".into());
            return Ok(res);
        }
        Opened::Damaged => {
            res.error = Some("pdfium could not open the file".into());
            return Ok(res);
        }
    };
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let base = CompressProgress { index, state: "working", pages, ..Default::default() };
    emit(&base);

    let target = (req.target_mb * 1024.0 * 1024.0) as u64;
    let by_target = req.mode == "target";
    if by_target && before <= target {
        res.status = "under_target";
        emit(&CompressProgress { state: "done", page: pages, ..base });
        return Ok(res);
    }

    let mut attempt_no = 0u32;
    let mut best: Option<(PathBuf, u64)> = None;
    let mut step = 0usize; // ladder index (target mode)
    loop {
        attempt_no += 1;
        let (preset, dpi) = if by_target {
            (if LADDER[step] >= 150 { "ebook" } else { "screen" }, LADDER[step])
        } else {
            preset_of(&req.preset)
        };
        let out = work.join(format!("a{attempt_no}.pdf"));
        let prev = best.as_ref().map(|b| b.1);
        let args = gs_args(preset, dpi, req.grayscale, pw, input, &out);
        let r = run_gs(&args, |page| {
            emit(&CompressProgress {
                page,
                attempt: by_target.then_some(attempt_no),
                dpi: by_target.then_some(dpi),
                current_size: prev.filter(|_| by_target),
                ..base.clone()
            })
        });
        match r {
            Err(GsErr::Cancelled) => return Err(()),
            Err(GsErr::Failed(e)) => {
                if best.is_none() {
                    res.error = Some(e);
                    return Ok(res);
                }
                break; // keep what we have from earlier attempts
            }
            Ok(()) => {}
        }
        let size = size_of(&out);
        if size == 0 {
            if best.is_none() {
                res.error = Some("gs produced no output".into());
                return Ok(res);
            }
            break;
        }
        if best.as_ref().map_or(true, |b| size < b.1) {
            best = Some((out, size));
        }
        if !by_target {
            break;
        }
        match next_attempt(step, size, before, target) {
            Some(n) => step = n,
            None => break,
        }
    }

    let Some((best_path, best_size)) = best else { return Ok(res) };
    res.after = Some(best_size);
    let status = if by_target {
        decide(best_size, before, target)
    } else if best_size >= before {
        "no_reduction"
    } else {
        "done"
    };
    res.status = status;
    match status {
        "done" => match place(&best_path, out_dir, &stem, "_compressed", "pdf") {
            Ok(o) => res.output = Some(o.display().to_string()),
            Err(e) => (res.status, res.error) = ("failed", Some(e)),
        },
        "target_missed" => {
            let keep = temp_root().join(format!("miss_{}.pdf", stamp()));
            match move_file(&best_path, &keep) {
                Ok(()) => res.temp = Some(keep.display().to_string()),
                Err(e) => (res.status, res.error) = ("failed", Some(format!("cannot keep temp result: {e}"))),
            }
        }
        _ => {}
    }
    emit(&CompressProgress { state: "done", page: pages, current_size: res.after, ..base });
    Ok(res)
}

fn run_all(
    req: &CompressRequest,
    out_dir_for: &dyn Fn(&Path) -> PathBuf,
    emit: &dyn Fn(&CompressProgress),
) -> Result<Vec<CompressResult>, String> {
    let work = temp_root().join(stamp());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let mut results: Vec<CompressResult> = Vec::new();
    let mut cancelled = false;
    for (i, f) in req.files.iter().enumerate() {
        if CANCEL.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        match process_file(i, f, req, &out_dir_for(Path::new(&f.path)), &work, emit) {
            Ok(r) => results.push(r),
            Err(()) => {
                cancelled = true;
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    if cancelled {
        for t in results.iter().filter_map(|r| r.temp.as_ref()) {
            let _ = std::fs::remove_file(t);
        }
        return Err("cancelled".into());
    }
    Ok(results)
}

pub(crate) fn folder_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path().document_dir().unwrap_or_else(|_| std::env::temp_dir()).join("majipdf")
}

// ---------- commands ----------

#[tauri::command]
pub async fn compress(app: tauri::AppHandle, req: CompressRequest) -> Result<Vec<CompressResult>, String> {
    let guard = crate::job::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard; // released only after run_all (incl. temp cleanup) returns
        let docs = folder_dir(&app);
        let folder = req.out_mode == "folder";
        run_all(
            &req,
            &|orig| if folder { docs.clone() } else { orig.parent().map(Path::to_path_buf).unwrap_or_default() },
            &|p| {
                let _ = app.emit("compress-progress", p);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn compress_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
    kill_child();
}

/// A pending Compress result: a file directly inside `temp_root()` named `miss_*.pdf` (no `..`).
fn pending_result(temp: &str) -> Result<PathBuf, String> {
    let p = Path::new(temp);
    let bad = || format!("not a pending result: {temp}");
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(bad());
    }
    let file = p.canonicalize().map_err(|_| bad())?;
    let root = temp_root().canonicalize().map_err(|_| bad())?;
    let name = file.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if file.parent() != Some(root.as_path()) || !file.is_file() || !name.starts_with("miss_") || !name.ends_with(".pdf") {
        return Err(bad());
    }
    Ok(file)
}

#[tauri::command]
pub fn keep_result(app: tauri::AppHandle, temp: String, original: String, out_mode: String) -> Result<String, String> {
    let temp = pending_result(&temp)?;
    let orig = Path::new(&original);
    let dir = if out_mode == "folder" { folder_dir(&app) } else { orig.parent().map(Path::to_path_buf).unwrap_or_default() };
    let stem = orig.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    place(&temp, &dir, &stem, "_compressed", "pdf").map(|p| p.display().to_string())
}

#[tauri::command]
pub fn discard_result(temp: String) -> Result<(), String> {
    std::fs::remove_file(pending_result(&temp)?).map_err(|e| e.to_string())
}

/// Windows: explorer.exe; macOS: /usr/bin/open.
// ponytail: system file opener instead of tauri-plugin-opener (no new dependency/capability); Linux unsupported.
fn opener() -> Command {
    #[cfg(windows)]
    return Command::new(system_exe("explorer.exe"));
    #[cfg(not(windows))]
    Command::new("/usr/bin/open")
}
#[tauri::command]
pub fn open_path(path: String) {
    let p = Path::new(&path);
    let ok_ext = p.extension().is_some_and(|e| ["pdf", "docx", "xlsx"].iter().any(|x| e.eq_ignore_ascii_case(x)));
    if ok_ext && p.is_file() {
        let _ = opener().arg(p).spawn();
    }
}

#[tauri::command]
pub fn reveal_path(path: String) {
    if path.contains('"') {
        return;
    }
    let mut cmd = opener();
    // explorer only parses /select,"<path>" as one raw arg; Rust auto-quoting breaks paths with spaces.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::raw_arg(&mut cmd, format!("/select,\"{path}\""));
    #[cfg(not(windows))]
    cmd.args(["-R", &path]);
    let _ = cmd.spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_collision() {
        let taken = [PathBuf::from("d/a_compressed.pdf"), PathBuf::from("d/a_compressed (2).pdf")];
        let p = output_name(Path::new("d"), "a", "_compressed", "pdf", |p| taken.contains(&p.to_path_buf()));
        assert_eq!(p, PathBuf::from("d/a_compressed (3).pdf"));
        assert_eq!(output_name(Path::new("d"), "b", "_compressed", "pdf", |_| false), PathBuf::from("d/b_compressed.pdf"));
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("majipdf_test_{tag}_{}", stamp()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn place_never_clobbers() {
        let d = scratch("place");
        std::fs::write(d.join("a_compressed.pdf"), b"OLD").unwrap();
        let w = d.join("work.pdf");
        std::fs::write(&w, b"NEW").unwrap();
        let out = place(&w, &d, "a", "_compressed", "pdf").unwrap();
        assert_eq!(out, d.join("a_compressed (2).pdf"));
        assert_eq!(std::fs::read(d.join("a_compressed.pdf")).unwrap(), b"OLD");
        assert_eq!(std::fs::read(&out).unwrap(), b"NEW");
        assert!(!w.exists());
        // move_file onto an existing file fails and leaves both untouched
        std::fs::write(&w, b"X").unwrap();
        let e = move_file(&w, &out).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&out).unwrap(), b"NEW");
        assert!(w.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn result_path_guard() {
        let root = temp_root();
        std::fs::create_dir_all(&root).unwrap();
        let good = root.join(format!("miss_{}.pdf", stamp()));
        std::fs::write(&good, b"x").unwrap();
        assert!(pending_result(&good.display().to_string()).is_ok());
        let other = root.join(format!("other_{}.pdf", stamp()));
        std::fs::write(&other, b"x").unwrap();
        assert!(pending_result(&other.display().to_string()).is_err()); // not miss_*
        let dotdot = root.join("..").join(good.file_name().unwrap());
        assert!(pending_result(&dotdot.display().to_string()).is_err());
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("miss_x.pdf"), b"x").unwrap();
        assert!(pending_result(&sub.join("miss_x.pdf").display().to_string()).is_err()); // not directly inside
        assert!(discard_result(other.display().to_string()).is_err());
        assert!(other.exists());
        assert!(discard_result(good.display().to_string()).is_ok());
        assert!(!good.exists());
        let _ = std::fs::remove_file(&other);
        let _ = std::fs::remove_dir_all(&sub);
    }

    #[test]
    fn sweep_rules() {
        use std::time::Duration;
        let live: std::collections::HashSet<u32> = [100].into();
        let (old, young) = (Duration::from_secs(7200), Duration::from_secs(60));
        assert_eq!(owner_pid("100_1234567"), Some(100));
        assert_eq!(owner_pid("miss_100_1234567.pdf"), Some(100));
        assert_eq!(owner_pid("junk"), None);
        assert_eq!(owner_pid("a_b"), None);
        assert!(!sweepable("100_5", old, &live)); // owner alive
        assert!(!sweepable("miss_100_5.pdf", old, &live));
        assert!(!sweepable("200_5", young, &live)); // dead but young
        assert!(sweepable("200_5", old, &live));
        assert!(sweepable("miss_200_5.pdf", old, &live));
        assert!(!sweepable("junk", old, &live)); // unparseable: 24 h
        assert!(sweepable("junk", Duration::from_secs(25 * 3600), &live));
    }

    #[test]
    fn ladder() {
        let (orig, target) = (10_000, 3_000);
        assert_eq!(next_attempt(0, 2_900, orig, target), None); // fits at first try
        assert_eq!(next_attempt(0, 8_000, orig, target), Some(1)); // still too big -> next dpi
        assert_eq!(next_attempt(0, 12_000, orig, target), Some(4)); // grew -> jump to 72
        assert_eq!(next_attempt(4, 12_000, orig, target), Some(5)); // already at 72 -> 60
        assert_eq!(next_attempt(5, 8_000, orig, target), None); // ladder exhausted
        assert_eq!(decide(2_900, orig, target), "done");
        assert_eq!(decide(8_000, orig, target), "target_missed");
        assert_eq!(decide(10_000, orig, target), "no_reduction");
    }

    /// cargo test e2e -- --ignored --nocapture
    #[test]
    #[ignore]
    fn e2e() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples");
        let pw = std::fs::read_to_string(samples.join("password.txt")).unwrap().trim().to_string();
        let mut pdfs: Vec<PathBuf> = std::fs::read_dir(&samples)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
            .collect();
        pdfs.sort();
        for p in &pdfs {
            let i = pdf::info(p, p.display().to_string()).unwrap();
            let v = serde_json::to_value(&i).unwrap();
            println!(
                "{:<40.40} size={:<9} pages={:<5} enc={} sig={} dmg={} pdf={}",
                v["name"].as_str().unwrap(), v["size_bytes"], v["pages"], v["encrypted"], v["signed"], v["damaged"], v["is_pdf"]
            );
        }
        let is = |p: &&PathBuf, pre: &str| p.file_name().unwrap().to_string_lossy().starts_with(pre);
        let locked = pdfs.iter().find(|p| is(p, "locked_")).unwrap();
        match pdf::open(locked, Some(&pw)) {
            Opened::Ok { pages, signed } => println!("unlock ok pages={pages} signed={signed}"),
            _ => panic!("unlock failed"),
        }
        assert!(matches!(pdf::open(locked, Some("wrong")), Opened::NeedsPassword));
        let letter = pdfs.iter().find(|p| is(p, "letter_")).unwrap();
        let out = std::env::temp_dir().join("majipdf_e2e_out");
        let _ = std::fs::remove_dir_all(&out);
        for (mode, tmb) in [("preset", 0.0), ("target", 0.02)] {
            let req = CompressRequest {
                files: vec![FileReq { path: letter.display().to_string(), password: None }],
                mode: mode.into(),
                preset: "smallest".into(),
                target_mb: tmb,
                grayscale: false,
                out_mode: "next".into(),
            };
            let events = std::cell::Cell::new(0);
            let r = run_all(&req, &|_| out.clone(), &|p| {
                events.set(events.get() + 1);
                if p.attempt.is_some() {
                    println!("  progress page={} attempt={:?} dpi={:?} size={:?}", p.page, p.attempt, p.dpi, p.current_size)
                }
            })
            .unwrap();
            let r = &r[0];
            println!(
                "{mode}: before={} after={:?} status={} output={:?} temp={:?} err={:?} events={}",
                r.before, r.after, r.status, r.output, r.temp, r.error, events.get()
            );
            if let Some(t) = &r.temp {
                let _ = discard_result(t.clone());
            }
        }
    }
}
