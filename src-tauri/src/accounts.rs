//! As contas: a webview de cada uma e o que se faz com ela. Tudo aqui roda na main thread.

use crate::{
    automation, lives,
    config::{Account, MAX_ACCOUNTS},
    deck::{self, Deck, Status},
    prompt, webkit,
};
use objc2::MainThreadMarker;
use std::time::Duration;
use tauri::{
    utils::config::BackgroundThrottlingPolicy, webview::PageLoadEvent, AppHandle, LogicalPosition,
    LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl, Window,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
use uuid::Uuid;

pub const GAME_URL: &str = "https://pokeidle.io/app";
/// Safari de verdade: com o user agent padrão do WKWebView alguns sites servem uma versão capada.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

const RECOMPRA_JS: &str = include_str!("../scripts/recompra.js");
const REPORT_JS: &str = include_str!("../scripts/report.js");
const KEYS_JS: &str = include_str!("../scripts/keys.js");

const DELETE_LABEL: &str = "Remover e apagar dados";
const KEEP_LABEL: &str = "Remover e manter dados";

pub fn label(id: &Uuid) -> String {
    format!("acc-{}", id.simple())
}

pub fn id_from_label(label: &str) -> Option<Uuid> {
    label.strip_prefix("acc-").and_then(|s| Uuid::parse_str(s).ok())
}

fn webview(app: &AppHandle, id: Uuid) -> Option<Webview> {
    app.get_webview(&label(&id))
}

pub fn spawn(window: &Window, account: &Account) -> tauri::Result<Webview> {
    let mtm = MainThreadMarker::new().expect("webview de conta criada fora da main thread");
    let id = account.id;
    let reduce_motion = window.app_handle().state::<Deck>().read(|c| c.reduce_motion);
    let motion = automation::MOTION_JS.replace("__DECK_REDUCE_MOTION__", if reduce_motion { "true" } else { "false" });
    let builder = WebviewBuilder::new(label(&id), WebviewUrl::External(GAME_URL.parse().unwrap()))
        .with_webview_configuration(webkit::configuration(mtm, Some(id), false, true))
        .user_agent(USER_AGENT)
        .background_throttling(BackgroundThrottlingPolicy::Disabled)
        .initialization_script(KEYS_JS)
        .initialization_script(motion)
        .initialization_script(automation::ECHO_JS)
        .initialization_script(RECOMPRA_JS)
        .initialization_script(REPORT_JS)
        .on_page_load(move |webview, payload| {
            // Desligar navega para about:blank; isso não é "carregando" o jogo.
            if payload.url().scheme() != "https" {
                return;
            }
            let app = webview.app_handle();
            let deck = app.state::<Deck>();
            match payload.event() {
                PageLoadEvent::Started => deck.set_runtime(id, |r| r.status = Status::Loading),
                PageLoadEvent::Finished => {
                    deck.set_runtime(id, |r| {
                        r.status = Status::Ok;
                        r.started_at.get_or_insert_with(deck::now_ms);
                    });
                    if payload.url().host_str() == Some("pokeidle.io") {
                        automation::on_game_loaded(app, id);
                    }
                }
            }
            deck::refresh_later(app);
        });
    let webview = window.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0))?;
    if account.muted {
        webkit::set_page_muted(&webview, true);
    }
    window.app_handle().state::<Deck>().set_runtime(id, |r| *r = Default::default());
    Ok(webview)
}

fn attach(app: &AppHandle, account: &Account) {
    if let Some(window) = deck::main_window(app) {
        if let Err(err) = spawn(&window, account) {
            eprintln!("[deck] não criou a webview de {}: {err}", account.name);
        }
    }
}

/// Cria a conta e já pede o nome (o padrão "Conta N" fica se a pessoa cancelar).
pub fn add(app: &AppHandle) -> Option<Uuid> {
    let account = app.state::<Deck>().update(|config| {
        if config.accounts.len() >= MAX_ACCOUNTS {
            return None;
        }
        let account = config.new_account();
        config.accounts.push(account.clone());
        Some(account)
    })?;
    attach(app, &account);
    deck::refresh(app);
    if std::env::var_os("DECK_SELFTEST").is_none() {
        prompt::rename(app, account.id, "Nova conta");
    }
    Some(account.id)
}

pub fn restore(app: &AppHandle, id: Uuid) {
    let account = app.state::<Deck>().update(|config| {
        if config.accounts.len() >= MAX_ACCOUNTS {
            return None;
        }
        let index = config.removed.iter().position(|a| a.id == id)?;
        let account = config.removed.remove(index);
        config.accounts.push(account.clone());
        Some(account)
    });
    if let Some(account) = account {
        if !account.off {
            attach(app, &account);
        }
        deck::refresh(app);
    }
}

/// Tira a conta do deck (nunca a última: o deck tem de 1 a 4). Com `delete_data`, apaga login, cookies e localStorage dela; sem, guarda a
/// conta em `removed` para poder restaurar.
pub fn remove(app: &AppHandle, id: Uuid, delete_data: bool) {
    let deck = app.state::<Deck>();
    let removed = deck.update(|config| {
        if config.accounts.len() <= 1 {
            return None;
        }
        let index = config.accounts.iter().position(|a| a.id == id)?;
        let account = config.accounts.remove(index);
        if delete_data {
            config.pending_delete.push(id);
        } else {
            config.removed.push(account.clone());
        }
        config.fix_focus();
        Some(account)
    });
    let Some(account) = removed else { return };
    if let Some(webview) = webview(app, id) {
        webkit::close_page(&webview);
        let _ = webview.close();
    }
    deck.forget(id);
    lives::close(app, id);
    deck::refresh(app);
    if delete_data {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match webkit::wipe_data_store(&app, id).await {
                Ok(()) => println!("[deck] dados de {} apagados ({id})", account.name),
                Err(err) => eprintln!("[deck] não apagou os dados de {} ({id}): {err}", account.name),
            }
        });
    }
}

/// Apaga do disco os data stores das contas removidas na última vez (o conteúdo já foi limpo na hora).
/// O wry nunca libera as WKWebViews que fecha, e o WebKit só apaga data store sem nenhuma viva.
pub async fn purge_pending(app: AppHandle) {
    let pending = app.state::<Deck>().read(|c| c.pending_delete.clone());
    for id in pending {
        match webkit::remove_data_store(&app, id).await {
            Ok(()) => {
                println!("[deck] data store {id} apagado do disco");
                app.state::<Deck>().update(|config| config.pending_delete.retain(|p| *p != id));
            }
            Err(err) => eprintln!("[deck] data store {id} fica para a próxima: {err}"),
        }
    }
}

/// Troca duas contas de lugar (menu "Trocar de lugar com" e arrastar um cabeçalho até outro).
pub fn swap(app: &AppHandle, id: Uuid, other: Uuid) {
    app.state::<Deck>().update(|config| {
        let a = config.accounts.iter().position(|acc| acc.id == id);
        let b = config.accounts.iter().position(|acc| acc.id == other);
        if let (Some(a), Some(b)) = (a, b) {
            config.accounts.swap(a, b);
        }
    });
    deck::refresh(app);
}

pub fn rename(app: &AppHandle, id: Uuid, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    app.state::<Deck>().update(|config| {
        if let Some(account) = config.account_mut(id) {
            account.name = name.chars().take(40).collect();
        }
    });
    deck::refresh(app);
}

/// Desligada, a página vira about:blank e some; a conta continua no deck e não gasta nada.
pub fn set_off(app: &AppHandle, id: Uuid, off: bool) {
    let deck = app.state::<Deck>();
    let Some(account) = deck.update(|config| {
        let account = config.account_mut(id)?;
        account.off = off;
        Some(account.clone())
    }) else {
        return;
    };
    if off {
        if let Some(webview) = webview(app, id) {
            let _ = webview.navigate("about:blank".parse().unwrap());
            let _ = webview.hide();
        }
        deck.set_runtime(id, |r| *r = Default::default());
        lives::close(app, id);
    } else {
        if deck.read(|c| c.lives_accounts().contains(&id)) {
            lives::open(app, id);
        }
        match webview(app, id) {
            Some(webview) => {
                deck.set_runtime(id, |r| *r = Default::default());
                let _ = webview.navigate(GAME_URL.parse().unwrap());
            }
            None => attach(app, &account),
        }
    }
    deck::refresh(app);
}

pub fn toggle_mute(app: &AppHandle, id: Uuid) {
    let muted = app.state::<Deck>().update(|config| {
        let account = config.account_mut(id)?;
        account.muted = !account.muted;
        Some(account.muted)
    });
    if let (Some(muted), Some(webview)) = (muted, webview(app, id)) {
        webkit::set_page_muted(&webview, muted);
    }
    deck::refresh(app);
}

pub fn reload(app: &AppHandle, id: Uuid) {
    let Some(webview) = webview(app, id) else { return };
    app.state::<Deck>().set_runtime(id, |r| r.started_at = None);
    let _ = webview.reload();
}

pub fn inspect(app: &AppHandle, id: Uuid) {
    if let Some(webview) = webview(app, id) {
        webview.open_devtools();
    }
}

pub fn confirm_remove(app: &AppHandle, id: Uuid) {
    let Some(account) = app.state::<Deck>().accounts().into_iter().find(|a| a.id == id) else { return };
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "Remover \"{}\" do deck?\n\nApagar os dados tira o login e os cookies dessa conta deste Mac. \
             Mantendo, ela volta com o login em Contas → Restaurar conta removida.",
            account.name
        ))
        .title("Remover conta")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            DELETE_LABEL.into(),
            KEEP_LABEL.into(),
            "Cancelar".into(),
        ))
        .show_with_result(move |result| {
            let delete = match result {
                MessageDialogResult::Yes => Some(true),
                MessageDialogResult::No => Some(false),
                MessageDialogResult::Custom(label) if label == DELETE_LABEL => Some(true),
                MessageDialogResult::Custom(label) if label == KEEP_LABEL => Some(false),
                _ => None,
            };
            if let Some(delete) = delete {
                let app = handle.clone();
                let _ = handle.run_on_main_thread(move || remove(&app, id, delete));
            }
        });
}

#[tauri::command]
pub async fn account_eval(app: AppHandle, id: Uuid, script: String) -> Result<String, String> {
    let webview = webview(&app, id).ok_or_else(|| "conta não existe".to_owned())?;
    webkit::call_async(&webview, &script, Duration::from_secs(30)).await
}

/// Chamado pela página do jogo a cada segundo. A conta vem do rótulo da webview, não do que a página diz.
#[tauri::command]
pub async fn account_report(app: AppHandle, webview: Webview, report: serde_json::Value) {
    use tauri::Emitter;
    if let Some(id) = id_from_label(webview.label()) {
        app.state::<Deck>().set_report(id, report.clone());
        automation::on_report(&app, id, &report);
        let _ = app.emit_to("deck", "report", serde_json::json!({ "id": id, "report": report }));
    }
}
