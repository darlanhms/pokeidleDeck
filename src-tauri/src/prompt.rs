//! A janelinha que pede o nome da conta (nova ou renomear). Uma janela separada, porque HTML do deck
//! não desenha por cima das webviews das contas.

use crate::{accounts, deck::{self, Deck}};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;

const LABEL: &str = "prompt";
const SIZE: (f64, f64) = (380.0, 150.0);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptInfo {
    title: String,
    label: String,
    value: String,
    #[serde(skip)]
    id: Uuid,
}

#[derive(Default)]
pub struct PromptState(Mutex<Option<PromptInfo>>);

pub fn rename(app: &AppHandle, id: Uuid, title: &str) {
    let Some(name) = app.state::<Deck>().accounts().into_iter().find(|a| a.id == id).map(|a| a.name) else {
        return;
    };
    *app.state::<PromptState>().0.lock().unwrap() = Some(PromptInfo {
        title: title.to_owned(),
        label: "Nome no deck (o nick do jogo, por exemplo):".to_owned(),
        value: name,
        id,
    });
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_title(title);
        let _ = window.emit("prompt-changed", ());
        let _ = window.set_focus();
        return;
    }
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("prompt.html".into()))
        .title(title)
        .inner_size(SIZE.0, SIZE.1)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .always_on_top(true)
        .focused(true);
    // Centralizada sobre o deck, não no meio da tela.
    builder = match deck::main_window(app).and_then(|w| {
        let scale = w.scale_factor().ok()?;
        let position = w.outer_position().ok()?.to_logical::<f64>(scale);
        let size = w.outer_size().ok()?.to_logical::<f64>(scale);
        Some((position.x + (size.width - SIZE.0) / 2.0, position.y + 120.0))
    }) {
        Some((x, y)) => builder.position(x, y),
        None => builder.center(),
    };
    if let Err(err) = builder.build() {
        eprintln!("[deck] não abriu a janela do nome: {err}");
    }
}

#[tauri::command]
pub fn prompt_info(app: AppHandle) -> Option<PromptInfo> {
    app.state::<PromptState>().0.lock().unwrap().clone()
}

/// A página mede o formulário; a janela fica com essa altura mais a barra de título, que fica por cima
/// do conteúdo e cortaria os botões.
#[tauri::command]
pub fn prompt_fit(window: WebviewWindow, height: f64) {
    let inset = window.ns_window().map_or(0.0, deck::titlebar_inset_of);
    let _ = window.set_size(tauri::LogicalSize::new(SIZE.0, height.ceil() + inset));
}

/// `value` vazio é cancelar.
#[tauri::command]
pub fn prompt_submit(app: AppHandle, window: WebviewWindow, value: Option<String>) {
    let pending = app.state::<PromptState>().0.lock().unwrap().take();
    if let (Some(pending), Some(value)) = (pending, value) {
        accounts::rename(&app, pending.id, &value);
    }
    let _ = window.close();
}
