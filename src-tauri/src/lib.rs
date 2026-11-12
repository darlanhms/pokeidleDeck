#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("O PokeIdle Deck roda no macOS 14+ e no Windows 10/11.");

mod accounts;
mod automation;
mod central;
mod config;
mod deck;
#[cfg(target_os = "macos")]
mod kick;
#[cfg(not(target_os = "macos"))]
#[path = "kick_stub.rs"]
mod kick;
mod layout;
mod live_check;
mod lives;
mod lives_config;
mod memory;
mod hunt;
mod official;
mod page;
mod points;
mod menus;
mod prompt;
#[cfg(target_os = "macos")]
mod selftest;
#[cfg(not(target_os = "macos"))]
#[path = "selftest_stub.rs"]
mod selftest;
#[cfg(target_os = "macos")]
mod webkit;
#[cfg(windows)]
#[path = "webview2.rs"]
mod webkit;

use tauri::Manager;

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(menus::AppMenuState::default())
        .manage(prompt::PromptState::default())
        .manage(automation::Automation::default())
        .manage(lives::Lives::default())
        .manage(points::Points::default())
        .manage(central::Central::default())
        .manage(official::Official::default())
        .manage(hunt::Hunt::default())
        .invoke_handler(tauri::generate_handler![
            menus::deck_state,
            menus::deck_action,
            menus::deck_menu,
            menus::zoom_menu,
            menus::account_menu,
            menus::page_key,
            menus::ultra_menu,
            lives::lives_ui_state,
            lives::lives_ui_select,
            lives::lives_ui_park,
            lives::lives_ui_points,
            lives::lives_ui_full_page,
            lives_config::lives_config_get,
            lives_config::lives_config_set,
            points::registro_get,
            central::central_state,
            central::central_select,
            central::central_choose,
            central::central_redeem,
            central::central_channel,
            central::central_action,
            points::registro_open_file,
            automation::account_click,
            hunt::hunt_wishlist,
            automation::restock_get,
            automation::restock_set,
            accounts::account_eval,
            accounts::account_report,
            prompt::prompt_info,
            prompt::prompt_fit,
            prompt::prompt_submit,
            kick::kick_status,
        ])
        .on_menu_event(menus::on_menu_event)
        .setup(|app| {
            let handle = app.handle();
            #[cfg(windows)]
            log_to_file(handle);
            hunt::load(handle);
            deck::setup(handle)?;
            hunt::update_later(handle);
            memory::start(handle);
            if std::env::var_os("DECK_SELFTEST").is_none() {
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    webkit::compile_block_list(&handle).await;
                    let app = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        lives::start(&app);
                        points::start(&app);
                    });
                });
            }
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

/// No Windows o app não tem console: os avisos (`eprintln!`) vão para `deck.log`, na pasta de dados.
#[cfg(windows)]
fn log_to_file(app: &tauri::AppHandle) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        System::Console::{SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE},
    };
    let dir = deck::data_dir(app);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("deck.log");
    // Começa de novo a cada abertura, guardando a anterior.
    let _ = std::fs::rename(&path, dir.join("deck.anterior.log"));
    let Ok(file) = std::fs::File::create(&path) else { return };
    let handle = HANDLE(file.as_raw_handle());
    unsafe {
        let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
    }
    std::mem::forget(file);
    std::panic::set_hook(Box::new(|info| eprintln!("[pânico] {info}")));
}
