//! Spike do player da Kick fora da tela: abre o mesmo canal em formas diferentes de esconder a
//! webview e confere a cada 30 s qual continua tocando.

use crate::{deck, webkit};
use objc2::MainThreadMarker;
use serde::Serialize;
use std::{
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{
    utils::config::BackgroundThrottlingPolicy, AppHandle, Emitter, LogicalPosition, LogicalSize,
    Manager, WebviewBuilder, WebviewUrl, WebviewWindowBuilder,
};

#[derive(Clone, Copy, PartialEq)]
enum Host {
    VisibleWindow,
    OffscreenWindow,
    HiddenChild,
}

struct Variant {
    key: &'static str,
    host: Host,
    occlusion_detection: bool,
}

const VARIANTS: [Variant; 3] = [
    Variant { key: "janela-fora", host: Host::OffscreenWindow, occlusion_detection: true },
    Variant { key: "janela-fora-sem-oclusao", host: Host::OffscreenWindow, occlusion_detection: false },
    Variant { key: "filha-oculta", host: Host::HiddenChild, occlusion_detection: false },
];

const VISIBLE_JS: &str = include_str!("../scripts/visible.js");

const CHECK_EVERY: Duration = Duration::from_secs(30);

/// Mudo por JS também, caso o `_setPageMuted:` some numa versão do macOS.
const MUTE_JS: &str = "addEventListener('play', (e) => { if (e.target instanceof HTMLMediaElement) e.target.muted = true; }, true);\n\
    window.__deckErrors = [];\n\
    addEventListener('error', (e) => window.__deckErrors.push(String(e.message || (e.target && e.target.src) || e)), true);\n\
    addEventListener('unhandledrejection', (e) => window.__deckErrors.push('rejection: ' + String(e.reason)));";

const STATUS_JS: &str = "const v = document.querySelector('video');\n\
    const page = { title: document.title, text: (document.body ? document.body.innerText : '').slice(0, 120).replace(/\\s+/g, ' '),\n\
      mse: typeof MediaSource, mms: typeof ManagedMediaSource, errors: (window.__deckErrors || []).slice(-3) };\n\
    if (!v) return { video: false, visibility: document.visibilityState, page };\n\
    return { video: true, paused: v.paused, time: v.currentTime, ready: v.readyState, muted: v.muted,\n\
             visibility: document.visibilityState + '/' + (window.__deckRealVisibility ? window.__deckRealVisibility() : '?'), width: v.videoWidth, network: v.networkState,\n\
             src: (v.currentSrc || '').slice(0, 60), error: v.error && v.error.code, page };";

const RESUME_JS: &str = "const v = document.querySelector('video');\n\
    if (!v) return 'sem vídeo';\n\
    try { await v.play(); return v.paused ? 'continua pausado' : 'ok'; } catch (e) { return String(e); }";

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStatus {
    key: String,
    playing: Option<bool>,
    current_time: f64,
    checks: u32,
    stopped_checks: u32,
    resumes: u32,
    detail: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KickSnapshot {
    channel: String,
    elapsed_secs: u64,
    players: Vec<PlayerStatus>,
}

pub struct Kick {
    channel: String,
    started: Instant,
    players: Mutex<Vec<PlayerStatus>>,
    log: PathBuf,
}

impl Kick {
    fn snapshot(&self) -> KickSnapshot {
        KickSnapshot {
            channel: self.channel.clone(),
            elapsed_secs: self.started.elapsed().as_secs(),
            players: self.players.lock().unwrap().clone(),
        }
    }

    fn write_log(&self, line: &str) {
        let secs = self.started.elapsed().as_secs();
        let line = format!("[{:02}:{:02}] {line}", secs / 60, secs % 60);
        println!("[kick] {line}");
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&self.log) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn webview_label(variant: &Variant) -> String {
    format!("kick-{}", variant.key)
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    if std::env::var("DECK_KICK").as_deref() == Ok("0") {
        return Ok(());
    }
    let channel = std::env::var("DECK_KICK_CHANNEL").unwrap_or_else(|_| "gaules".into());
    let url: tauri::Url = format!("https://player.kick.com/{channel}").parse().expect("url do player");
    let mtm = MainThreadMarker::new().expect("setup roda na main thread");

    // Logo depois da borda direita do monitor mais à direita, como o original faz no Windows.
    let right_edge = app
        .available_monitors()?
        .iter()
        .map(|m| (m.position().x as f64 + m.size().width as f64) / m.scale_factor())
        .fold(0.0, f64::max);

    for (index, variant) in VARIANTS.iter().enumerate() {
        let label = webview_label(variant);
        let config = webkit::configuration(mtm, None, true);
        match variant.host {
            Host::VisibleWindow | Host::OffscreenWindow => {
                let x = if variant.host == Host::VisibleWindow { 60.0 } else { right_edge + 100.0 + index as f64 * 520.0 };
                let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url.clone()))
                    .with_webview_configuration(config)
                    .user_agent(deck::USER_AGENT)
                    .background_throttling(BackgroundThrottlingPolicy::Disabled)
                    .initialization_script(VISIBLE_JS)
                    .initialization_script(MUTE_JS)
                    .title(format!("Kick · {}", variant.key))
                    .decorations(false)
                    .skip_taskbar(true)
                    .focused(false)
                    .inner_size(480.0, 270.0)
                    .position(x, 60.0)
                    .build()?;
                println!("[kick] {label}: janela em {:?}", window.outer_position()?);
            }
            Host::HiddenChild => {
                let Some(main) = app.get_window("main") else { continue };
                let webview = main.add_child(
                    WebviewBuilder::new(&label, WebviewUrl::External(url.clone()))
                        .with_webview_configuration(config)
                        .user_agent(deck::USER_AGENT)
                        .background_throttling(BackgroundThrottlingPolicy::Disabled)
                        .initialization_script(VISIBLE_JS)
                    .initialization_script(MUTE_JS),
                    LogicalPosition::new(0.0, 0.0),
                    LogicalSize::new(480.0, 270.0),
                )?;
                webview.hide()?;
            }
        }
        if let Some(webview) = app.get_webview(&label) {
            webkit::set_page_muted(&webview, true);
            if !variant.occlusion_detection {
                webkit::disable_occlusion_detection(&webview);
            }
        }
    }

    let kick = Kick {
        channel: channel.clone(),
        started: Instant::now(),
        players: Mutex::new(
            VARIANTS
                .iter()
                .map(|v| PlayerStatus { key: v.key.into(), ..Default::default() })
                .collect(),
        ),
        log: deck::data_dir(app).join("kick-spike.log"),
    };
    kick.write_log(&format!("início: canal {channel}, variantes {}", VARIANTS.map(|v| v.key).join(", ")));
    app.manage(kick);
    tauri::async_runtime::spawn(monitor(app.clone()));
    Ok(())
}

async fn monitor(app: AppHandle) {
    tokio::time::sleep(Duration::from_secs(15)).await;
    loop {
        for (index, variant) in VARIANTS.iter().enumerate() {
            let Some(webview) = app.get_webview(&webview_label(variant)) else { continue };
            let status = webkit::call_async(&webview, STATUS_JS, Duration::from_secs(10)).await;
            let kick = app.state::<Kick>();
            let previous = kick.players.lock().unwrap()[index].clone();
            let mut next = previous.clone();
            next.checks += 1;
            let mut resume = None;
            match status.as_deref().map(serde_json::from_str::<serde_json::Value>) {
                Ok(Ok(v)) if v["video"] == true => {
                    let time = v["time"].as_f64().unwrap_or(0.0);
                    let paused = v["paused"].as_bool().unwrap_or(true);
                    let advanced = time > previous.current_time + 1.0;
                    let playing = !paused && advanced;
                    next.playing = Some(playing);
                    next.current_time = time;
                    next.detail = format!(
                        "t={time:.0}s (+{:.0}) paused={paused} ready={} visibility={} muted={} {}px",
                        time - previous.current_time,
                        v["ready"],
                        v["visibility"].as_str().unwrap_or("?"),
                        v["muted"],
                        v["width"],
                    );
                    if !playing {
                        next.detail.push_str(&format!(" network={} src={} error={} page={}", v["network"], v["src"], v["error"], v["page"]));
                    }
                    if !playing {
                        next.stopped_checks += 1;
                        if paused {
                            resume = Some(webkit::call_async(&webview, RESUME_JS, Duration::from_secs(10)).await);
                            next.resumes += 1;
                        }
                    }
                }
                Ok(Ok(v)) => {
                    next.playing = Some(false);
                    next.stopped_checks += 1;
                    next.detail = format!("sem <video> (visibility={}) page={}", v["visibility"].as_str().unwrap_or("?"), v["page"]);
                }
                Ok(Err(err)) => next.detail = format!("resposta inválida: {err}"),
                Err(err) => next.detail = format!("não respondeu: {err}"),
            }
            let symbol = match next.playing {
                Some(true) => "▶",
                Some(false) => "⏸",
                None => "?",
            };
            let mut line = format!("{} {symbol} {}", variant.key, next.detail);
            if let Some(resume) = resume {
                line.push_str(&format!(" → play(): {}", resume.unwrap_or_else(|e| e)));
            }
            kick.write_log(&line);
            kick.players.lock().unwrap()[index] = next;
        }
        let snapshot = app.state::<Kick>().snapshot();
        let _ = app.emit_to("deck", "kick", &snapshot);
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

#[tauri::command]
pub fn kick_status(app: AppHandle) -> Option<KickSnapshot> {
    app.try_state::<Kick>().map(|kick| kick.snapshot())
}
