use crate::{
    config::{Account, DeckConfig, MAX_ACCOUNTS},
    layout, webkit,
};
use objc2::MainThreadMarker;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    utils::config::BackgroundThrottlingPolicy,
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, Webview, WebviewBuilder,
    WebviewUrl, Window, WindowBuilder, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};
use uuid::Uuid;

pub const GAME_URL: &str = "https://pokeidle.io/app";
/// Safari de verdade: com o user agent padrão do WKWebView alguns sites servem uma versão capada.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

const TOP_BAR: f64 = 36.0;
const BOTTOM_BAR: f64 = 24.0;
const HEADER: f64 = 26.0;
const GAP: f64 = 4.0;

const RECOMPRA_JS: &str = include_str!("../scripts/recompra.js");
const REPORT_JS: &str = include_str!("../scripts/report.js");

const DELETE_LABEL: &str = "Remover e apagar dados";
const KEEP_LABEL: &str = "Remover e manter dados";

pub struct Deck {
    config: Mutex<DeckConfig>,
    path: PathBuf,
    reports: Mutex<HashMap<Uuid, serde_json::Value>>,
    inset: Mutex<f64>,
}

impl Deck {
    fn update<T>(&self, change: impl FnOnce(&mut DeckConfig) -> T) -> T {
        let mut config = self.config.lock().unwrap();
        let result = change(&mut config);
        if let Err(err) = config.save(&self.path) {
            eprintln!("[deck] não salvou {}: {err}", self.path.display());
        }
        result
    }

    pub fn accounts(&self) -> Vec<Account> {
        self.config.lock().unwrap().accounts.clone()
    }

    pub fn last_report(&self, id: Uuid) -> Option<serde_json::Value> {
        self.reports.lock().unwrap().get(&id).cloned()
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pane {
    id: Uuid,
    name: String,
    #[serde(flatten)]
    rect: layout::Rect,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    panes: Vec<Pane>,
    header: f64,
    top_bar: f64,
    bottom_bar: f64,
    #[serde(skip)]
    inset: f64,
    #[serde(skip)]
    width: f64,
    #[serde(skip)]
    height: f64,
}

pub fn data_dir(app: &AppHandle) -> PathBuf {
    match std::env::var_os("DECK_DATA_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => app.path().app_data_dir().expect("sem pasta de dados do app"),
    }
}

pub fn label(id: &Uuid) -> String {
    format!("acc-{}", id.simple())
}

fn id_from_label(label: &str) -> Option<Uuid> {
    label.strip_prefix("acc-").and_then(|s| Uuid::parse_str(s).ok())
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let dir = data_dir(app);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("deck.json");
    let config = DeckConfig::load(&path);
    config.save(&path)?;
    let accounts = config.accounts.clone();
    app.manage(Deck { config: Mutex::new(config), path, reports: Mutex::default(), inset: Mutex::new(0.0) });
    tauri::async_runtime::spawn(purge_pending(app.clone()));

    let window = WindowBuilder::new(app, "main")
        .title("PokeIdle Deck")
        .inner_size(1280.0, 820.0)
        .min_inner_size(480.0, 360.0)
        .build()?;
    window.add_child(
        WebviewBuilder::new("deck", WebviewUrl::App("index.html".into())),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(1280.0, 820.0),
    )?;
    for account in &accounts {
        spawn_account(&window, account)?;
    }
    relayout(app);

    let handle = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }) {
            relayout(&handle);
        }
    });
    Ok(())
}

fn spawn_account(window: &Window, account: &Account) -> tauri::Result<Webview> {
    let mtm = MainThreadMarker::new().expect("webview de conta criada fora da main thread");
    let builder = WebviewBuilder::new(
        label(&account.id),
        WebviewUrl::External(GAME_URL.parse().unwrap()),
    )
    .with_webview_configuration(webkit::configuration(mtm, Some(account.id), false))
    .user_agent(USER_AGENT)
    .background_throttling(BackgroundThrottlingPolicy::Disabled)
    .initialization_script(RECOMPRA_JS)
    .initialization_script(REPORT_JS);
    window.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0))
}

/// Altura da barra de título que fica por cima do conteúdo. A janela do Tauri usa conteúdo de
/// tamanho cheio, e o WKWebView que fica embaixo da barra se desloca sozinho, desalinhando o deck.
fn titlebar_inset(window: &Window) -> f64 {
    let Ok(pointer) = window.ns_window() else { return 0.0 };
    let ns_window = unsafe { &*(pointer as *const objc2_app_kit::NSWindow) };
    let Some(content) = ns_window.contentView() else { return 0.0 };
    (content.frame().size.height - ns_window.contentLayoutRect().size.height).max(0.0)
}

/// Os retângulos são relativos à webview do deck, que começa logo abaixo da barra de título (`inset`).
pub fn current_layout(app: &AppHandle) -> Option<Layout> {
    let window = app.get_window("main")?;
    let scale = window.scale_factor().ok()?;
    let size = window.inner_size().ok()?.to_logical::<f64>(scale);
    let deck = app.state::<Deck>();
    let inset = *deck.inset.lock().unwrap();
    let (accounts, stacked) = {
        let config = deck.config.lock().unwrap();
        (config.accounts.clone(), config.stacked)
    };
    let area = layout::Rect {
        x: 0.0,
        y: TOP_BAR,
        w: size.width,
        h: (size.height - inset - TOP_BAR - BOTTOM_BAR).max(0.0),
    };
    let rects = layout::grid(accounts.len(), area, stacked, GAP);
    let panes = accounts
        .into_iter()
        .zip(rects)
        .map(|(account, rect)| Pane { id: account.id, name: account.name, rect })
        .collect();
    Some(Layout { panes, header: HEADER, top_bar: TOP_BAR, bottom_bar: BOTTOM_BAR, inset, width: size.width, height: size.height - inset })
}

pub fn relayout(app: &AppHandle) {
    let Some(window) = app.get_window("main") else { return };
    *app.state::<Deck>().inset.lock().unwrap() = titlebar_inset(&window);
    let Some(layout) = current_layout(app) else { return };
    let inset = layout.inset;
    if let Some(deck) = app.get_webview("deck") {
        let _ = deck.set_bounds(Rect {
            position: LogicalPosition::new(0.0, inset).into(),
            size: LogicalSize::new(layout.width, layout.height).into(),
        });
    }
    for pane in &layout.panes {
        if let Some(webview) = app.get_webview(&label(&pane.id)) {
            let r = pane.rect;
            let _ = webview.set_bounds(Rect {
                position: LogicalPosition::new(r.x, inset + r.y + HEADER).into(),
                size: LogicalSize::new(r.w, (r.h - HEADER).max(0.0)).into(),
            });
        }
    }
    let _ = app.emit_to("deck", "layout", &layout);
}

pub fn add_account(app: &AppHandle) -> Option<Account> {
    let deck = app.state::<Deck>();
    let account = deck.update(|config| {
        if config.accounts.len() >= MAX_ACCOUNTS {
            return None;
        }
        let account = config.new_account();
        config.accounts.push(account.clone());
        Some(account)
    })?;
    attach(app, &account);
    Some(account)
}

pub fn restore_account(app: &AppHandle, id: Uuid) {
    let deck = app.state::<Deck>();
    let account = deck.update(|config| {
        if config.accounts.len() >= MAX_ACCOUNTS {
            return None;
        }
        let index = config.removed.iter().position(|a| a.id == id)?;
        let account = config.removed.remove(index);
        config.accounts.push(account.clone());
        Some(account)
    });
    if let Some(account) = account {
        attach(app, &account);
    }
}

fn attach(app: &AppHandle, account: &Account) {
    if let Some(window) = app.get_window("main") {
        if let Err(err) = spawn_account(&window, account) {
            eprintln!("[deck] não criou a webview de {}: {err}", account.name);
        }
    }
    relayout(app);
}

/// Tira a conta do deck. Com `delete_data`, apaga o data store dela (login, cookies, localStorage);
/// sem, guarda a conta em `removed` para poder restaurar.
pub fn remove_account(app: &AppHandle, id: Uuid, delete_data: bool) {
    let deck = app.state::<Deck>();
    let removed = deck.update(|config| {
        let index = config.accounts.iter().position(|a| a.id == id)?;
        let account = config.accounts.remove(index);
        if delete_data {
            config.pending_delete.push(id);
        } else {
            config.removed.push(account.clone());
        }
        Some(account)
    });
    let Some(account) = removed else { return };
    if let Some(webview) = app.get_webview(&label(&id)) {
        webkit::close_page(&webview);
        let _ = webview.close();
    }
    relayout(app);
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
async fn purge_pending(app: AppHandle) {
    let pending = app.state::<Deck>().config.lock().unwrap().pending_delete.clone();
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

fn move_account(app: &AppHandle, id: Uuid, offset: isize) {
    app.state::<Deck>().update(|config| {
        let Some(index) = config.accounts.iter().position(|a| a.id == id) else { return };
        let target = index as isize + offset;
        if target >= 0 && (target as usize) < config.accounts.len() {
            config.accounts.swap(index, target as usize);
        }
    });
    relayout(app);
}

fn confirm_remove(app: &AppHandle, account: Account) {
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
                let _ = handle.run_on_main_thread(move || remove_account(&app, account.id, delete));
            }
        });
}

pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    let mut parts = id.splitn(3, ':');
    let (scope, action, target) = (parts.next(), parts.next(), parts.next());
    let target = target.and_then(|t| Uuid::parse_str(t).ok());
    match (scope, action, target) {
        (Some("deck"), Some("add"), _) => {
            add_account(app);
        }
        (Some("deck"), Some("stack"), _) => {
            app.state::<Deck>().update(|config| config.stacked = !config.stacked);
            relayout(app);
        }
        (Some("deck"), Some("restore"), Some(id)) => restore_account(app, id),
        (Some("acc"), Some("left"), Some(id)) => move_account(app, id, -1),
        (Some("acc"), Some("right"), Some(id)) => move_account(app, id, 1),
        (Some("acc"), Some("reload"), Some(id)) => {
            if let Some(webview) = app.get_webview(&label(&id)) {
                let _ = webview.reload();
            }
        }
        (Some("acc"), Some("inspect"), Some(id)) => {
            if let Some(webview) = app.get_webview(&label(&id)) {
                webview.open_devtools();
            }
        }
        (Some("acc"), Some("remove"), Some(id)) => {
            let account = app.state::<Deck>().accounts().into_iter().find(|a| a.id == id);
            if let Some(account) = account {
                confirm_remove(app, account);
            }
        }
        _ => {}
    }
}

#[tauri::command]
pub fn deck_state(app: AppHandle) -> Option<Layout> {
    current_layout(&app)
}

#[tauri::command]
pub fn deck_menu(app: AppHandle, window: Window, x: f64, y: f64) -> Result<(), String> {
    let deck = app.state::<Deck>();
    let (count, removed, stacked) = {
        let config = deck.config.lock().unwrap();
        (config.accounts.len(), config.removed.clone(), config.stacked)
    };
    let build = || -> tauri::Result<Menu<tauri::Wry>> {
        let add = MenuItem::with_id(&app, "deck:add", "+ Nova conta", count < MAX_ACCOUNTS, None::<&str>)?;
        let restore = Submenu::with_id(&app, "deck:restore-menu", "Restaurar conta removida…", !removed.is_empty() && count < MAX_ACCOUNTS)?;
        for account in &removed {
            restore.append(&MenuItem::with_id(
                &app,
                format!("deck:restore:{}", account.id),
                &account.name,
                true,
                None::<&str>,
            )?)?;
        }
        let stack = CheckMenuItem::with_id(&app, "deck:stack", "Duas contas empilhadas", true, stacked, None::<&str>)?;
        Menu::with_items(&app, &[&add, &restore, &PredefinedMenuItem::separator(&app)?, &stack])
    };
    let menu = build().map_err(|e| e.to_string())?;
    let inset = *deck.inset.lock().unwrap();
    window
        .popup_menu_at(&menu, LogicalPosition::new(x, y + inset))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn account_menu(app: AppHandle, window: Window, id: Uuid) -> Result<(), String> {
    let accounts = app.state::<Deck>().accounts();
    let Some(index) = accounts.iter().position(|a| a.id == id) else { return Ok(()) };
    let item = |action: &str, text: &str, enabled: bool| {
        MenuItem::with_id(&app, format!("acc:{action}:{id}"), text, enabled, None::<&str>)
    };
    let build = || -> tauri::Result<Menu<tauri::Wry>> {
        Menu::with_items(
            &app,
            &[
                &item("left", "Mover para a esquerda", index > 0)?,
                &item("right", "Mover para a direita", index + 1 < accounts.len())?,
                &PredefinedMenuItem::separator(&app)?,
                &item("reload", "Recarregar", true)?,
                &item("inspect", "Inspecionar página", true)?,
                &PredefinedMenuItem::separator(&app)?,
                &item("remove", "Remover conta…", true)?,
            ],
        )
    };
    let menu = build().map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn account_eval(app: AppHandle, id: Uuid, script: String) -> Result<String, String> {
    let webview = app
        .get_webview(&label(&id))
        .ok_or_else(|| "conta não existe".to_owned())?;
    webkit::call_async(&webview, &script, Duration::from_secs(30)).await
}

/// Chamado pela página do jogo a cada segundo. A conta vem do rótulo da webview, não do que a página diz.
#[tauri::command]
pub async fn account_report(app: AppHandle, webview: Webview, report: serde_json::Value) {
    if let Some(id) = id_from_label(webview.label()) {
        app.state::<Deck>().reports.lock().unwrap().insert(id, report.clone());
        let _ = app.emit_to("deck", "report", serde_json::json!({ "id": id, "report": report }));
    }
}
