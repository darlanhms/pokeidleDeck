//! O que o Tauri/wry não expõe do WebView2 (Windows). Mesma interface do `webkit.rs` do macOS.
//!
//! Cada conta tem a própria pasta de dados do WebView2, como o perfil de cada conta no original; o jogo
//! e as lives da conta dividem a mesma pasta (e o mesmo login da Twitch/Kick).

use crate::page::{Page, BLOCKED};
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::{AppHandle, Manager, Runtime, Webview, WebviewBuilder};
use tokio::sync::oneshot;
use uuid::Uuid;
use webview2_com::{
    take_pwstr, CallDevToolsProtocolMethodCompletedHandler, GetCookiesCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2_2, ICoreWebView2_8, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
    },
    WebResourceRequestedEventHandler,
};
use windows::core::{Interface, HSTRING, PWSTR};

/// Os argumentos do original: página escondida ou coberta não desacelera, e vídeo toca sem clique.
/// Todas as páginas de uma mesma pasta de dados precisam dos mesmos argumentos.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
    --disable-background-timer-throttling --disable-backgrounding-occluded-windows \
    --disable-renderer-backgrounding --autoplay-policy=no-user-gesture-required --process-per-site";

struct Reply<T>(Mutex<Option<oneshot::Sender<T>>>);

impl<T> Reply<T> {
    fn new() -> (Self, oneshot::Receiver<T>) {
        let (tx, rx) = oneshot::channel();
        (Self(Mutex::new(Some(tx))), rx)
    }

    fn send(&self, value: T) {
        if let Some(tx) = self.0.lock().unwrap().take() {
            let _ = tx.send(value);
        }
    }
}

fn store_dir(app: &AppHandle, id: Uuid) -> PathBuf {
    crate::deck::data_dir(app).join("WebView2").join(id.simple().to_string())
}

/// A pasta de dados da conta (ou uma sessão anônima, sem `store`).
pub fn prepare<R: Runtime>(app: &AppHandle, builder: WebviewBuilder<R>, store: Option<Uuid>, _page: Page) -> WebviewBuilder<R> {
    match store {
        Some(id) => builder.data_directory(store_dir(app, id)).additional_browser_args(&browser_args(app, id)),
        None => builder.incognito(true),
    }
}

/// Com `DECK_DEBUG_PORT=9300`, o navegador da 1ª conta abre o DevTools remoto na 9301, o da 2ª na 9302…
fn browser_args(app: &AppHandle, id: Uuid) -> String {
    let port = std::env::var("DECK_DEBUG_PORT").ok().and_then(|p| p.parse::<u16>().ok());
    let index = app.state::<crate::deck::Deck>().read(|c| c.accounts.iter().position(|a| a.id == id));
    match (port, index) {
        (Some(base), Some(index)) => format!("{BROWSER_ARGS} --remote-debugging-port={}", base + 1 + index as u16),
        _ => BROWSER_ARGS.to_owned(),
    }
}

/// Depois de criada: o bloqueio de anúncios entra como filtro de pedidos.
pub fn attach(webview: &Webview, page: Page) {
    if !page.block_ads {
        return;
    }
    let _ = webview.with_webview(|platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else { return };
        if let Err(err) = block_ads(&core) {
            eprintln!("[webview2] bloqueio de anúncios: {err}");
        }
    });
}

unsafe fn block_ads(core: &ICoreWebView2) -> windows::core::Result<()> {
    for domain in BLOCKED {
        for pattern in [format!("*://{domain}/*"), format!("*://*.{domain}/*")] {
            unsafe { core.AddWebResourceRequestedFilter(&HSTRING::from(pattern), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)? };
        }
    }
    let environment = core.cast::<ICoreWebView2_2>()?;
    let environment = unsafe { environment.Environment()? };
    let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            let response = unsafe {
                environment.CreateWebResourceResponse(None, 403, &HSTRING::from("Blocked"), &HSTRING::new())?
            };
            unsafe { args.SetResponse(&response)? };
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { core.add_WebResourceRequested(&handler, &mut token) }
}

/// Roda `script` como corpo de uma função async na página e devolve o valor resolvido em JSON, pelo
/// `Runtime.evaluate` com `awaitPromise` do DevTools (como o original).
pub async fn call_async(webview: &Webview, script: &str, timeout: Duration) -> Result<String, String> {
    let expression = format!("(async () => {{\n{}\n}})()", crate::page::async_body(script));
    let params = serde_json::json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }).to_string();
    let (reply, rx) = Reply::<Result<String, String>>::new();
    let reply = std::sync::Arc::new(reply);
    let on_error = reply.clone();
    webview
        .with_webview(move |platform| unsafe {
            let started = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, json| {
                    reply.send(match result {
                        Ok(()) => evaluated(&json),
                        Err(err) => Err(err.message()),
                    });
                    Ok(())
                }));
                core.CallDevToolsProtocolMethod(&HSTRING::from("Runtime.evaluate"), &HSTRING::from(params), &handler)
            })();
            if let Err(err) = started {
                on_error.send(Err(err.message()));
            }
        })
        .map_err(|e| e.to_string())?;
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("a página fechou antes de responder".into()),
        Err(_) => Err("a página não respondeu a tempo".into()),
    }
}

/// A resposta do `Runtime.evaluate`: o texto JSON que o corpo devolveu, ou a mensagem da exceção.
fn evaluated(json: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if let Some(exception) = v.get("exceptionDetails") {
        let message = exception["exception"]["description"].as_str().or(exception["text"].as_str()).unwrap_or("erro no script");
        return Err(message.to_owned());
    }
    Ok(v["result"]["value"].as_str().unwrap_or("null").to_owned())
}

/// No WebView2 a página fecha junto com a webview.
pub fn close_page(_webview: &Webview) {}

/// A pasta da conta só sai com o navegador dela fechado: fica para o próximo início.
pub async fn wipe_data_store(_app: &AppHandle, _id: Uuid) -> Result<(), String> {
    Err("os dados saem no próximo início do deck".into())
}

pub async fn remove_data_store(app: &AppHandle, id: Uuid) -> Result<(), String> {
    let dir = store_dir(app, id);
    if !dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
}

/// Uma página aberta na pasta de dados da conta, para ler os cookies dela.
fn page_of(app: &AppHandle, store: Uuid) -> Option<Webview> {
    let id = store.simple().to_string();
    let mut pages: Vec<Webview> = app
        .webviews()
        .into_values()
        .filter(|w| {
            let label = w.label();
            crate::accounts::id_from_label(label) == Some(store) || label.starts_with(&format!("live-{id}-"))
        })
        .collect();
    pages.sort_by_key(|w| !w.label().starts_with("acc-"));
    pages.into_iter().next()
}

/// Os cookies (nome, valor) da conta cujo domínio termina em `domain`, inclusive os HttpOnly.
pub async fn cookies(app: &AppHandle, store: Uuid, domain: &str) -> Result<Vec<(String, String)>, String> {
    let page = page_of(app, store).ok_or("nenhuma página da conta aberta")?;
    let domain = domain.trim_start_matches('.').to_owned();
    let (reply, rx) = Reply::<Result<Vec<(String, String)>, String>>::new();
    let reply = std::sync::Arc::new(reply);
    let on_error = reply.clone();
    page.with_webview(move |platform| unsafe {
        let started = (|| -> windows::core::Result<()> {
            let core = platform.controller().CoreWebView2()?.cast::<ICoreWebView2_2>()?;
            let manager = core.CookieManager()?;
            let handler = GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                let found = result.and_then(|()| {
                    let mut out = Vec::new();
                    if let Some(list) = list {
                        let mut count = 0u32;
                        list.Count(&mut count)?;
                        for i in 0..count {
                            let cookie = list.GetValueAtIndex(i)?;
                            let read = |f: &dyn Fn(*mut PWSTR) -> windows::core::Result<()>| {
                                let mut text = PWSTR::null();
                                f(&mut text).map(|()| take_pwstr(text))
                            };
                            let host = read(&|p| cookie.Domain(p))?;
                            let host = host.trim_start_matches('.');
                            if host == domain || host.ends_with(&format!(".{domain}")) {
                                out.push((read(&|p| cookie.Name(p))?, read(&|p| cookie.Value(p))?));
                            }
                        }
                    }
                    Ok(out)
                });
                reply.send(found.map_err(|e| e.message()));
                Ok(())
            }));
            manager.GetCookies(&HSTRING::new(), &handler)
        })();
        if let Err(err) = started {
            on_error.send(Err(err.message()));
        }
    })
    .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| "o WebView2 não respondeu".to_owned())?
        .map_err(|_| "o WebView2 não respondeu".to_owned())?
}

/// No Windows o bloqueio não precisa de compilação.
pub async fn compile_block_list(_app: &AppHandle) {}

pub fn set_page_muted(webview: &Webview, muted: bool) {
    let _ = webview.with_webview(move |platform| unsafe {
        if let Ok(core) = platform.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_8>()) {
            let _ = core.SetIsMuted(muted);
        }
    });
}

/// O processo do navegador da página (no WebView2, um por pasta de dados).
pub async fn web_process(webview: &Webview) -> Option<u32> {
    let (tx, rx) = oneshot::channel();
    webview
        .with_webview(move |platform| unsafe {
            let mut pid = 0u32;
            if let Ok(core) = platform.controller().CoreWebView2() {
                let _ = core.BrowserProcessId(&mut pid);
            }
            let _ = tx.send(pid);
        })
        .ok()?;
    let pid = tokio::time::timeout(Duration::from_secs(2), rx).await.ok()?.ok()?;
    (pid > 0).then_some(pid)
}
