/// Pinned SHA-256 of the Accurate OCR payload, verified 2026-11-18 (onnxruntime.dll 1.28.3: Authenticode valid,
/// Microsoft Corporation). Pinned while microsoft/onnxruntime is offline on GitHub (supply-chain caution, see
/// _docs/STATUS.md Risks). A different file fails the build; update a hash only after re-verifying the new file.
const PINNED: [(&str, &str); 4] = [
    ("../_tools/paddle/onnxruntime.dll", "4d2774a5f64e4a230b16b74b167171e92f65b0683c19680c1d04fb284b92a6e2"),
    ("../_tools/paddle/ch_PP-OCRv5_det_mobile.onnx", "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae"),
    ("../_tools/paddle/latin_PP-OCRv5_rec_mobile.onnx", "b20bd37c168a570f583afbc8cd7925603890efbcdc000a59e22c269d160b5f5a"),
    ("../_tools/paddle/latin_v5.dict.txt", "7b16919dcfbd4d646187f6e12ef947a0ad02c931038a5195799297889cea1f1c"),
];

fn main() {
    // every non-Mac build (dev too: debug loads these straight from _tools); a missing file is fine in debug (falls back to Fast)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        verify_pins();
    }
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
    const FILES: [(&str, &str); 9] = [
        ("pdfium.dll", "../_tools/pdfium/bin/pdfium.dll"),
        ("gswin64c.exe", "../_tools/gs/bin/gswin64c.exe"),
        ("gsdll64.dll", "../_tools/gs/bin/gsdll64.dll"),
        ("eng.traineddata", "../_tools/tessdata/eng.traineddata"),
        ("ind.traineddata", "../_tools/tessdata/ind.traineddata"),
        ("onnxruntime.dll", "../_tools/paddle/onnxruntime.dll"),
        ("ch_PP-OCRv5_det_mobile.onnx", "../_tools/paddle/ch_PP-OCRv5_det_mobile.onnx"),
        ("latin_PP-OCRv5_rec_mobile.onnx", "../_tools/paddle/latin_PP-OCRv5_rec_mobile.onnx"),
        ("latin_v5.dict.txt", "../_tools/paddle/latin_v5.dict.txt"),
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
        format!("pub const RUNTIME_HASH8: &str = \"{hash8}\";\npub const MANIFEST: [(&str, u64, &str); {}] = [\n{entries}];\n", FILES.len()),
    )
    .unwrap();
}

fn verify_pins() {
    use sha2::{Digest, Sha256};
    for (src, want) in PINNED {
        println!("cargo:rerun-if-changed={src}");
        let Ok(data) = std::fs::read(src) else { continue };
        let got = hex(&Sha256::digest(&data));
        assert!(got == want, "{src} does not match its pinned SHA-256 (got {got}, want {want}). Verify the file (Authenticode + source) before changing PINNED in build.rs.");
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
