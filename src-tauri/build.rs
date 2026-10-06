fn main() {
    // Windows release only: the macOS app ships gs/pdfium as bundle resources (tauri.macos.conf.json).
    if std::env::var("PROFILE").as_deref() == Ok("release") && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_runtime();
    }
    tauri_build::build()
}

/// Release only: packs pdfium + Ghostscript into `OUT_DIR/runtime.zip` and writes the SHA-256 manifest
/// (`runtime_manifest.rs`) that `runtime.rs` verifies on every launch. Debug builds use `_tools` directly.
fn embed_runtime() {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    // Fixed order: the manifest lines (and so the folder hash) are reproducible.
    const FILES: [(&str, &str); 3] = [
        ("pdfium.dll", "../_tools/pdfium/bin/pdfium.dll"),
        ("gswin64c.exe", "../_tools/gs/bin/gswin64c.exe"),
        ("gsdll64.dll", "../_tools/gs/bin/gsdll64.dll"),
    ];
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    let mut zw = zip::ZipWriter::new(std::fs::File::create(out.join("runtime.zip")).unwrap());
    let mut lines = String::new();
    let mut entries = String::new();
    for (name, src) in FILES {
        println!("cargo:rerun-if-changed={src}");
        let data = std::fs::read(src).unwrap_or_else(|e| {
            panic!("release build needs {src} ({e}). Put pdfium and portable Ghostscript under _tools/ first.")
        });
        let sha = hex(&Sha256::digest(&data));
        zw.start_file(name, opts).unwrap();
        zw.write_all(&data).unwrap();
        lines.push_str(&format!("{name} {} {sha}\n", data.len()));
        entries.push_str(&format!("    (\"{name}\", {}, \"{sha}\"),\n", data.len()));
    }
    zw.finish().unwrap();
    let hash8 = &hex(&Sha256::digest(lines.as_bytes()))[..8];
    std::fs::write(
        out.join("runtime_manifest.rs"),
        format!("pub const RUNTIME_HASH8: &str = \"{hash8}\";\npub const MANIFEST: [(&str, u64, &str); 3] = [\n{entries}];\n"),
    )
    .unwrap();
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
