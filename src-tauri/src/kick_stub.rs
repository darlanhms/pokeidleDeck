//! O spike da Kick (fase 1) testou o WebKit; no Windows não há o que testar.

use tauri::AppHandle;

pub fn setup(_app: &AppHandle) -> tauri::Result<()> {
    Ok(())
}

#[tauri::command]
pub fn kick_status(_app: AppHandle) -> Option<()> {
    None
}
