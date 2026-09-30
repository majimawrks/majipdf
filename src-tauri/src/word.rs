//! PDF -> Word backend: drives installed Word through the embedded PowerShell COM script. See _docs/word-contract.md.
use crate::compress::{self, place, size_of, stamp, temp_root, CANCEL};
use crate::pdf::{self, Opened};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use tauri::Emitter;

const SCRIPT: &str = include_str!("../scripts/pdf2docx.ps1");
const CANCELLED: &str = "cancelled";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Deserialize)]
pub struct WordRequest {
    path: String,
    password: Option<String>,
    out_mode: String,
}

#[derive(Serialize, Clone)]
pub struct WordProgress {
    stage: &'static str,
}

#[derive(Serialize, Debug)]
pub struct WordResult {
    output: String,
    size: u64,
    seconds: f64,
}

// ---------- script output ----------

#[derive(Debug, PartialEq)]
pub(crate) enum Line {
    Pid(u32),
    Stage(String),
    Ok { seconds: f64 },
    Error(String),
    Other,
}

pub(crate) fn parse_line(line: &str) -> Line {
    let line = line.trim();
    if let Some(n) = line.strip_prefix("PID ").and_then(|s| s.trim().parse().ok()) {
        Line::Pid(n)
    } else if let Some(s) = line.strip_prefix("STAGE ") {
        Line::Stage(s.trim().to_string())
    } else if let Some(rest) = line.strip_prefix("OK ") {
        let seconds = rest.split_whitespace().find_map(|t| t.strip_prefix("seconds=")).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        Line::Ok { seconds }
    } else if line.starts_with("ERROR ") {
        Line::Error(line.to_string())
    } else {
        Line::Other
    }
}

// ---------- process control ----------

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
static WORD_PID: AtomicU32 = AtomicU32::new(0);

pub(crate) fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(cmd, CREATE_NO_WINDOW);
    cmd
}

/// `%SystemRoot%\System32\<exe>`: Windows tools are never resolved through PATH/CWD.
pub(crate) fn sys32(exe: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let mut p = PathBuf::from(root).join("System32");
    if exe.eq_ignore_ascii_case("powershell.exe") {
        p = p.join("WindowsPowerShell").join("v1.0");
    }
    p.join(exe)
}

/// Force-kills `pid` (and its tree) only if it is still a COM-launched `image` (WINWORD.EXE / EXCEL.EXE:
/// command line has `/Automation`), so a reused pid or the user's own Office is never touched.
pub(crate) fn kill_owned(pid: u32, image: &str) {
    let check = format!("$p=Get-CimInstance Win32_Process -Filter 'ProcessId={pid}'; if($p -and $p.Name -eq '{image}' -and $p.CommandLine -like '*/Automation*'){{exit 0}}else{{exit 1}}");
    let mut c = Command::new(sys32("powershell.exe"));
    c.args(["-NoProfile", "-NonInteractive", "-Command", &check]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    if hidden(&mut c).status().is_ok_and(|s| s.success()) {
        let _ = hidden(Command::new(sys32("taskkill.exe")).args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null())).status();
    }
}

/// Kills only the WINWORD the script reported (and its tree, e.g. PDFREFLOW.EXE), then the powershell child.
fn kill_all() {
    let pid = WORD_PID.load(Ordering::SeqCst);
    if pid != 0 {
        kill_owned(pid, "WINWORD.EXE");
    }
    if let Some(c) = CHILD.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}

#[cfg(windows)]
fn suppress_pdf_warning() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).create_subkey("Software\\Microsoft\\Office\\16.0\\Word\\Options") {
        let _ = k.set_value("DisableConvertPdfWarning", &1u32);
    }
}

#[cfg(not(windows))]
fn suppress_pdf_warning() {}

// ---------- work ----------

/// Unencrypted temp copy of the input (Word flags files it crashed on and cannot open password PDFs).
pub(crate) fn prepare_copy(src: &Path, password: Option<&str>, dst: &Path) -> Result<(), String> {
    match pdf::open(src, None) {
        Opened::Ok { .. } => std::fs::copy(src, dst).map(|_| ()).map_err(|e| format!("cannot copy input: {e}")),
        Opened::NeedsPassword => {
            let pw = password.filter(|p| !p.is_empty()).ok_or("password required or wrong")?;
            if !matches!(pdf::open(src, Some(pw)), Opened::Ok { .. }) {
                return Err("password required or wrong".into());
            }
            let p = pdf::pdfium()?;
            let doc = p.load_pdf_from_file(src, Some(pw)).map_err(|e| e.to_string())?;
            let mut out = p.create_new_pdf().map_err(|e| e.to_string())?;
            let n = doc.pages().len() as i32;
            out.pages_mut().copy_page_range_from_document(&doc, 0..=n - 1, 0).map_err(|e| e.to_string())?;
            out.save_to_file(dst).map_err(|e| e.to_string())
        }
        Opened::Damaged => Err("pdfium could not open the file".into()),
    }
}

fn convert(req: &WordRequest, out_dir: &Path, emit: &dyn Fn(&WordProgress)) -> Result<WordResult, String> {
    if !crate::office_status().word {
        return Err("word_missing".into());
    }
    WORD_PID.store(0, Ordering::SeqCst);
    let start = std::time::Instant::now();
    let work = temp_root().join(stamp());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let r = convert_in(req, &work, out_dir, emit, start);
    WORD_PID.store(0, Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&work);
    r
}

fn convert_in(req: &WordRequest, work: &Path, out_dir: &Path, emit: &dyn Fn(&WordProgress), start: std::time::Instant) -> Result<WordResult, String> {
    emit(&WordProgress { stage: "preparing" });
    let src = Path::new(&req.path);
    let (input, output, script): (PathBuf, PathBuf, PathBuf) = (work.join("in.pdf"), work.join("out.docx"), work.join("pdf2docx.ps1"));
    prepare_copy(src, req.password.as_deref(), &input)?;
    std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
    suppress_pdf_warning();
    if CANCEL.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }

    let mut cmd = Command::new(sys32("powershell.exe"));
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
                WORD_PID.store(n, Ordering::SeqCst);
                if CANCEL.load(Ordering::SeqCst) {
                    kill_all(); // cancel arrived before we knew the pid
                }
            }
            Line::Stage(s) => {
                let stage = match s.as_str() {
                    "converting" => "converting",
                    "saving" => "saving",
                    _ => continue,
                };
                emit(&WordProgress { stage });
            }
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
        return Err("Word produced no output".into());
    }
    let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    let dest = place(&output, out_dir, &stem, "", "docx")?;
    Ok(WordResult { output: dest.display().to_string(), size: size_of(&dest), seconds: start.elapsed().as_secs_f64() })
}

// ---------- commands ----------

#[tauri::command]
pub async fn word_convert(app: tauri::AppHandle, req: WordRequest) -> Result<WordResult, String> {
    let guard = crate::job::begin()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard; // held until convert() has cleaned up
        let dir = if req.out_mode == "folder" {
            compress::folder_dir(&app)
        } else {
            Path::new(&req.path).parent().map(Path::to_path_buf).unwrap_or_default()
        };
        convert(&req, &dir, &|p| {
            let _ = app.emit("word-progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn word_cancel() {
    CANCEL.store(true, Ordering::SeqCst);
    kill_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_lines() {
        assert_eq!(parse_line("PID 1234"), Line::Pid(1234));
        assert_eq!(parse_line("STAGE converting\r"), Line::Stage("converting".into()));
        assert_eq!(parse_line("STAGE saving"), Line::Stage("saving".into()));
        assert_eq!(parse_line("OK seconds=13.4 out=C:\\t\\out.docx"), Line::Ok { seconds: 13.4 });
        let e = "ERROR seconds=1.2 msg=Word cannot open it";
        assert_eq!(parse_line(e), Line::Error(e.into()));
        assert_eq!(parse_line("PID abc"), Line::Other);
        assert_eq!(parse_line("something else"), Line::Other);
    }

    // ---- e2e (ignored): cargo test word_e2e -- --ignored --nocapture ----

    fn samples() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples")
    }

    fn find(prefix: &str) -> PathBuf {
        let mut v: Vec<PathBuf> = std::fs::read_dir(samples())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy().to_string();
                n.starts_with(prefix) && n.ends_with(".pdf") && !n.contains("_compressed") && !n.contains("_merged") && !n.contains("_organized")
            })
            .collect();
        v.sort();
        v.remove(0)
    }

    fn out_dir(name: &str) -> PathBuf {
        CANCEL.store(false, Ordering::SeqCst); // tests call convert() directly, without job::begin()
        let d = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn word_pids() -> String {
        let o = Command::new("tasklist").args(["/FI", "IMAGENAME eq WINWORD.EXE", "/FO", "CSV", "/NH"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn req(p: &Path, pw: Option<String>) -> WordRequest {
        WordRequest { path: p.display().to_string(), password: pw, out_mode: "next".into() }
    }

    #[test]
    #[ignore]
    fn word_e2e() {
        println!("WINWORD before: {}", word_pids());
        let letter = find("letter_");
        let out = out_dir("majipdf_word_e2e");
        let r = convert(&req(&letter, None), &out, &|p| println!("  stage {}", p.stage)).unwrap();
        println!("letter -> {} size={} seconds={:.1}", r.output, r.size, r.seconds);
        assert!(r.size > 0 && r.output.ends_with(".docx"));

        let pw = std::fs::read_to_string(samples().join("password.txt")).unwrap().trim().to_string();
        let locked = find("locked_");
        assert!(matches!(pdf::open(&locked, None), Opened::NeedsPassword));
        let r = convert(&req(&locked, Some(pw)), &out, &|p| println!("  stage {}", p.stage)).unwrap();
        println!("locked -> {} size={} seconds={:.1}", r.output, r.size, r.seconds);
        assert!(r.size > 0);
        let e = convert(&req(&locked, None), &out, &|_| {}).unwrap_err();
        println!("locked without password -> {e}");
        println!("WINWORD after: {}", word_pids());
    }

    #[test]
    #[ignore]
    fn scanned_e2e() {
        let mut pdfs: Vec<PathBuf> = std::fs::read_dir(samples()).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "pdf")).collect();
        pdfs.sort();
        for p in &pdfs {
            let v = serde_json::to_value(pdf::info(p, p.display().to_string()).unwrap()).unwrap();
            println!("{:<45.45} pages={:<5} encrypted={:<5} scanned={}", v["name"].as_str().unwrap(), v["pages"], v["encrypted"], v["scanned"]);
        }
    }

    #[test]
    #[ignore]
    fn word_cancel_e2e() {
        println!("WINWORD before: {}", word_pids());
        let p = find("scan_ocr_");
        let out = out_dir("majipdf_word_cancel");
        let h = std::thread::spawn({
            let p = p.clone();
            let out = out.clone();
            move || convert(&req(&p, None), &out, &|s| println!("  stage {}", s.stage))
        });
        std::thread::sleep(std::time::Duration::from_secs(5));
        let pid = WORD_PID.load(Ordering::SeqCst);
        println!("reported WINWORD pid={pid}");
        assert!(pid != 0, "no PID reported after 5 s");
        word_cancel();
        let r = h.join().unwrap();
        println!("result: {:?}", r.as_ref().err());
        assert_eq!(r.err().as_deref(), Some(CANCELLED));
        std::thread::sleep(std::time::Duration::from_secs(1));
        let gone = !word_pids().contains(&format!("\"{pid}\""));
        println!("WINWORD after: {}", word_pids());
        assert!(gone, "reported WINWORD pid {pid} still running");
        assert!(!out.exists() || std::fs::read_dir(&out).unwrap().next().is_none(), "output written");
    }
}
