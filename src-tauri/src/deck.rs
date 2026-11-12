//! Estado do deck, layout aplicado às webviews e a janela principal.

use crate::{
    accounts,
    config::{DeckConfig, LayoutMode, WindowBounds, MAX_ACCOUNTS},
    layout,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, WebviewBuilder, WebviewUrl,
    Window, WindowBuilder, WindowEvent,
};
use uuid::Uuid;

pub const TOP_BAR: f64 = 36.0;
pub const BOTTOM_BAR: f64 = 24.0;
pub const HEADER: f64 = 34.0;
const GAP: f64 = 4.0;
const DEFAULT_SIZE: (f64, f64) = (1400.0, 900.0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    #[default]
    Starting,
    Loading,
    Ok,
    Off,
}

#[derive(Clone, Debug, Default)]
pub struct Runtime {
    pub status: Status,
    /// Quando a página do jogo terminou de carregar (ms desde a época), para o "rodando há".
    pub started_at: Option<u64>,
}

pub struct Deck {
    config: Mutex<DeckConfig>,
    path: PathBuf,
    reports: Mutex<HashMap<Uuid, serde_json::Value>>,
    runtime: Mutex<HashMap<Uuid, Runtime>>,
    inset: Mutex<f64>,
    applied_zoom: Mutex<HashMap<Uuid, f64>>,
    window: Mutex<Option<WindowBounds>>,
}

impl Deck {
    /// Muda a config e grava o deck.json.
    pub fn update<T>(&self, change: impl FnOnce(&mut DeckConfig) -> T) -> T {
        let mut config = self.config.lock().unwrap();
        let result = change(&mut config);
        if let Err(err) = config.save(&self.path) {
            eprintln!("[deck] não salvou {}: {err}", self.path.display());
        }
        result
    }

    pub fn read<T>(&self, f: impl FnOnce(&DeckConfig) -> T) -> T {
        f(&self.config.lock().unwrap())
    }

    pub fn accounts(&self) -> Vec<crate::config::Account> {
        self.read(|c| c.accounts.clone())
    }

    pub fn last_report(&self, id: Uuid) -> Option<serde_json::Value> {
        self.reports.lock().unwrap().get(&id).cloned()
    }

    pub fn set_report(&self, id: Uuid, report: serde_json::Value) {
        self.reports.lock().unwrap().insert(id, report);
    }

    pub fn runtime(&self, id: Uuid) -> Runtime {
        self.runtime.lock().unwrap().get(&id).cloned().unwrap_or_default()
    }

    pub fn set_runtime(&self, id: Uuid, change: impl FnOnce(&mut Runtime)) {
        change(self.runtime.lock().unwrap().entry(id).or_default());
    }

    pub fn forget(&self, id: Uuid) {
        self.runtime.lock().unwrap().remove(&id);
        self.reports.lock().unwrap().remove(&id);
        self.applied_zoom.lock().unwrap().remove(&id);
    }

    pub fn inset(&self) -> f64 {
        *self.inset.lock().unwrap()
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneView {
    pub id: Uuid,
    pub name: String,
    pub off: bool,
    pub muted: bool,
    pub status: Status,
    pub started_at: Option<u64>,
    pub focused: bool,
    pub restock_on: bool,
    /// Relativo à webview do deck; `None` é conta escondida pelo modo ampliada.
    pub rect: Option<layout::Rect>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckView {
    pub panes: Vec<PaneView>,
    pub layout: LayoutMode,
    pub zoom: f64,
    pub stacked: bool,
    pub can_add: bool,
    pub removed: usize,
    pub fullscreen: bool,
    pub ultra: bool,
    pub reduce_motion: bool,
    pub echo: bool,
    pub header: f64,
    pub top_bar: f64,
    pub bottom_bar: f64,
    #[serde(skip)]
    pub width: f64,
    #[serde(skip)]
    pub height: f64,
}

pub fn data_dir(app: &AppHandle) -> PathBuf {
    match std::env::var_os("DECK_DATA_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => app.path().app_data_dir().expect("sem pasta de dados do app"),
    }
}

pub fn main_window(app: &AppHandle) -> Option<Window> {
    app.get_window("main")
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let dir = data_dir(app);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("deck.json");
    let config = DeckConfig::load(&path);
    config.save(&path)?;
    let (accounts, bounds, maximized) = (config.accounts.clone(), config.window, config.maximized);
    app.manage(Deck {
        config: Mutex::new(config),
        path,
        reports: Mutex::default(),
        runtime: Mutex::default(),
        inset: Mutex::new(0.0),
        applied_zoom: Mutex::default(),
        window: Mutex::new(bounds),
    });
    tauri::async_runtime::spawn(accounts::purge_pending(app.clone()));

    let mut builder = WindowBuilder::new(app, "main")
        .title("PokeIdle Deck")
        .min_inner_size(480.0, 360.0)
        .visible(false);
    builder = match bounds.filter(|b| fits_a_monitor(app, b)) {
        Some(b) => builder.position(b.x, b.y).inner_size(b.width, b.height),
        None => builder.inner_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1).center(),
    };
    let window = builder.build()?;
    window.add_child(
        WebviewBuilder::new("deck", WebviewUrl::App("index.html".into())),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1),
    )?;
    for account in accounts.iter().filter(|a| !a.off) {
        accounts::spawn(&window, account)?;
    }
    if maximized {
        window.maximize()?;
    }
    window.show()?;
    refresh(app);

    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            remember_bounds(&handle);
            refresh(&handle);
        }
        WindowEvent::Moved(_) => remember_bounds(&handle),
        WindowEvent::CloseRequested { .. } => save_window(&handle),
        _ => {}
    });
    Ok(())
}

fn fits_a_monitor(app: &AppHandle, bounds: &WindowBounds) -> bool {
    let Ok(monitors) = app.available_monitors() else { return false };
    monitors.iter().any(|m| {
        let scale = m.scale_factor();
        let (x, y) = (m.position().x as f64 / scale, m.position().y as f64 / scale);
        let (w, h) = (m.size().width as f64 / scale, m.size().height as f64 / scale);
        bounds.x < x + w && bounds.x + bounds.width > x && bounds.y < y + h && bounds.y + bounds.height > y
    })
}

/// Guarda a posição em memória a cada movimento; o disco só é gravado ao fechar.
fn remember_bounds(app: &AppHandle) {
    let Some(window) = main_window(app) else { return };
    let normal = !window.is_fullscreen().unwrap_or(false)
        && !window.is_maximized().unwrap_or(false)
        && !window.is_minimized().unwrap_or(false);
    if !normal {
        return;
    }
    let (Ok(scale), Ok(position), Ok(size)) = (window.scale_factor(), window.outer_position(), window.inner_size()) else {
        return;
    };
    let position = position.to_logical::<f64>(scale);
    let size = size.to_logical::<f64>(scale);
    *app.state::<Deck>().window.lock().unwrap() =
        Some(WindowBounds { x: position.x, y: position.y, width: size.width, height: size.height });
}

pub fn save_window(app: &AppHandle) {
    let Some(window) = main_window(app) else { return };
    let maximized = window.is_maximized().unwrap_or(false);
    let deck = app.state::<Deck>();
    let bounds = *deck.window.lock().unwrap();
    deck.update(|config| {
        config.maximized = maximized;
        if bounds.is_some() {
            config.window = bounds;
        }
    });
}

/// Altura da barra de título que fica por cima do conteúdo. A janela do Tauri usa conteúdo de
/// tamanho cheio, e o WKWebView que fica embaixo da barra se desloca sozinho, desalinhando o deck.
fn titlebar_inset(window: &Window) -> f64 {
    window.ns_window().map_or(0.0, titlebar_inset_of)
}

pub fn titlebar_inset_of(pointer: *mut std::ffi::c_void) -> f64 {
    let ns_window = unsafe { &*(pointer as *const objc2_app_kit::NSWindow) };
    let Some(content) = ns_window.contentView() else { return 0.0 };
    (content.frame().size.height - ns_window.contentLayoutRect().size.height).max(0.0)
}

pub fn view(app: &AppHandle) -> Option<DeckView> {
    let window = main_window(app)?;
    let scale = window.scale_factor().ok()?;
    let size = window.inner_size().ok()?.to_logical::<f64>(scale);
    let fullscreen = window.is_fullscreen().unwrap_or(false);
    let deck = app.state::<Deck>();
    let echo = app.state::<crate::automation::Automation>().echo();
    let height = size.height - deck.inset();
    let area = layout::Rect {
        x: 0.0,
        y: TOP_BAR,
        w: size.width,
        h: (height - TOP_BAR - BOTTOM_BAR).max(0.0),
    };
    Some(deck.read(|config| {
        let focused = config.focused.and_then(|id| config.accounts.iter().position(|a| a.id == id));
        let rects = layout::arrange(config.layout, config.accounts.len(), focused, area, config.stacked, GAP);
        let panes = config
            .accounts
            .iter()
            .zip(rects)
            .map(|(account, rect)| {
                let runtime = deck.runtime(account.id);
                PaneView {
                    id: account.id,
                    name: account.name.clone(),
                    off: account.off,
                    muted: account.muted,
                    status: if account.off { Status::Off } else { runtime.status },
                    started_at: runtime.started_at,
                    focused: config.layout == LayoutMode::Focus && config.focused == Some(account.id),
                    restock_on: account.restock.on,
                    rect,
                }
            })
            .collect();
        DeckView {
            panes,
            layout: config.layout,
            zoom: config.zoom,
            stacked: config.stacked,
            can_add: config.accounts.len() < MAX_ACCOUNTS,
            removed: config.removed.len(),
            fullscreen,
            ultra: config.ultra,
            reduce_motion: config.reduce_motion,
            echo,
            header: HEADER,
            top_bar: TOP_BAR,
            bottom_bar: BOTTOM_BAR,
            width: size.width,
            height,
        }
    }))
}

/// Põe cada webview no lugar (ou esconde), aplica o zoom e avisa a página do deck. Main thread.
pub fn refresh(app: &AppHandle) {
    let Some(window) = main_window(app) else { return };
    let deck = app.state::<Deck>();
    *deck.inset.lock().unwrap() = titlebar_inset(&window);
    let Some(view) = view(app) else { return };
    let inset = deck.inset();
    if let Some(webview) = app.get_webview("deck") {
        let _ = webview.set_bounds(Rect {
            position: LogicalPosition::new(0.0, inset).into(),
            size: LogicalSize::new(view.width, view.height).into(),
        });
    }
    for pane in &view.panes {
        let Some(webview) = app.get_webview(&accounts::label(&pane.id)) else { continue };
        match pane.rect {
            Some(r) if !pane.off => {
                let _ = webview.set_bounds(Rect {
                    position: LogicalPosition::new(r.x, inset + r.y + HEADER).into(),
                    size: LogicalSize::new(r.w, (r.h - HEADER).max(0.0)).into(),
                });
                let _ = webview.show();
            }
            _ => {
                let _ = webview.hide();
            }
        }
        let mut applied = deck.applied_zoom.lock().unwrap();
        if applied.get(&pane.id) != Some(&view.zoom) {
            let _ = webview.set_zoom(view.zoom);
            applied.insert(pane.id, view.zoom);
        }
    }
    let _ = app.emit_to("deck", "deck", &view);
    crate::menus::sync_app_menu(app);
}

/// Para quem não está na main thread (o autoteste, os diálogos).
pub fn refresh_later(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || refresh(&handle));
}
