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
    take_pwstr, CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler, GetCookiesCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2CallDevToolsProtocolMethodCompletedHandler, ICoreWebView2Settings2, ICoreWebView2_11,
        ICoreWebView2_2, ICoreWebView2_8, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
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
    let _ = webview.with_webview(move |platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else { return };
        if let Err(err) = hide_webview_brand(&core) {
            eprintln!("[webview2] identificação do navegador: {err}");
        }
        if page.block_ads {
            if let Err(err) = block_ads(&core) {
                eprintln!("[webview2] bloqueio de anúncios: {err}");
            }
        }
    });
}

/// A Twitch recusa o login ("Não há suporte para seu navegador") quando os client hints trazem a marca
/// "Microsoft Edge WebView2". Fica a mesma identificação do Edge, sem essa marca, na página e em cada
/// iframe de outro site e worker (processos à parte): a checagem anti-robô da Twitch roda num iframe
/// (k.twitchcdn.net) e estranha também marcas diferentes entre ele e a página.
unsafe fn hide_webview_brand(core: &ICoreWebView2) -> windows::core::Result<()> {
    let params = unsafe { brand_override(core)? };
    let sessions = core.cast::<ICoreWebView2_11>()?;
    let receiver = unsafe { core.GetDevToolsProtocolEventReceiver(&HSTRING::from("Target.attachedToTarget"))? };
    let child_params = params.clone();
    // Cada frame ou worker novo nasce parado (waitForDebuggerOnStart) até receber a identificação.
    let on_attached = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut json = PWSTR::null();
        unsafe { args.ParameterObjectAsJson(&mut json)? };
        let event: serde_json::Value = serde_json::from_str(&take_pwstr(json)).unwrap_or_default();
        let Some(session) = event["sessionId"].as_str() else { return Ok(()) };
        let session = HSTRING::from(session);
        for (method, body) in [
            ("Network.setUserAgentOverride", child_params.as_str()),
            ("Target.setAutoAttach", AUTO_ATTACH),
            ("Runtime.runIfWaitingForDebugger", "{}"),
        ] {
            let _ = unsafe {
                sessions.CallDevToolsProtocolMethodForSession(&session, &HSTRING::from(method), &HSTRING::from(body), &ignore())
            };
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe { receiver.add_DevToolsProtocolEventReceived(&on_attached, &mut token)? };
    unsafe { core.CallDevToolsProtocolMethod(&HSTRING::from("Target.setAutoAttach"), &HSTRING::from(AUTO_ATTACH), &ignore())? };
    // A página pode ter começado a carregar antes de a troca valer (o 1º pedido sai com a marca): o que já
    // não é about:blank nem o jogo recarrega com a identificação nova.
    let page = core.clone();
    let reload = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, _| {
        match result {
            Err(err) => eprintln!("[webview2] identificação do navegador: {}", err.message()),
            Ok(()) => {
                let mut source = PWSTR::null();
                if unsafe { page.Source(&mut source) }.is_ok() {
                    let url = take_pwstr(source);
                    if url.starts_with("http") && !url.contains("pokeidle.io") {
                        let _ = unsafe { page.Reload() };
                    }
                }
            }
        }
        Ok(())
    }));
    unsafe { core.CallDevToolsProtocolMethod(&HSTRING::from("Network.setUserAgentOverride"), &HSTRING::from(params), &reload) }
}

const AUTO_ATTACH: &str = r#"{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}"#;

fn ignore() -> ICoreWebView2CallDevToolsProtocolMethodCompletedHandler {
    CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())))
}

/// Os parâmetros do `Network.setUserAgentOverride`: a identificação do próprio WebView2, sem a marca dele.
unsafe fn brand_override(core: &ICoreWebView2) -> windows::core::Result<String> {
    let settings = unsafe { core.Settings()? }.cast::<ICoreWebView2Settings2>()?;
    let mut text = PWSTR::null();
    unsafe { settings.UserAgent(&mut text)? };
    let user_agent = take_pwstr(text);
    let full = user_agent.split("Chrome/").nth(1).and_then(|v| v.split_whitespace().next()).unwrap_or("0.0.0.0").to_owned();
    let major = full.split('.').next().unwrap_or("0").to_owned();
    let brands = |version: &str| {
        serde_json::json!([
            { "brand": "Chromium", "version": version },
            { "brand": "Microsoft Edge", "version": version },
            { "brand": "Not-A.Brand", "version": if version.contains('.') { "99.0.0.0" } else { "99" } },
        ])
    };
    Ok(serde_json::json!({
        "userAgent": user_agent,
        "userAgentMetadata": {
            "brands": brands(&major),
            "fullVersionList": brands(&full),
            "platform": "Windows",
            "platformVersion": "19.0.0",
            "architecture": "x86",
            "bitness": "64",
            "model": "",
            "mobile": false,
        },
    })
    .to_string())
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
