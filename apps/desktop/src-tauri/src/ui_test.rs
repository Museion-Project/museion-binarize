//! Opt-in debug harness: fixture dialogs, genuine application IPC.
//! No listener or script execution is compiled into ordinary/release builds.
use std::io::Write;
use tauri::Listener;

pub fn attach(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.on_page_load(|webview, payload| {
        if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            return;
        }
        let Ok(script) = std::env::var("MPDF_UI_TEST_SCRIPT") else {
            return;
        };
        let Ok(log) = std::env::var("MPDF_UI_TEST_LOG") else {
            return;
        };
        webview.listen("mpdf-ui-test", move |event| {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
            {
                let _ = writeln!(file, "{}", event.payload());
            }
        });
        if let Ok(js) = std::fs::read_to_string(script) {
            let _ = webview.eval(&js);
        }
    })
}
