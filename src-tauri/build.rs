fn main() {
    let commands = tauri_build::AppManifest::new().commands(&[
        "deck_state",
        "deck_menu",
        "account_menu",
        "account_eval",
        "account_report",
        "kick_status",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands))
        .expect("failed to run tauri-build");
}
