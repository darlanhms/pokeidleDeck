#[cfg(not(target_os = "macos"))]
compile_error!("O PokeIdle Deck só roda no macOS 14+.");

mod accounts;
mod config;
mod deck;
mod kick;
mod layout;
mod menus;
mod prompt;
mod selftest;
mod webkit;

use tauri::Manager;

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(menus::AppMenuState::default())
        .manage(prompt::PromptState::default())
        .invoke_handler(tauri::generate_handler![
            menus::deck_state,
            menus::deck_action,
            menus::deck_menu,
            menus::zoom_menu,
            menus::account_menu,
            menus::page_key,
            accounts::account_eval,
            accounts::account_report,
            prompt::prompt_info,
            prompt::prompt_submit,
            kick::kick_status,
        ])
        .on_menu_event(menus::on_menu_event)
        .setup(|app| {
            let handle = app.handle();
            deck::setup(handle)?;
            match std::env::var("DECK_SELFTEST") {
                Ok(phase) => selftest::start(handle.clone(), phase),
                Err(_) => kick::setup(handle)?,
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if app.try_state::<deck::Deck>().is_some() {
                deck::save_window(app);
            }
        }
    });
}
