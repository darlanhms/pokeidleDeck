//! O que o Tauri/wry não expõe do WKWebView.

use block2::RcBlock;
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool},
    sel, MainThreadMarker,
};
use objc2_foundation::{ns_string, NSError, NSString, NSUUID};
use objc2_web_kit::{
    WKAudiovisualMediaTypes, WKContentWorld, WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
use std::{sync::Mutex, time::Duration};
use tauri::Webview;
use uuid::Uuid;

/// Configuração com o data store persistente da conta. Passar a configuração pronta é o único jeito de
/// ligar o autoplay sem gesto, e com ela o wry deixa de aplicar o `data_store_identifier` sozinho.
pub fn configuration(
    mtm: MainThreadMarker,
    store: Option<Uuid>,
    autoplay: bool,
) -> Retained<WKWebViewConfiguration> {
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        let data_store = match store {
            Some(id) => {
                WKWebsiteDataStore::dataStoreForIdentifier(&NSUUID::from_bytes(*id.as_bytes()), mtm)
            }
            None => WKWebsiteDataStore::nonPersistentDataStore(mtm),
        };
        config.setWebsiteDataStore(&data_store);
        if autoplay {
            config.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::None);
        }
        config
    }
}

/// Roda `script` como corpo de uma função async na página (pode usar `await`) e devolve o valor
/// resolvido em JSON. É o `Runtime.evaluate` com `awaitPromise` do original.
pub async fn call_async(webview: &Webview, script: &str, timeout: Duration) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    // O valor volta como texto para não depender da conversão JS → NSDictionary do WebKit.
    let body = format!(
        "const __deckValue = await (async () => {{\n{script}\n}})();\n\
         return JSON.stringify(__deckValue === undefined ? null : __deckValue);"
    );
    webview
        .with_webview(move |platform| unsafe {
            let webview = &*(platform.inner() as *const WKWebView);
            let mtm = MainThreadMarker::new_unchecked();
            let tx = Mutex::new(Some(tx));
            let done = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                let result = if let Some(error) = error.as_ref() {
                    Err(describe(error))
                } else {
                    match value.as_ref().and_then(|v| v.downcast_ref::<NSString>()) {
                        Some(text) => Ok(text.to_string()),
                        None => Ok("null".to_owned()),
                    }
                };
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
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
    let message = error
        .userInfo()
        .objectForKey(ns_string!("WKJavaScriptExceptionMessage"))
        .and_then(|m| m.downcast::<NSString>().ok())
        .map(|m| m.to_string());
    match message {
        Some(message) => message,
        None => error.localizedDescription().to_string(),
    }
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
        if responds(webview, sel!(_setWindowOcclusionDetectionEnabled:)) {
            let _: () = msg_send![webview, _setWindowOcclusionDetectionEnabled: Bool::NO];
        }
    });
}

unsafe fn responds(object: &AnyObject, selector: objc2::runtime::Sel) -> bool {
    let responds: Bool = unsafe { msg_send![object, respondsToSelector: selector] };
    responds.as_bool()
}
