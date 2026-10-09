use serde::Serialize;
use std::path::Path;

mod compress;
mod job;
mod merge;
mod ocr;
mod organize;
mod pdf;
mod runtime;
mod split;
mod edit;
mod excel;
mod word;

#[derive(Serialize)]
struct OfficeStatus {
    word: bool,
    excel: bool,
}

/// Directories expand recursively to their `.pdf` files.
#[tauri::command]
async fn file_info(paths: Vec<String>) -> Vec<pdf::FileInfo> {
    // Off the IPC thread: pdfium may be locked by a running conversion.
    tauri::async_runtime::spawn_blocking(move || {
        let mut files = Vec::new();
        for p in &paths {
            pdf::expand(Path::new(p), &mut files);
        }
        files.iter().filter_map(|f| pdf::info(f, f.display().to_string())).collect()
    })
    .await
    .unwrap_or_default()
}

#[derive(Serialize)]
struct UnlockResult {
    ok: bool,
    pages: Option<u32>,
    signed: bool,
    scanned: bool,
}

#[tauri::command]
async fn unlock(path: String, password: String) -> UnlockResult {
    let none = UnlockResult { ok: false, pages: None, signed: false, scanned: false };
    tauri::async_runtime::spawn_blocking(move || match pdf::open(Path::new(&path), Some(&password)) {
        pdf::Opened::Ok { pages, signed } => UnlockResult { ok: true, pages: Some(pages), signed, scanned: pdf::is_scanned(Path::new(&path), Some(&password)) },
        _ => UnlockResult { ok: false, pages: None, signed: false, scanned: false },
    })
    .await
    .unwrap_or(none)
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

/// Called by an inline script in index.html as soon as the preloader markup is parsed (before the
/// app bundle loads), so the hidden window appears with the preloader instead of an empty WebView.
#[tauri::command]
fn show_window(window: tauri::WebviewWindow) {
    let _ = window.show();
}

#[tauri::command]
fn office_status() -> OfficeStatus {
    office_status_impl()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    runtime::harden();
    // Off the startup path: a big leftover (e.g. a 50 MB scan's temp) shouldn't delay the window.
    std::thread::spawn(compress::sweep_temp);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The window starts hidden and the frontend shows it after its first paint (main.ts).
            // Fallback so a broken/slow page can never leave the app invisible.
            use tauri::Manager;
            runtime::init(&app.package_info().version.to_string());
            runtime::warm();
            if let Some(w) = app.get_webview_window("main") {
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    let _ = w.show();
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            file_info,
            show_window,
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
            excel::excel_cancel,
            ocr::ocr_run,
            ocr::ocr_cancel,
            edit::edit_open,
            edit::edit_page,
            edit::edit_render,
            edit::edit_apply,
            edit::edit_apply_obj,
            edit::edit_fonts,
            edit::sig::sig_list,
            edit::sig::sig_import,
            edit::sig::sig_add_drawn,
            edit::sig::sig_rename,
            edit::sig::sig_delete,
            edit::edit_undo,
            edit::edit_redo,
            edit::edit_history,
            edit::edit_save,
            edit::edit_close
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
