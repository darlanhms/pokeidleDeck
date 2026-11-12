fn main() {
    let commands = tauri_build::AppManifest::new().commands(&[
        "deck_state",
        "deck_action",
        "deck_menu",
        "zoom_menu",
        "account_menu",
        "page_key",
        "ultra_menu",
        "lives_menu",
        "lives_ui_state",
        "lives_ui_select",
        "lives_ui_park",
        "lives_config_get",
        "lives_config_set",
        "account_click",
        "restock_get",
        "restock_set",
        "account_eval",
        "account_report",
        "prompt_info",
        "prompt_fit",
        "prompt_submit",
        "kick_status",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands))
        .expect("failed to run tauri-build");
}
