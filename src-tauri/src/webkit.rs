//! O que o Tauri/wry não expõe do WKWebView.

use block2::RcBlock;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, Sel},
    sel, MainThreadMarker,
};
use objc2_foundation::{ns_string, NSArray, NSDate, NSError, NSString, NSUUID};
use objc2_web_kit::{
    WKAudiovisualMediaTypes, WKContentWorld, WKWebView, WKWebViewConfiguration, WKWebsiteDataRecord,
    WKWebsiteDataStore,
};
use crate::page::{Page, BLOCKED};
use std::{ptr::NonNull, sync::Mutex, time::Duration};
use tauri::{AppHandle, Runtime, Webview, WebviewBuilder};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Os completion handlers do WebKit são blocos `Fn`; a resposta só pode sair uma vez.
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

fn ns_uuid(id: Uuid) -> Retained<NSUUID> {
    NSUUID::from_bytes(*id.as_bytes())
}

/// O data store da conta (ou um efêmero, sem `store`) e o resto de `page`, na configuração do WKWebView.
/// Só na main thread.
pub fn prepare<R: Runtime>(_app: &AppHandle, builder: WebviewBuilder<R>, store: Option<Uuid>, page: Page) -> WebviewBuilder<R> {
    let mtm = MainThreadMarker::new().expect("webview criada fora da main thread");
    let config = configuration(mtm, store, page.autoplay, page.keep_running_hidden);
    if page.block_ads {
        add_block_list(&config);
    }
    builder.with_webview_configuration(config)
}

/// Depois de criada: com `keep_running_hidden`, o WebKit também deixa de olhar se a janela está coberta.
pub fn attach(webview: &Webview, page: Page) {
    if page.keep_running_hidden && page.autoplay {
        disable_occlusion_detection(webview);
    }
}

/// Configuração com o data store persistente da conta (ou um efêmero, sem `store`). Passar a
/// configuração pronta é o único jeito de ligar o autoplay sem gesto; com ela o wry deixa de aplicar
/// o `data_store_identifier` sozinho.
pub fn configuration(
    mtm: MainThreadMarker,
    store: Option<Uuid>,
    autoplay: bool,
    keep_running_hidden: bool,
) -> Retained<WKWebViewConfiguration> {
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        let data_store = match store {
            Some(id) => WKWebsiteDataStore::dataStoreForIdentifier(&ns_uuid(id), mtm),
            None => WKWebsiteDataStore::nonPersistentDataStore(mtm),
        };
        config.setWebsiteDataStore(&data_store);
        if autoplay {
            config.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::None);
        }
        if keep_running_hidden {
            keep_running_when_hidden(&config);
        }
        config
    }
}

/// Página oculta (fora da tela ou coberta) tem o processo suprimido e os timers estrangulados pelo
/// WebKit, o que mata o player da Kick em poucos minutos e deixaria as contas escondidas (modo
/// ampliada) mais lentas. Preferências privadas do WKPreferences.
unsafe fn keep_running_when_hidden(config: &WKWebViewConfiguration) {
    let preferences = unsafe { config.preferences() };
    let object: &AnyObject = &preferences;
    unsafe {
        let a = responds(object, sel!(_setPageVisibilityBasedProcessSuppressionEnabled:));
        if a {
            let _: () = msg_send![object, _setPageVisibilityBasedProcessSuppressionEnabled: Bool::NO];
        }
        let b = responds(object, sel!(_setHiddenPageDOMTimerThrottlingEnabled:));
        if b {
            let _: () = msg_send![object, _setHiddenPageDOMTimerThrottlingEnabled: Bool::NO];
        }
        let c = responds(object, sel!(_setHiddenPageDOMTimerThrottlingAutoIncreases:));
        if c {
            let _: () = msg_send![object, _setHiddenPageDOMTimerThrottlingAutoIncreases: Bool::NO];
        }
        if !(a && b && c) {
            eprintln!("[webkit] WKPreferences privadas ausentes: supressão={a} throttling={b} auto={c}");
        }
    }
}

/// Roda `script` como corpo de uma função async na página (pode usar `await`) e devolve o valor
/// resolvido em JSON. É o `Runtime.evaluate` com `awaitPromise` do original.
pub async fn call_async(webview: &Webview, script: &str, timeout: Duration) -> Result<String, String> {
    let (reply, rx) = Reply::<Result<String, String>>::new();
    // O valor volta como texto para não depender da conversão JS → NSDictionary do WebKit.
    let body = crate::page::async_body(script);
    webview
        .with_webview(move |platform| unsafe {
            let webview = &*(platform.inner() as *const WKWebView);
            let mtm = MainThreadMarker::new_unchecked();
            let done = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                reply.send(match error.as_ref() {
                    Some(error) => Err(describe(error)),
                    None => Ok(value
                        .as_ref()
                        .and_then(|v| v.downcast_ref::<NSString>())
                        .map_or_else(|| "null".to_owned(), |text| text.to_string())),
                });
            });
            webview.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                &NSString::from_str(&body),
                None,
                None,
                &WKContentWorld::pageWorld(mtm),
                Some(&done),
            );
        })
        .map_err(|e| e.to_string())?;
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("a página fechou antes de responder".into()),
        Err(_) => Err("a página não respondeu a tempo".into()),
    }
}

fn describe(error: &NSError) -> String {
    error
        .userInfo()
        .objectForKey(ns_string!("WKJavaScriptExceptionMessage"))
        .and_then(|m| m.downcast::<NSString>().ok())
        .map_or_else(|| error.localizedDescription().to_string(), |m| m.to_string())
}

/// Fecha a página de verdade. O `Drop` do wry tira a WKWebView da janela mas segura uma referência
/// extra de propósito, então sem isto a página removida continuaria rodando (e conectada ao jogo).
pub fn close_page(webview: &Webview) {
    let _ = webview.with_webview(|platform| unsafe {
        let object = &*(platform.inner() as *const AnyObject);
        if responds(object, sel!(_close)) {
            let _: () = msg_send![object, _close];
        } else {
            let webview = &*(platform.inner() as *const WKWebView);
            webview.stopLoading();
            webview.loadHTMLString_baseURL(ns_string!(""), None);
        }
    });
}

/// Apaga o conteúdo do data store (cookies, localStorage, cache...) na hora, mesmo em uso.
pub async fn wipe_data_store(app: &AppHandle, id: Uuid) -> Result<(), String> {
    let (reply, rx) = Reply::<()>::new();
    app.run_on_main_thread(move || unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let store = WKWebsiteDataStore::dataStoreForIdentifier(&ns_uuid(id), mtm);
        let done = RcBlock::new(move || reply.send(()));
        store.removeDataOfTypes_modifiedSince_completionHandler(
            &WKWebsiteDataStore::allWebsiteDataTypes(mtm),
            &NSDate::distantPast(),
            &done,
        );
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "o WebKit não respondeu".to_owned())
}

/// Os sites com dados guardados no data store, com os tipos de dado de cada um.
pub async fn data_records(app: &AppHandle, id: Uuid) -> Result<Vec<String>, String> {
    let (reply, rx) = Reply::<Vec<String>>::new();
    app.run_on_main_thread(move || unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let store = WKWebsiteDataStore::dataStoreForIdentifier(&ns_uuid(id), mtm);
        let done = RcBlock::new(move |records: NonNull<NSArray<WKWebsiteDataRecord>>| {
            let records = records.as_ref();
            reply.send(
                records
                    .iter()
                    .map(|r| {
                        let types: Vec<String> = r.dataTypes().iter().map(|t| t.to_string()).collect();
                        format!("{} {types:?}", r.displayName())
                    })
                    .collect(),
            );
        });
        store.fetchDataRecordsOfTypes_completionHandler(&WKWebsiteDataStore::allWebsiteDataTypes(mtm), &done);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "o WebKit não respondeu".to_owned())
}

/// Apaga o data store do disco. Só funciona com nenhuma WKWebView dele viva no processo, e como o wry
/// nunca libera as que fecha, na prática só no começo do app.
pub async fn remove_data_store(app: &AppHandle, id: Uuid) -> Result<(), String> {
    let (reply, rx) = Reply::<Result<(), String>>::new();
    app.run_on_main_thread(move || unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let done = RcBlock::new(move |error: *mut NSError| {
            reply.send(match error.as_ref() {
                Some(error) => Err(error.localizedDescription().to_string()),
                None => Ok(()),
            });
        });
        WKWebsiteDataStore::removeDataStoreForIdentifier_completionHandler(&ns_uuid(id), &done, mtm);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "o WebKit não respondeu".to_owned())?
}

/// Os cookies (nome, valor) do data store da conta cujo domínio termina em `domain`. Lê inclusive os
/// HttpOnly, como o `session_token` da Kick e o `auth-token` da Twitch, que a página não enxerga.
pub async fn cookies(app: &AppHandle, store: Uuid, domain: &str) -> Result<Vec<(String, String)>, String> {
    let (reply, rx) = Reply::<Vec<(String, String)>>::new();
    let domain = domain.trim_start_matches('.').to_owned();
    app.run_on_main_thread(move || unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let jar = WKWebsiteDataStore::dataStoreForIdentifier(&ns_uuid(store), mtm).httpCookieStore();
        let done = RcBlock::new(move |cookies: NonNull<NSArray<objc2_foundation::NSHTTPCookie>>| {
            let found = cookies
                .as_ref()
                .iter()
                .filter(|c| {
                    let host = c.domain().to_string();
                    let host = host.trim_start_matches('.');
                    host == domain || host.ends_with(&format!(".{domain}"))
                })
                .map(|c| (c.name().to_string(), c.value().to_string()))
                .collect();
            reply.send(found);
        });
        jar.getAllCookies(&done);
    })
    .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| "o WebKit não respondeu".to_owned())?
        .map_err(|_| "o WebKit não respondeu".to_owned())
}

thread_local! {
    static BLOCK_LIST: std::cell::RefCell<Option<Retained<objc2_web_kit::WKContentRuleList>>> = const { std::cell::RefCell::new(None) };
}

fn block_rules() -> String {
    let rules: Vec<serde_json::Value> = BLOCKED
        .iter()
        .map(|domain| {
            serde_json::json!({
                "trigger": { "url-filter": format!(r"^[a-z]+://([^/]+\.)?{}[:/]", domain.replace('.', r"\.")) },
                "action": { "type": "block" },
            })
        })
        .collect();
    serde_json::Value::Array(rules).to_string()
}

/// Compila a lista de bloqueio; as webviews criadas depois disso a recebem em `add_block_list`.
pub async fn compile_block_list(app: &AppHandle) {
    let (reply, rx) = Reply::<()>::new();
    let json = block_rules();
    let started = app.run_on_main_thread(move || unsafe {
        let mtm = MainThreadMarker::new_unchecked();
        let Some(store) = objc2_web_kit::WKContentRuleListStore::defaultStore(mtm) else {
            return reply.send(());
        };
        let done = RcBlock::new(move |list: *mut objc2_web_kit::WKContentRuleList, error: *mut NSError| {
            match Retained::retain(list) {
                Some(list) => BLOCK_LIST.with(|b| *b.borrow_mut() = Some(list)),
                None => eprintln!(
                    "[webkit] lista de bloqueio não compilou: {:?}",
                    error.as_ref().map(|e| e.localizedDescription().to_string())
                ),
            }
            reply.send(());
        });
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(ns_string!("deck-bloqueio")),
            Some(&NSString::from_str(&json)),
            Some(&done),
        );
    });
    if started.is_ok() {
        let _ = tokio::time::timeout(Duration::from_secs(10), rx).await;
    }
}

fn add_block_list(config: &WKWebViewConfiguration) {
    BLOCK_LIST.with(|b| {
        if let Some(list) = b.borrow().as_ref() {
            unsafe { config.userContentController().addContentRuleList(list) };
        }
    });
}

/// Muta o áudio da página inteira (API privada `_setPageMuted:`; não é App Store).
pub fn set_page_muted(webview: &Webview, muted: bool) {
    let _ = webview.with_webview(move |platform| unsafe {
        let webview = &*(platform.inner() as *const AnyObject);
        if responds(webview, sel!(_setPageMuted:)) {
            let state: usize = if muted { 1 } else { 0 };
            let _: () = msg_send![webview, _setPageMuted: state];
        }
    });
}

/// Sem detecção de oclusão, o WebKit continua tratando a página como visível mesmo com a janela fora
/// da tela ou coberta (API privada `_setWindowOcclusionDetectionEnabled:`).
pub fn disable_occlusion_detection(webview: &Webview) {
    let _ = webview.with_webview(move |platform| unsafe {
        let webview = &*(platform.inner() as *const AnyObject);
        let available = responds(webview, sel!(_setWindowOcclusionDetectionEnabled:));
        if available {
            let _: () = msg_send![webview, _setWindowOcclusionDetectionEnabled: Bool::NO];
        }
        if !available {
            eprintln!("[webkit] _setWindowOcclusionDetectionEnabled ausente");
        }
    });
}

unsafe fn responds(object: &AnyObject, selector: Sel) -> bool {
    let responds: Bool = unsafe { msg_send![object, respondsToSelector: selector] };
    responds.as_bool()
}

/// O processo da página (`-[WKWebView _webProcessIdentifier]`, privado). 0 = página ainda sem processo.
pub async fn web_process(webview: &Webview) -> Option<u32> {
    let (tx, rx) = oneshot::channel();
    webview
        .with_webview(move |platform| unsafe {
            let object = &*(platform.inner() as *const AnyObject);
            let pid: i32 = if responds(object, sel!(_webProcessIdentifier)) { msg_send![object, _webProcessIdentifier] } else { 0 };
            let _ = tx.send(pid);
        })
        .ok()?;
    let pid = tokio::time::timeout(Duration::from_secs(2), rx).await.ok()?.ok()?;
    (pid > 0).then_some(pid as u32)
}
