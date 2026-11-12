//! O que o deck faz dentro das páginas do jogo: ULTRA, menos animações, eco de cliques e recompra.

use crate::{
    accounts,
    config::RestockConfig,
    deck::{self, Deck},
    webkit,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, Webview, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use uuid::Uuid;

pub const MOTION_JS: &str = include_str!("../scripts/motion.js");
pub const ECHO_JS: &str = include_str!("../scripts/echo.js");
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);

// Modo ULTRA Economia do jogo ("ULTRA Battery Saver"): a escolha fica no localStorage ("cfg-ultra") e o
// jogo acende a classe "modo-ultra" no <html> quando entra com ela ligada. Liga pelo mesmo caminho do
// jogador (Configurações → interruptor do ULTRA) e desliga pelo "Voltar ao jogo" da caixa. Fora do jogo,
// ou com a conta ainda sem equipe, só fica a escolha: o próprio jogo liga ao entrar.
const SET_ULTRA_JS: &str = "const on = ON;\n\
    const ultra = () => document.documentElement.classList.contains('modo-ultra');\n\
    if (!on) { document.querySelector('#ultra-sair')?.click(); localStorage.setItem('cfg-ultra', '0'); return !ultra(); }\n\
    if (ultra()) return true;\n\
    const app = document.querySelector('#app');\n\
    if (!app || app.classList.contains('hidden') || localStorage.getItem('cfg-ultra') === '1') { localStorage.setItem('cfg-ultra', '1'); return true; }\n\
    document.querySelector('#btn-config')?.click(); document.querySelector('#cfg-ultra')?.click();\n\
    return ultra();";

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
/// "ligado" (na tela), "escolhido" (liga ao entrar no jogo), "normal" ou "fora" (fora do jogo).
pub const ULTRA_STATE_JS: &str = "return document.documentElement.classList.contains('modo-ultra') ? 'ligado'\n\
    : localStorage.getItem('cfg-ultra') === '1' ? 'escolhido'\n\
    : document.querySelector('#app:not(.hidden)') ? 'normal' : 'fora';";

/// Estado que não vai para o deck.json: o eco começa desligado a cada abertura, como no original.
#[derive(Default)]
pub struct Automation {
    echo: AtomicBool,
}

impl Automation {
    pub fn echo(&self) -> bool {
        self.echo.load(Ordering::Relaxed)
    }
}

fn game_webviews(app: &AppHandle) -> Vec<(Uuid, String, Webview)> {
    app.state::<Deck>()
        .accounts()
        .into_iter()
        .filter(|a| !a.off)
        .filter_map(|a| app.get_webview(&accounts::label(&a.id)).map(|w| (a.id, a.name, w)))
        .collect()
}

fn on_game(webview: &Webview) -> bool {
    webview.url().is_ok_and(|u| u.host_str() == Some("pokeidle.io"))
}

/// A página do jogo terminou de carregar (inclusive quando o próprio jogo recarrega numa versão nova):
/// ela começa sem nada do deck, então tudo volta.
pub fn on_game_loaded(app: &AppHandle, id: Uuid) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(webview) = app.get_webview(&accounts::label(&id)) else { return };
        let (ultra, reduce_motion) = app.state::<Deck>().read(|c| (c.ultra, c.reduce_motion));
        let _ = webkit::call_async(&webview, &motion_script(reduce_motion), EVAL_TIMEOUT).await;
        if ultra {
            let _ = webkit::call_async(&webview, &SET_ULTRA_JS.replace("ON", "true"), EVAL_TIMEOUT).await;
        }
        push_restock(&app, id).await;
    });
}

fn motion_script(reduce: bool) -> String {
    format!("return window.__deckMotion ? window.__deckMotion.set({reduce}) : null;")
}

pub fn set_reduce_motion(app: &AppHandle, reduce: bool) {
    app.state::<Deck>().update(|c| c.reduce_motion = reduce);
    deck::refresh(app);
    for (_, _, webview) in game_webviews(app) {
        tauri::async_runtime::spawn(async move {
            let _ = webkit::call_async(&webview, &motion_script(reduce), EVAL_TIMEOUT).await;
        });
    }
}

pub fn set_ultra(app: &AppHandle, on: bool) {
    app.state::<Deck>().update(|c| c.ultra = on);
    deck::refresh(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let script = SET_ULTRA_JS.replace("ON", if on { "true" } else { "false" });
        let mut failed = Vec::new();
        for (_, name, webview) in game_webviews(&app) {
            if !on_game(&webview) {
                continue;
            }
            let ok = webkit::call_async(&webview, &script, EVAL_TIMEOUT).await.as_deref() == Ok("true");
            if !ok {
                failed.push(name);
            }
        }
        if !failed.is_empty() {
            app.dialog()
                .message(format!(
                    "Não consegui {} o ULTRA em: {}.\n\nO deck usa o interruptor do próprio jogo (Configurações → \
                     Desempenho → Modo ULTRA Economia): recarregue a conta e tente de novo, ou ligue por lá.",
                    if on { "ligar" } else { "desligar" },
                    failed.join(", ")
                ))
                .title("ULTRA")
                .kind(MessageDialogKind::Warning)
                .show(|_| {});
        }
    });
}

pub fn set_echo(app: &AppHandle, on: bool) {
    app.state::<Automation>().echo.store(on, Ordering::Relaxed);
    deck::refresh(app);
}

/// Um aviso rápido no cabeçalho da conta (some sozinho).
pub fn note(app: &AppHandle, id: Uuid, text: String) {
    let _ = app.emit_to("deck", "note", serde_json::json!({ "id": id, "text": text }));
}

/// Um clique de verdade numa página; os campos são relidos um a um (a página não escolhe o script).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Click {
    pub path: String,
    pub tag: String,
    pub text: String,
    pub fx: f64,
    pub fy: f64,
}

// O mesmo do original: acha o botão pelo caminho e pelo texto (ou o único com esse texto) e clica nele
// com a sequência de eventos de um mouse. Devolve false se a conta está numa tela sem esse botão.
const REPLAY_JS: &str = "const m = CLICK;\n\
    const textOf = el => String(el.getAttribute('aria-label') || el.getAttribute('title') || el.innerText || el.value || '').trim().replace(/\\s+/g, ' ').slice(0, 80);\n\
    let el = null;\n\
    try { el = document.querySelector(m.path); } catch (e) { }\n\
    if (el && textOf(el) !== m.text) el = null;\n\
    if (!el && m.text) {\n\
      let same = [];\n\
      try { same = [...document.querySelectorAll(m.tag)].filter(c => textOf(c) === m.text); } catch (e) { }\n\
      if (same.length === 1) el = same[0];\n\
    }\n\
    if (!el) return false;\n\
    const r = el.getBoundingClientRect();\n\
    const base = { bubbles: true, cancelable: true, composed: true, view: window, button: 0, clientX: r.left + r.width * m.fx, clientY: r.top + r.height * m.fy };\n\
    const pointer = { ...base, pointerId: 1, pointerType: 'mouse', isPrimary: true };\n\
    el.dispatchEvent(new PointerEvent('pointerdown', { ...pointer, buttons: 1 }));\n\
    el.dispatchEvent(new MouseEvent('mousedown', { ...base, buttons: 1 }));\n\
    el.dispatchEvent(new PointerEvent('pointerup', pointer));\n\
    el.dispatchEvent(new MouseEvent('mouseup', base));\n\
    el.click();\n\
    return true;";

/// Repete nas outras contas o clique feito numa; avisa no cabeçalho de quem não tem esse botão. Os
/// cliques repetidos não são "de verdade" (isTrusted falso), então não voltam em eco.
#[tauri::command]
pub async fn account_click(app: AppHandle, webview: Webview, click: Click) {
    if let Some(source) = accounts::id_from_label(webview.label()) {
        echo_from(&app, source, click).await;
    }
}

pub async fn echo_from(app: &AppHandle, source: Uuid, click: Click) {
    if !app.state::<Automation>().echo() {
        return;
    }
    let click = Click {
        path: click.path.chars().take(2000).collect(),
        tag: click.tag.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect(),
        text: click.text.chars().take(80).collect(),
        fx: click.fx.clamp(0.0, 1.0),
        fy: click.fy.clamp(0.0, 1.0),
    };
    let script = REPLAY_JS.replace("CLICK", &serde_json::to_string(&click).unwrap_or_default());
    let label = if click.text.is_empty() { "esse botão".to_owned() } else { format!("\"{}\"", click.text) };
    for (id, _, target) in game_webviews(app).into_iter().filter(|(id, _, _)| *id != source) {
        let app = app.clone();
        let script = script.clone();
        let label = label.clone();
        tauri::async_runtime::spawn(async move {
            if webkit::call_async(&target, &script, EVAL_TIMEOUT).await.as_deref() != Ok("true") {
                note(&app, id, format!("eco: {label} não está nesta tela"));
            }
        });
    }
}

// ---------- Recompra ----------

fn restock_label(id: Uuid) -> String {
    format!("restock-{}", id.simple())
}

pub fn restock_id_from_label(label: &str) -> Option<Uuid> {
    label.strip_prefix("restock-").and_then(|s| Uuid::parse_str(s).ok())
}

/// Manda a configuração da recompra para a página (o script dela já está lá desde o carregamento).
pub async fn push_restock(app: &AppHandle, id: Uuid) {
    let Some(webview) = app.get_webview(&accounts::label(&id)) else { return };
    if !on_game(&webview) {
        return;
    }
    let Some(config) = app
        .state::<Deck>()
        .read(|c| c.accounts.iter().find(|a| a.id == id).map(|a| a.restock.clone()))
    else {
        return;
    };
    let json = serde_json::to_string(&config).unwrap_or_default();
    let script = format!("return window.__deckRecompra ? window.__deckRecompra.configure({json}) : false;");
    let _ = webkit::call_async(&webview, &script, EVAL_TIMEOUT).await;
}

/// O relatório da página chegou: a recompra que perdeu a configuração (página nova sem o aviso de pronta)
/// recebe de novo, e a janela da recompra aberta vê o estoque.
pub fn on_report(app: &AppHandle, id: Uuid, report: &serde_json::Value) {
    if report["restock"]["configured"] == false {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { push_restock(&app, id).await });
    }
    let label = restock_label(id);
    if app.get_webview_window(&label).is_some() {
        let _ = app.emit_to(label.as_str(), "restock-status", &report["restock"]);
    }
}

/// O carrinho do cabeçalho: abre a recompra da conta (aberta de novo, só vem para a frente).
pub fn open_restock(app: &AppHandle, id: Uuid) {
    let label = restock_label(id);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return;
    }
    let Some(name) = app.state::<Deck>().accounts().into_iter().find(|a| a.id == id).map(|a| a.name) else {
        return;
    };
    deck::off_main(app, move |app| {
        let result = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("restock.html".into()))
            .title(format!("Recompra · {name}"))
            .inner_size(600.0, 780.0)
            .min_inner_size(520.0, 400.0)
            .minimizable(false)
            .center()
            .build();
        if let Err(err) = result {
            eprintln!("[deck] não abriu a recompra de {name}: {err}");
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestockInfo {
    name: String,
    config: RestockConfig,
    status: serde_json::Value,
}

/// A janela pergunta pela própria conta (o rótulo dela diz qual).
#[tauri::command]
pub fn restock_get(app: AppHandle, window: tauri::WebviewWindow) -> Option<RestockInfo> {
    let id = restock_id_from_label(window.label())?;
    let account = app.state::<Deck>().accounts().into_iter().find(|a| a.id == id)?;
    let status = app.state::<Deck>().last_report(id).map(|r| r["restock"].clone()).unwrap_or_default();
    Some(RestockInfo { name: account.name, config: account.restock, status })
}

/// Vale na hora, sem botão de salvar: grava, manda para a página e o carrinho do cabeçalho muda.
#[tauri::command]
pub async fn restock_set(app: AppHandle, window: tauri::WebviewWindow, config: RestockConfig) -> Result<(), String> {
    let id = restock_id_from_label(window.label()).ok_or("janela sem conta")?;
    set_restock(&app, id, config).await;
    Ok(())
}

pub async fn set_restock(app: &AppHandle, id: Uuid, config: RestockConfig) {
    let config = config.normalized();
    app.state::<Deck>().update(|c| {
        if let Some(account) = c.account_mut(id) {
            account.restock = config;
        }
    });
    deck::refresh_later(app);
    push_restock(app, id).await;
}
