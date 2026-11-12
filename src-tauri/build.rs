fn main() {
    let commands = tauri_build::AppManifest::new().commands(&[
        "deck_state",
        "deck_action",
        "deck_menu",
        "zoom_menu",
        "account_menu",
        "page_key",
        "account_eval",
        "account_report",
        "prompt_info",
        "prompt_submit",
        "kick_status",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands))
        .expect("failed to run tauri-build");
}
