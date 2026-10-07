//! Native payload (pdfium + Ghostscript) of the single-exe build. Release: embedded zip, extracted to
//! `%LOCALAPPDATA%\majipdf\runtime\<version>-<hash8>\`, verified against an embedded SHA-256 manifest on
//! every launch through handles that stay open (deny write/delete) while we run. Debug: `_tools/`.
//! Design: `_docs/packaging-design.md`. Never downloads anything.
//! macOS: no embedding or extraction; `gs` and `libpdfium.dylib` sit in the bundle (`Contents/Resources/runtime/`),
//! debug builds use `_tools/mac/`. The extraction code below is only used by the Windows release build.
#![cfg_attr(any(debug_assertions, not(windows)), allow(dead_code))]

use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// (file name, size, sha256 hex)
type Manifest = [(&'static str, u64, &'static str)];

#[cfg(all(windows, not(debug_assertions)))]
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/runtime_manifest.rs"));
    pub static ZIP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/runtime.zip"));
}

static VERSION: OnceLock<String> = OnceLock::new();

/// Call once at startup with the app version (from the Tauri config, which carries the build overlay).
pub fn init(version: &str) {
    let _ = VERSION.set(version.to_owned());
}

/// Removes the current directory from the DLL search path. Call first thing in `run()`.
pub fn harden() {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetDllDirectoryW(path: *const u16) -> i32;
        }
        unsafe { SetDllDirectoryW([0u16].as_ptr()) };
    }
}

/// Payload file names callers ask for (Windows also ships `gsdll64.dll`, resolved next to `GS`).
#[cfg(windows)]
pub const PDFIUM: &str = "pdfium.dll";
#[cfg(windows)]
pub const GS: &str = "gswin64c.exe";
#[cfg(not(windows))]
pub const PDFIUM: &str = "libpdfium.dylib";
#[cfg(not(windows))]
pub const GS: &str = "gs";
/// Tesseract language data for Ghostscript's `pdfocr24` device (same runtime folder as `GS`).
pub const TESS_ENG: &str = "eng.traineddata";
pub const TESS_IND: &str = "ind.traineddata";

/// Folder holding `eng` + `ind` traineddata: `<user data dir>/majipdf/tessdata/` if it has both, else the bundled one.
pub fn tessdata_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let user = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let user = std::env::var_os("HOME").map(|h| Path::new(&h).join("Library/Application Support"));
    let over = user.map(|u| u.join("majipdf").join("tessdata"));
    if let Some(d) = over.filter(|d| d.join(TESS_ENG).is_file() && d.join(TESS_IND).is_file()) {
        return Some(d);
    }
    path(TESS_ENG).ok()?.parent().map(Path::to_path_buf)
}

/// Absolute path of one payload file (`PDFIUM`, `GS`); on Windows release the first call
/// extracts/verifies (blocking), later calls are free.
pub fn path(name: &str) -> Result<PathBuf, String> {
    imp::path(name)
}

/// Background warm-up so the first tool use doesn't wait for extraction.
pub fn warm() {
    std::thread::spawn(|| {
        let _ = path(PDFIUM);
    });
}

#[cfg(all(debug_assertions, windows))]
mod imp {
    use super::*;
    pub fn path(name: &str) -> Result<PathBuf, String> {
        let sub = if name == PDFIUM { "pdfium/bin" } else if name.ends_with(".traineddata") { "tessdata" } else { "gs/bin" };
        Ok(Path::new(env!("CARGO_MANIFEST_DIR")).join("../_tools").join(sub).join(name))
    }
}

#[cfg(all(debug_assertions, not(windows)))]
mod imp {
    use super::*;
    pub fn path(name: &str) -> Result<PathBuf, String> {
        let sub = if name.ends_with(".traineddata") { "tessdata" } else { "mac" };
        Ok(Path::new(env!("CARGO_MANIFEST_DIR")).join("../_tools").join(sub).join(name))
    }
}

/// Bundle layout: `majipdf.app/Contents/MacOS/majipdf` next to `Contents/Resources/runtime/`.
#[cfg(all(not(debug_assertions), not(windows)))]
mod imp {
    use super::*;
    pub fn path(name: &str) -> Result<PathBuf, String> {
        let exe = std::env::current_exe().map_err(|e| format!("runtime unavailable: {e}"))?;
        let p = exe.parent().ok_or("runtime unavailable: no exe folder")?.join("../Resources/runtime").join(name);
        if p.is_file() { Ok(p) } else { Err(format!("runtime unavailable: {} is missing", p.display())) }
    }
}

#[cfg(all(not(debug_assertions), windows))]
mod imp {
    use super::*;
    pub fn path(name: &str) -> Result<PathBuf, String> {
        static R: OnceLock<Result<Runtime, String>> = OnceLock::new();
        if !is_manifest_name(name, &embedded::MANIFEST) {
            return Err(format!("unknown runtime file {name}"));
        }
        R.get_or_init(|| {
            let version = VERSION.get().ok_or("runtime not initialised")?;
            let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
            let root = Path::new(&local).join("majipdf").join("runtime");
            let t = Instant::now();
            let r = ensure(&root, version, embedded::RUNTIME_HASH8, &embedded::MANIFEST, embedded::ZIP);
            eprintln!("majipdf runtime ready in {:?}", t.elapsed());
            r
        })
        .as_ref()
        .map(|r| r.dir.join(name))
        .map_err(|e| format!("runtime unavailable: {e}"))
    }
}

/// The verified folder; the open handles keep its files from being changed or deleted while we run.
pub struct Runtime {
    pub dir: PathBuf,
    _held: Vec<File>,
}

// ---- pure helpers (unit-tested without the embedded zip) ----

/// Only exact manifest names are ever read from the zip or written to disk (no separators, no `..`).
fn is_manifest_name(name: &str, manifest: &Manifest) -> bool {
    manifest.iter().any(|m| m.0 == name)
}

fn target_name(version: &str, hash8: &str) -> String {
    let v: String = version.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    format!("{v}-{hash8}")
}

/// Entries of `root` that cleanup may delete: everything except the current target and the lock file.
fn cleanup_selection(entries: &[String], target: &str) -> Vec<String> {
    entries.iter().filter(|e| e.as_str() != target && e.as_str() != ".lock").cloned().collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// (bytes read, sha256 hex) of everything `r` yields.
fn hash_reader(r: &mut impl Read) -> io::Result<(u64, String)> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let k = r.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k as u64;
    }
    Ok((n, hex(&h.finalize())))
}

fn matches(got: (u64, String), size: u64, sha: &str) -> bool {
    got.0 == size && got.1.eq_ignore_ascii_case(sha)
}

// ---- file-system side ----

fn nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

fn no_reparse(p: &Path) -> Result<(), String> {
    let m = fs::symlink_metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
    #[cfg(windows)]
    if std::os::windows::fs::MetadataExt::file_attributes(&m) & 0x400 != 0 {
        return Err(format!("{} is a link/junction", p.display())); // FILE_ATTRIBUTE_REPARSE_POINT
    }
    if m.file_type().is_symlink() {
        return Err(format!("{} is a link", p.display()));
    }
    Ok(())
}

/// Read-only open that denies other writers/deleters, and opens a reparse point itself instead of following it.
fn open_held(p: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.share_mode(1 /* FILE_SHARE_READ */).custom_flags(0x0020_0000 /* FILE_FLAG_OPEN_REPARSE_POINT */);
    }
    o.open(p)
}

/// Inter-process lock: exclusive open of `path`, retried every 50 ms until `timeout`. Dropping releases it.
fn acquire_lock(path: &Path, timeout: Duration) -> Result<File, String> {
    let start = Instant::now();
    loop {
        let mut o = OpenOptions::new();
        o.write(true).create(true).truncate(false);
        #[cfg(windows)]
        std::os::windows::fs::OpenOptionsExt::share_mode(&mut o, 0);
        match o.open(path) {
            Ok(f) => return Ok(f),
            Err(e) if start.elapsed() < timeout && matches!(e.raw_os_error(), Some(32 | 33)) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("lock {}: {e}", path.display())),
        }
    }
}

/// Opens every manifest file through a deny-write handle and hashes it through that same handle.
/// The folder must hold exactly the manifest files. Returns the handles to keep.
fn open_verified(dir: &Path, manifest: &Manifest) -> Result<Vec<File>, String> {
    no_reparse(dir)?;
    for e in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let name = e.map_err(|e| e.to_string())?.file_name();
        if !name.to_str().is_some_and(|n| is_manifest_name(n, manifest)) {
            return Err(format!("unexpected entry {name:?}"));
        }
    }
    let mut held = Vec::new();
    for &(name, size, sha) in manifest {
        let mut f = open_held(&dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        let got = hash_reader(&mut f).map_err(|e| format!("{name}: {e}"))?;
        if !matches(got, size, sha) {
            return Err(format!("{name}: does not match the manifest"));
        }
        held.push(f);
    }
    Ok(held)
}

/// Unpack `zip` into a fresh staging folder (manifest names only), then rename it to `target`.
fn extract(root: &Path, target: &Path, manifest: &Manifest, zip: &[u8]) -> Result<(), String> {
    let stage = root.join(format!(".staging-{}-{}", std::process::id(), nanos()));
    fs::create_dir(&stage).map_err(|e| format!("staging: {e}"))?;
    let r = (|| -> Result<(), String> {
        let mut a = zip::ZipArchive::new(io::Cursor::new(zip)).map_err(|e| format!("embedded zip: {e}"))?;
        if let Some(bad) = a.file_names().find(|n| !is_manifest_name(n, manifest)) {
            return Err(format!("embedded zip has unexpected entry {bad:?}"));
        }
        for &(name, size, sha) in manifest {
            let mut src = a.by_name(name).map_err(|e| format!("{name}: {e}"))?.take(size + 1); // bomb guard
            let mut out = OpenOptions::new().write(true).create_new(true).open(stage.join(name)).map_err(|e| format!("{name}: {e}"))?;
            let mut h = Sha256::new();
            let mut n = 0u64;
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let k = src.read(&mut buf).map_err(|e| format!("{name}: {e}"))?;
                if k == 0 {
                    break;
                }
                h.update(&buf[..k]);
                out.write_all(&buf[..k]).map_err(|e| format!("{name}: {e}"))?;
                n += k as u64;
            }
            out.sync_all().map_err(|e| format!("{name}: {e}"))?;
            if !matches((n, hex(&h.finalize())), size, sha) {
                return Err(format!("{name}: embedded copy does not match the manifest"));
            }
        }
        if fs::symlink_metadata(target).is_ok() {
            let trash = root.join(format!(".trash-{}", nanos()));
            fs::rename(target, &trash).map_err(|e| format!("cannot move the damaged runtime aside: {e}"))?;
        }
        fs::rename(&stage, target).map_err(|e| format!("publish: {e}"))
    })();
    if r.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    r
}

fn cleanup(root: &Path, target: &str) {
    let Ok(rd) = fs::read_dir(root) else { return };
    let names: Vec<String> = rd.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
    for n in cleanup_selection(&names, target) {
        let p = root.join(&n);
        // Held open by another running instance -> fails, retried next launch. Junctions are unlinked, not followed.
        let _ = if fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) { fs::remove_file(&p) } else { fs::remove_dir_all(&p) };
    }
}

fn ensure(root: &Path, version: &str, hash8: &str, manifest: &Manifest, zip: &[u8]) -> Result<Runtime, String> {
    fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    no_reparse(root)?;
    if let Some(p) = root.parent() {
        no_reparse(p)?;
    }
    let lock = acquire_lock(&root.join(".lock"), Duration::from_secs(10))?;
    let name = target_name(version, hash8);
    let target = root.join(&name);
    let held = match open_verified(&target, manifest) {
        Ok(h) => h,
        Err(_) => {
            extract(root, &target, manifest, zip)?;
            open_verified(&target, manifest).map_err(|e| format!("after extraction: {e}"))?
        }
    };
    cleanup(root, &name);
    drop(lock);
    Ok(Runtime { dir: target, _held: held })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (manifest, zip bytes) for two tiny files; leaked to satisfy the 'static manifest type.
    fn fixture() -> (&'static Manifest, Vec<u8>) {
        let files: [(&'static str, &[u8]); 2] = [("a.dll", b"alpha-payload"), ("b.exe", b"beta")];
        let mut zw = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let mut m: Vec<(&'static str, u64, &'static str)> = Vec::new();
        for (n, d) in files {
            zw.start_file(n, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            zw.write_all(d).unwrap();
            let sha: &'static str = Box::leak(hex(&Sha256::digest(d)).into_boxed_str());
            m.push((n, d.len() as u64, sha));
        }
        (Box::leak(m.into_boxed_slice()), zw.finish().unwrap().into_inner())
    }

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("majipdf_rt_{tag}_{}_{}", std::process::id(), nanos()));
        fs::create_dir_all(&p).unwrap();
        p.join("runtime")
    }

    #[test]
    fn manifest_hash_check() {
        let (m, _) = fixture();
        let ok = hash_reader(&mut &b"alpha-payload"[..]).unwrap();
        assert!(matches(ok.clone(), m[0].1, m[0].2));
        assert!(!matches(ok, m[1].1, m[1].2)); // wrong file
        let bad = hash_reader(&mut &b"alpha-payloaD"[..]).unwrap(); // same size, one byte off
        assert!(!matches(bad, m[0].1, m[0].2));
        assert!(!matches(hash_reader(&mut &b"alpha-payloa"[..]).unwrap(), m[0].1, m[0].2)); // truncated
    }

    #[test]
    fn zip_path_rejection() {
        let (m, _) = fixture();
        assert!(is_manifest_name("a.dll", m));
        for bad in ["../a.dll", "..\\a.dll", "sub/a.dll", "C:\\evil.dll", "C:a.dll", "A.DLL", "a.dll\0", "", ".", "extra.dll", "/a.dll"] {
            assert!(!is_manifest_name(bad, m), "{bad:?}");
        }
    }

    #[test]
    fn cleanup_never_selects_current_target() {
        let e: Vec<String> = ["0.6.1-aaaa1111", "0.6.2-bbbb2222", ".lock", ".staging-1-2", ".trash-3"].map(String::from).to_vec();
        assert_eq!(cleanup_selection(&e, "0.6.2-bbbb2222"), ["0.6.1-aaaa1111", ".staging-1-2", ".trash-3"]);
        assert!(cleanup_selection(&e, "nope").iter().all(|s| s != ".lock"));
        assert_eq!(target_name("0.6/../x", "abcd1234"), "0.6_.._x-abcd1234");
    }

    #[test]
    fn ensure_extracts_repairs_and_cleans() {
        let (m, zip) = fixture();
        let root = tmp("ensure");
        fs::create_dir_all(root.join("0.5.0-old00000")).unwrap(); // stale version
        let rt = ensure(&root, "0.6.9", "cafebabe", m, &zip).unwrap();
        assert!(rt.dir.ends_with("0.6.9-cafebabe"));
        assert_eq!(fs::read(rt.dir.join("a.dll")).unwrap(), b"alpha-payload");
        assert!(!root.join("0.5.0-old00000").exists());
        #[cfg(windows)]
        {
            // held: cannot be overwritten or deleted while the handle lives
            assert!(OpenOptions::new().write(true).open(rt.dir.join("a.dll")).is_err());
            assert!(fs::remove_file(rt.dir.join("b.exe")).is_err());
            // a second instance also accepts it and never deletes it
            let rt2 = ensure(&root, "0.6.9", "cafebabe", m, &zip).unwrap();
            assert!(rt2.dir.join("a.dll").exists());
        }
        let dir = rt.dir.clone();
        drop(rt);
        // tamper while closed -> repaired
        fs::write(dir.join("a.dll"), b"alpha-payloaD").unwrap();
        fs::write(dir.join("extra.dll"), b"planted").unwrap();
        let rt = ensure(&root, "0.6.9", "cafebabe", m, &zip).unwrap();
        assert_eq!(fs::read(rt.dir.join("a.dll")).unwrap(), b"alpha-payload");
        assert!(!rt.dir.join("extra.dll").exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2, "only target + .lock remain"); // trash cleaned
        drop(rt);
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn lock_is_exclusive() {
        let root = tmp("lock");
        fs::create_dir_all(&root).unwrap();
        let l = root.join(".lock");
        let held = acquire_lock(&l, Duration::from_secs(1)).unwrap();
        assert!(acquire_lock(&l, Duration::from_millis(200)).is_err());
        drop(held);
        assert!(acquire_lock(&l, Duration::from_millis(200)).is_ok());
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    /// Release code path: `cargo test --release -- --ignored runtime_`. Extracts, runs gs and opens a PDF via pdfium.
    #[test]
    #[ignore]
    fn runtime_release_end_to_end() {
        init(&std::env::var("MAJIPDF_TEST_VERSION").unwrap_or_else(|_| "0.6.0-test".into()));
        let t = Instant::now();
        let gs = path(GS).unwrap();
        eprintln!("ensure: {:?} -> {}", t.elapsed(), gs.display());
        // First original letter_* sample (no file names of local samples in tracked code).
        let input = std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../_samples"))
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                let n = p.file_name().unwrap().to_string_lossy().to_lowercase();
                n.starts_with("letter_") && n.ends_with(".pdf") && !n.contains("_compressed")
            })
            .expect("a letter_*.pdf sample in _samples/");
        let out = std::env::temp_dir().join(format!("majipdf_rt_gs_{}.pdf", std::process::id()));
        let args: Vec<String> = ["-q", "-dNOPAUSE", "-dBATCH", "-dSAFER", "-sDEVICE=pdfwrite", "-dPDFSETTINGS=/screen"]
            .iter()
            .map(|s| s.to_string())
            .chain([format!("-sOutputFile={}", out.display()), input.display().to_string()])
            .collect();
        crate::compress::run_gs(&args, |_| {}).unwrap_or_else(|_| panic!("gs failed"));
        let size = fs::metadata(&out).unwrap().len();
        eprintln!("gs output: {size} bytes");
        assert!(size > 0);
        match crate::pdf::open(&out, None) {
            crate::pdf::Opened::Ok { pages, .. } => eprintln!("pdfium opened gs output: {pages} pages"),
            _ => panic!("pdfium could not open the gs output"),
        }
        let _ = fs::remove_file(out);
    }
}
