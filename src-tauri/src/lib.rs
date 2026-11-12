#[cfg(not(target_os = "macos"))]
compile_error!("O PokeIdle Deck só roda no macOS 14+.");

mod config;
mod deck;
mod kick;
mod layout;
mod selftest;
mod webkit;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            deck::deck_state,
            deck::deck_menu,
            deck::account_menu,
            deck::account_eval,
            deck::account_report,
            kick::kick_status,
        ])
        .on_menu_event(deck::on_menu_event)
        .setup(|app| {
            let handle = app.handle();
            deck::setup(handle)?;
            match std::env::var("DECK_SELFTEST") {
                Ok(phase) => selftest::start(handle.clone(), phase),
                Err(_) => kick::setup(handle)?,
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
