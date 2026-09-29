//! PDF -> Excel backend: own pdfium engine (default, `engine.rs`) or installed Excel Power Query via the embedded
//! PowerShell COM script. See _docs/excel-contract.md.
mod engine;

use crate::compress::{self, place, size_of, stamp, temp_root, CANCEL};
use crate::pdf::{self, Opened};
use crate::word::{hidden, parse_line, prepare_copy, Line};
use engine::{Options, SheetMode, CANCELLED};
use pdfium_render::prelude::{PdfiumError, PdfiumInternalError};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

const SCRIPT: &str = include_str!("../../scripts/pdf2xlsx.ps1");

#[derive(Deserialize)]
pub struct ExcelRequest {
    path: String,
    password: Option<String>,
    engine: String,
    sheet_mode: String,
    numbers: bool,
    out_mode: String,
}

#[derive(Serialize, Clone)]
pub struct ExcelProgress {
    stage: &'static str,
    page: usize,
    pages: usize,
}

#[derive(Serialize, Debug)]
pub struct ExcelResult {
    output: String,
    size: u64,
    seconds: f64,
    pages: usize,
    empty_pages: usize,
}

// ---------- routing ----------

fn convert(req: &ExcelRequest, out_dir: &Path, emit: &dyn Fn(&ExcelProgress)) -> Result<ExcelResult, String> {
    match req.engine.as_str() {
        "own" => run_in_work_dir(req, out_dir, emit, convert_own),
        "excel" => {
            if !crate::office_status().excel {
                return Err("excel_missing".into());
            }
            EXCEL_PID.store(0, Ordering::SeqCst);
            let r = run_in_work_dir(req, out_dir, emit, convert_excel);
            EXCEL_PID.store(0, Ordering::SeqCst);
            r
        }
        other => Err(format!("unknown engine: {other}")),
    }
}

type Job = fn(&ExcelRequest, &Path, &Path, &dyn Fn(&ExcelProgress), std::time::Instant) -> Result<ExcelResult, String>;

/// Every path converts inside a fresh temp dir (Excel `SaveAs` into OneDrive fails; cancel leaves nothing behind).
fn run_in_work_dir(req: &ExcelRequest, out_dir: &Path, emit: &dyn Fn(&ExcelProgress), job: Job) -> Result<ExcelResult, String> {
    CANCEL.store(false, Ordering::SeqCst);
    let start = std::time::Instant::now();
    let work = temp_root().join(stamp());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let r = job(req, &work, out_dir, emit, start);
    let _ = std::fs::remove_dir_all(&work);
    r
}

fn stem_of(req: &ExcelRequest) -> String {
    Path::new(&req.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into())
}

// ---------- own engine ----------

fn convert_own(req: &ExcelRequest, work: &Path, out_dir: &Path, emit: &dyn Fn(&ExcelProgress), start: std::time::Instant) -> Result<ExcelResult, String> {
    let src = Path::new(&req.path);
    let pw = req.password.as_deref().filter(|p| !p.is_empty());
    let output = work.join("out.xlsx");
    let opts = Options { numbers: req.numbers, sheet_mode: if req.sheet_mode == "one" { SheetMode::One } else { SheetMode::PerPage } };
    let stats = {
        let p = pdf::pdfium()?; // the shared instance; held for the run, so thumbnails wait
        let doc = p.load_pdf_from_file(src, pw).map_err(|e| match e {
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => "password required or wrong".to_string(),
            e => e.to_string(),
        })?;
        engine::convert(&doc, &output, &opts, &|stage, page, pages| emit(&ExcelProgress { stage, page, pages }), &CANCEL)?
    };
    let dest = place(&output, out_dir, &stem_of(req), "", "xlsx")?;
    Ok(ExcelResult { output: dest.display().to_string(), size: size_of(&dest), seconds: start.elapsed().as_secs_f64(), pages: stats.pages, empty_pages: stats.empty_pages })
}

// ---------- Excel engine (Power Query v2 script) ----------

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
static EXCEL_PID: AtomicU32 = AtomicU32::new(0);

/// Kills only the EXCEL.EXE the script reported (never the user's own Excel), then the powershell child.
fn kill_all() {
    let pid = EXCEL_PID.load(Ordering::SeqCst);
    if pid != 0 {
        let _ = hidden(Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null())).status();
    }
    if let Some(c) = CHILD.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}

fn convert_excel(req: &ExcelRequest, work: &Path, out_dir: &Path, emit: &dyn Fn(&ExcelProgress), start: std::time::Instant) -> Result<ExcelResult, String> {
    let src = Path::new(&req.path);
    let (input, output, script): (PathBuf, PathBuf, PathBuf) = (work.join("in.pdf"), work.join("out.xlsx"), work.join("pdf2xlsx.ps1"));
    let progress = |stage| emit(&ExcelProgress { stage, page: 0, pages: 0 });
    progress("excel");
    prepare_copy(src, req.password.as_deref(), &input)?;
    let pages = match pdf::open(&input, None) {
        Opened::Ok { pages, .. } => pages as usize,
        _ => 0,
    };
    std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
    if CANCEL.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }

    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .arg("-In")
        .arg(&input)
        .arg("-Out")
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = hidden(&mut cmd).spawn().map_err(|e| format!("cannot start powershell: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    *CHILD.lock().unwrap() = Some(child);
    if CANCEL.load(Ordering::SeqCst) {
        kill_all(); // cancel arrived between the flag check and spawn
    }
    let err_thread = std::thread::spawn(move || BufReader::new(stderr).lines().map_while(Result::ok).collect::<Vec<_>>().join("\n"));

    let mut error_line: Option<String> = None;
    let mut tail: Vec<String> = Vec::new();
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        match parse_line(&line) {
            Line::Pid(n) => {
                EXCEL_PID.store(n, Ordering::SeqCst);
                if CANCEL.load(Ordering::SeqCst) {
                    kill_all(); // cancel arrived before we knew the pid
                }
            }
            Line::Stage(_) => progress("excel"),
            Line::Error(l) => error_line = Some(l),
            Line::Ok { .. } => {}
            Line::Other => {
                tail.push(line);
                if tail.len() > 10 {
                    tail.remove(0);
                }
            }
        }
    }
    let status = CHILD.lock().unwrap().take().map(|mut c| c.wait());
    let stderr_tail = err_thread.join().unwrap_or_default();
    if CANCEL.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    let code = match status {
        Some(Ok(s)) => s.code(),
        other => return Err(format!("powershell wait failed: {other:?}")),
    };
    if code != Some(0) {
        let msg = error_line.unwrap_or_else(|| format!("{}\n{}", tail.join("\n"), stderr_tail).trim().to_string());
        return Err(format!("{msg} (exit code {})", code.map_or("none".into(), |c| c.to_string())));
    }
    if size_of(&output) == 0 {
        return Err("Excel produced no output".into());
    }
    let dest = place(&output, out_dir, &stem_of(req), "_v2", "xlsx")?;
    Ok(ExcelResult { output: dest.display().to_string(), size: size_of(&dest), seconds: start.elapsed().as_secs_f64(), pages, empty_pages: 0 })
}

// ---------- commands ----------

#[tauri::command]
pub async fn excel_convert(app: tauri::AppHandle, req: ExcelRequest) -> Result<ExcelResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = if req.out_mode == "folder" {
            compress::folder_dir(&app)
        } else {
            Path::new(&req.path).parent().map(Path::to_path_buf).unwrap_or_default()
        };
        convert(&req, &dir, &|p| {
            let _ = app.emit("excel-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn excel_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
    kill_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Instant;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    /// First original (not `_compressed`/`_merged`/`_organized`) sample PDF whose name starts with `prefix` and contains `needle`.
    fn find(prefix: &str, needle: &str) -> PathBuf {
        let mut v: Vec<PathBuf> = std::fs::read_dir(root().join("_samples"))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy().to_string();
                n.starts_with(prefix) && n.contains(needle) && n.ends_with(".pdf") && !n.contains("_compressed") && !n.contains("_merged") && !n.contains("_organized")
            })
            .collect();
        v.sort();
        v.remove(0)
    }

    fn out_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn req(p: &Path, engine: &str, mode: &str, numbers: bool) -> ExcelRequest {
        ExcelRequest { path: p.display().to_string(), password: None, engine: engine.into(), sheet_mode: mode.into(), numbers, out_mode: "next".into() }
    }

    fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut v = Vec::new();
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            let mut b = Vec::new();
            f.read_to_end(&mut b).unwrap();
            v.push((f.name().to_string(), b));
        }
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    fn diff_hint(a: &[u8], b: &[u8]) -> String {
        let i = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        let ctx = |s: &[u8]| String::from_utf8_lossy(&s[i.saturating_sub(80)..(i + 80).min(s.len())]).to_string();
        format!("sizes {} vs {}, first diff at {i}\n  own:      {}\n  baseline: {}", a.len(), b.len(), ctx(a), ctx(b))
    }

    fn excel_pids() -> String {
        let o = Command::new("tasklist").args(["/FI", "IMAGENAME eq EXCEL.EXE", "/FO", "CSV", "/NH"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    // ---- ignored (need pdfium + samples): cargo test excel_ -- --ignored --nocapture --test-threads=1 ----

    #[test]
    #[ignore]
    fn excel_regression_vs_spike() {
        let cases = [("table_", "IKPA", "b5_IKPA.xlsx"), ("table_", "Bon", "b5_Bon.xlsx"), ("letter_", "", "b5_letter.xlsx"), ("signed_", "", "b5_signed.xlsx")];
        let mut bad = 0;
        for (prefix, needle, base) in cases {
            let pdf = find(prefix, needle);
            let out = out_dir(&format!("majipdf_xlsx_reg_{base}"));
            let r = convert(&req(&pdf, "own", "per_page", false), &out, &|_| {}).unwrap();
            let (own, baseline) = (entries(Path::new(&r.output)), entries(&root().join("_spike/out").join(base)));
            println!("{base}: {:.2}s, {} pages, {} bytes", r.seconds, r.pages, r.size);
            let names = |v: &Vec<(String, Vec<u8>)>| v.iter().map(|e| e.0.clone()).collect::<Vec<_>>();
            if names(&own) != names(&baseline) {
                println!("  ENTRY LIST DIFFERS\n  own: {:?}\n  baseline: {:?}", names(&own), names(&baseline));
            }
            for (name, data) in &own {
                if name == "docProps/core.xml" {
                    continue; // creation timestamp
                }
                match baseline.iter().find(|e| &e.0 == name) {
                    Some((_, b)) if b == data => println!("  identical  {name}"),
                    Some((_, b)) => {
                        bad += 1;
                        println!("  DIFFERENT  {name}: {}", diff_hint(data, b));
                    }
                    None => {
                        bad += 1;
                        println!("  EXTRA      {name}");
                    }
                }
            }
            for (name, _) in &baseline {
                if own.iter().all(|e| &e.0 != name) {
                    bad += 1;
                    println!("  MISSING    {name}");
                }
            }
        }
        assert_eq!(bad, 0, "output differs from the spike baselines");
    }

    #[test]
    #[ignore]
    // big_scan_footer: 670 pages of 9 image strips each; took ~1.2 s/page until scan pages (images ≥80 % of the page) skipped images.
    fn excel_own_timings() {
        for (prefix, needle) in [("letter_", ""), ("table_", "Bon"), ("table_", "IKPA"), ("signed_", ""), ("scan_ocr_", ""), ("scanned_", ""), ("big_scan_", "")] {
            let pdf = find(prefix, needle);
            let out = out_dir("majipdf_xlsx_timing");
            let r = convert(&req(&pdf, "own", "per_page", false), &out, &|_| {}).unwrap();
            println!("{:<48.48} {:>4} pages {:>4} empty  {:>7} B  {:.2}s", pdf.file_name().unwrap().to_string_lossy(), r.pages, r.empty_pages, r.size, r.seconds);
        }
    }

    #[test]
    #[ignore]
    fn excel_sheet_mode_one() {
        let pdf = find("signed_", "");
        let out = out_dir("majipdf_xlsx_one");
        let r = convert(&req(&pdf, "own", "one", false), &out, &|_| {}).unwrap();
        let e = entries(Path::new(&r.output));
        let text = |n: &str| e.iter().find(|x| x.0 == n).map(|x| String::from_utf8_lossy(&x.1).to_string());
        let sheets = e.iter().filter(|x| x.0.starts_with("xl/worksheets/sheet")).count();
        let s = text("xl/worksheets/sheet1.xml").unwrap();
        let dim = s.split("<dimension ref=\"").nth(1).and_then(|t| t.split('"').next()).unwrap_or("?").to_string();
        let brks: Vec<&str> = s.split("<brk id=\"").skip(1).filter_map(|t| t.split('"').next()).collect();
        println!("pages={} sheets={sheets} used range={dim} page breaks (0-based first row of page)={brks:?}", r.pages);
        println!("workbook: {}", text("xl/workbook.xml").unwrap().split("<sheets>").nth(1).unwrap_or("").split("</sheets>").next().unwrap_or(""));
        println!("pageSetup: {}", s.split("<pageSetup").nth(1).unwrap_or("").split("/>").next().unwrap_or(""));
        assert_eq!(sheets, 1);
        assert_eq!(brks.len(), r.pages - 1 - r.empty_pages);
    }

    #[test]
    #[ignore]
    fn excel_engine_e2e() {
        println!("EXCEL before: {}", excel_pids());
        let pdf = find("table_", "Bon");
        let out = out_dir("majipdf_xlsx_excel_e2e");
        let r = convert(&req(&pdf, "excel", "per_page", false), &out, &|p| println!("  stage {}", p.stage)).unwrap();
        println!("Bon (excel) -> {} size={} seconds={:.1} pages={}", r.output, r.size, r.seconds, r.pages);
        assert!(r.size > 0 && r.output.ends_with("_v2.xlsx"));
        println!("EXCEL after: {}", excel_pids());
    }

    #[test]
    #[ignore]
    fn excel_cancel_e2e() {
        println!("EXCEL before: {}", excel_pids());
        let pdf = find("scan_ocr_", "");
        let out = out_dir("majipdf_xlsx_cancel");
        let h = std::thread::spawn({
            let (pdf, out) = (pdf.clone(), out.clone());
            move || convert(&req(&pdf, "excel", "per_page", false), &out, &|p| println!("  stage {}", p.stage))
        });
        std::thread::sleep(std::time::Duration::from_secs(5));
        let pid = EXCEL_PID.load(Ordering::SeqCst);
        println!("reported EXCEL pid={pid}");
        assert!(pid != 0, "no PID reported after 5 s");
        let t = Instant::now();
        excel_cancel();
        let r = h.join().unwrap();
        println!("result: {:?} (cancel took {:.1}s)", r.as_ref().err(), t.elapsed().as_secs_f64());
        assert_eq!(r.err().as_deref(), Some(CANCELLED));
        std::thread::sleep(std::time::Duration::from_secs(1));
        let gone = !excel_pids().contains(&format!("\"{pid}\""));
        println!("EXCEL after: {}", excel_pids());
        assert!(gone, "reported EXCEL pid {pid} still running");
        assert!(!out.exists() || std::fs::read_dir(&out).unwrap().next().is_none(), "output written");
    }
}
