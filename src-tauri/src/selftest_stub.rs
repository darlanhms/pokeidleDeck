//! Os autotestes usam o WebKit e o `screencapture` do macOS; no Windows eles não rodam.

use tauri::AppHandle;

pub fn start(app: AppHandle, phase: String) {
    eprintln!("[selftest] a fase {phase} só roda no macOS");
    app.exit(2);
}
