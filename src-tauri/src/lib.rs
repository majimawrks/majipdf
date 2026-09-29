use serde::Serialize;
use std::path::Path;

mod compress;
mod merge;
mod organize;
mod pdf;
mod split;
mod excel;
mod word;

#[derive(Serialize)]
struct OfficeStatus {
    word: bool,
    excel: bool,
}

/// Directories expand recursively to their `.pdf` files.
#[tauri::command]
fn file_info(paths: Vec<String>) -> Vec<pdf::FileInfo> {
    let mut files = Vec::new();
    for p in &paths {
        pdf::expand(Path::new(p), &mut files);
    }
    files.iter().filter_map(|f| pdf::info(f, f.display().to_string())).collect()
}

#[derive(Serialize)]
struct UnlockResult {
    ok: bool,
    pages: Option<u32>,
    signed: bool,
    scanned: bool,
}

#[tauri::command]
fn unlock(path: String, password: String) -> UnlockResult {
    match pdf::open(Path::new(&path), Some(&password)) {
        pdf::Opened::Ok { pages, signed } => UnlockResult { ok: true, pages: Some(pages), signed, scanned: pdf::is_scanned(Path::new(&path), Some(&password)) },
        _ => UnlockResult { ok: false, pages: None, signed: false, scanned: false },
    }
}

#[cfg(windows)]
fn office_status_impl() -> OfficeStatus {
    use winreg::enums::HKEY_CLASSES_ROOT;
    use winreg::RegKey;

    let hkcr = RegKey::predef(HKEY_CLASSES_ROOT);
    let has = |prog_id: &str| -> bool {
        hkcr.open_subkey(format!("{prog_id}\\CLSID")).is_ok()
    };
    OfficeStatus {
        word: has("Word.Application"),
        excel: has("Excel.Application"),
    }
}

#[cfg(not(windows))]
fn office_status_impl() -> OfficeStatus {
    OfficeStatus {
        word: false,
        excel: false,
    }
}

#[tauri::command]
fn office_status() -> OfficeStatus {
    office_status_impl()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Off the startup path: a big leftover (e.g. a 50 MB scan's temp) shouldn't delay the window.
    std::thread::spawn(|| compress::sweep_temp(std::time::Duration::from_secs(6 * 3600)));
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            file_info,
            unlock,
            office_status,
            compress::compress,
            compress::compress_cancel,
            compress::keep_result,
            compress::discard_result,
            compress::open_path,
            compress::reveal_path,
            merge::merge,
            merge::merge_cancel,
            merge::thumbnail,
            split::split,
            split::split_cancel,
            organize::organize,
            organize::organize_cancel,
            word::word_convert,
            word::word_cancel,
            excel::excel_convert,
            excel::excel_cancel
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
