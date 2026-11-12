//! Lives da Twitch e da Kick para os pontos dos canais (o bônus das lives no jogo). Cada conta marcada
//! tem uma janela estacionada fora da tela com o player das primeiras lives da lista que estão no ar,
//! no data store da própria conta (onde ficam os logins da Twitch e da Kick), mudas.

use crate::{
    accounts::USER_AGENT,
    automation,
    config::{LiveLimit, Site, SlotChoice},
    deck::{self, Deck},
    live_check, official, webkit,
};
use objc2::MainThreadMarker;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    utils::config::BackgroundThrottlingPolicy, webview::PageLoadEvent, AppHandle, Emitter,
    LogicalPosition, LogicalSize, Manager, Rect, WebviewBuilder, WebviewUrl, WindowBuilder,
    WindowEvent,
};
use uuid::Uuid;

const VISIBLE_JS: &str = include_str!("../scripts/visible.js");
const MUTE_JS: &str = "addEventListener('play', (e) => { if (e.target instanceof HTMLMediaElement) e.target.muted = true; }, true);";

/// Clica a confirmação de conteúdo adulto da Twitch, dá play se pausou e diz se o vídeo anda.
const PLAY_JS: &str = "return (() => { document.querySelector('[data-a-target=\"player-overlay-mature-accept\"], [data-a-target=\"content-classification-gate-overlay-start-watching-button\"]')?.click(); const v = document.querySelector('video'); if (!v) return 'sem'; if (v.paused) v.play().catch(() => {}); return [v.paused, v.readyState, v.currentTime].join(' '); })();";

const TWITCH_LOGIN: &str = "https://www.twitch.tv/login";
const KICK_LOGIN: &str = "https://kick.com/";
const CHECK_EVERY: Duration = Duration::from_secs(120);
const WATCH_EVERY: Duration = Duration::from_secs(30);
const WINDOW_SIZE: (f64, f64) = (1100.0, 760.0);
const TAB_BAR: f64 = 40.0;

pub fn player_url(site: Site, channel: &str) -> String {
    match site {
        Site::Twitch => format!("https://player.twitch.tv/?channel={channel}&parent=twitch.tv&player=popout"),
        Site::Kick => format!("https://player.kick.com/{channel}"),
    }
}

#[derive(Clone, Debug)]
struct Slot {
    label: String,
    /// `None` é uma página fixa de entrar (`login`).
    site: Option<Site>,
    login: Option<Site>,
    channel: Option<String>,
    url: Option<String>,
    loaded: bool,
    video_time: Option<f64>,
    stalls: u32,
    retries: u32,
    reloads: u32,
    playing: Option<bool>,
    why: Option<live_check::Why>,
}

#[derive(Clone, Debug)]
struct Chat {
    label: String,
    site: Site,
}

struct LiveWindow {
    slots: Vec<Slot>,
    /// Modo "Pontos": a página do canal de cada live, lado a lado, só com o botão dos pontos.
    points: bool,
    full_page: bool,
    chats: Vec<Chat>,
    revealed: bool,
    selected: usize,
    twitch_login: Option<bool>,
    kick_login: Option<bool>,
}

#[derive(Default)]
pub struct Lives {
    windows: Mutex<HashMap<Uuid, LiveWindow>>,
    live: Mutex<HashMap<(Site, String), Option<bool>>>,
    answered: Mutex<HashSet<Site>>,
    check_now: AtomicBool,
    last_check: Mutex<Option<Instant>>,
}

fn window_label(id: Uuid) -> String {
    format!("lives-{}", id.simple())
}

fn ui_label(id: Uuid) -> String {
    format!("livesui-{}", id.simple())
}

pub fn account_of_window(label: &str) -> Option<Uuid> {
    label
        .strip_prefix("lives-")
        .or_else(|| label.strip_prefix("livesui-"))
        .and_then(|s| Uuid::parse_str(s).ok())
}

pub fn has_window(app: &AppHandle, id: Uuid) -> bool {
    app.state::<Lives>().windows.lock().unwrap().contains_key(&id)
}

/// Logo à direita do monitor mais à direita, como o original no Windows.
fn parked_position(app: &AppHandle) -> (f64, f64) {
    let right = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| (m.position().x as f64 + m.size().width as f64) / m.scale_factor())
        .fold(0.0, f64::max);
    (right + 100.0, 0.0)
}

pub fn start(app: &AppHandle) {
    let ids = app.state::<Deck>().read(|c| c.lives_accounts());
    for id in ids {
        open(app, id);
    }
    tauri::async_runtime::spawn(check_loop(app.clone()));
    tauri::async_runtime::spawn(watch_loop(app.clone()));
}

/// Abre a janela de lives da conta (main thread). Conta desligada ou sem canais: nada.
pub fn open(app: &AppHandle, id: Uuid) {
    if has_window(app, id) {
        return;
    }
    let Some(account) = app.state::<Deck>().read(|c| c.accounts.iter().find(|a| a.id == id).cloned()) else {
        return;
    };
    let (twitch, kick) = (official::slots(app, Site::Twitch), official::slots(app, Site::Kick));
    if account.off || twitch + kick == 0 {
        return;
    }
    let (x, y) = parked_position(app);
    // Sem barra de título enquanto fora da tela: o macOS puxa de volta janela com barra que sai da tela.
    let window = match WindowBuilder::new(app, window_label(id))
        .title(format!("Lives — {}", account.name))
        .decorations(false)
        .inner_size(WINDOW_SIZE.0, WINDOW_SIZE.1)
        .position(x, y)
        .focused(false)
        .skip_taskbar(true)
        .build()
    {
        Ok(window) => window,
        Err(err) => return eprintln!("[lives] não abriu a janela de {}: {err}", account.name),
    };
    let _ = window.add_child(
        WebviewBuilder::new(ui_label(id), WebviewUrl::App("lives.html".into())),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(WINDOW_SIZE.0, WINDOW_SIZE.1),
    );

    let mut sites: Vec<(Option<Site>, Option<Site>)> = Vec::new();
    if twitch > 0 {
        sites.push((None, Some(Site::Twitch)));
        sites.extend(std::iter::repeat_n((Some(Site::Twitch), None), twitch));
    }
    if kick > 0 {
        sites.push((None, Some(Site::Kick)));
        sites.extend(std::iter::repeat_n((Some(Site::Kick), None), kick));
    }
    let mtm = MainThreadMarker::new().expect("lives criadas fora da main thread");
    let mut slots = Vec::new();
    for (index, (site, login)) in sites.into_iter().enumerate() {
        let label = format!("live-{}-{index}", id.simple());
        let handle_label = label.clone();
        let config = webkit::configuration(mtm, Some(id), true, true);
        webkit::add_block_list(&config);
        let builder = WebviewBuilder::new(&label, WebviewUrl::External("about:blank".parse().unwrap()))
            .with_webview_configuration(config)
            .user_agent(USER_AGENT)
            .background_throttling(BackgroundThrottlingPolicy::Disabled)
            .initialization_script(VISIBLE_JS)
            .initialization_script(MUTE_JS)
            .initialization_script(automation::MOTION_JS.replace("__DECK_REDUCE_MOTION__", "true"))
            .on_page_load(move |webview, payload| {
                let blank = payload.url().scheme() == "about";
                let finished = matches!(payload.event(), PageLoadEvent::Finished);
                let app = webview.app_handle();
                with_slot(app, id, &handle_label, |slot| slot.loaded = finished && !blank && slot.url.is_some());
                // Entrar na Twitch/Kick termina numa página nova: confere o login na hora, sem esperar a
                // próxima conferência.
                if finished && !blank && login.is_some() {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move { refresh_login(&app, id).await });
                }
            });
        match window.add_child(builder, LogicalPosition::new(0.0, TAB_BAR), LogicalSize::new(640.0, 360.0)) {
            Ok(webview) => {
                webkit::set_page_muted(&webview, true);
                webkit::disable_occlusion_detection(&webview);
            }
            Err(err) => eprintln!("[lives] não criou o player {label}: {err}"),
        }
        slots.push(Slot {
            label,
            site,
            login,
            channel: None,
            url: None,
            loaded: false,
            video_time: None,
            stalls: 0,
            retries: 0,
            reloads: 0,
            playing: None,
            why: None,
        });
    }
    app.state::<Lives>().windows.lock().unwrap().insert(
        id,
        LiveWindow {
            slots,
            points: false,
            full_page: false,
            chats: Vec::new(),
            revealed: false,
            selected: 0,
            twitch_login: None,
            kick_login: None,
        },
    );

    let handle = app.clone();
    window.on_window_event(move |event| match event {
        // Fechar no X só tira da tela: as lives continuam juntando pontos.
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            park(&handle, id);
        }
        WindowEvent::Resized(_) => {
            remember_points_height(&handle, id);
            layout(&handle, id);
        }
        _ => {}
    });
    layout(app, id);
    apply(app, id);
    app.state::<Lives>().check_now.store(true, Ordering::Relaxed);
    if app.try_state::<crate::points::Points>().is_some() {
        crate::points::delay(app);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { refresh_login(&app, id).await });
}

/// Fecha a janela e os players da conta (lives desligadas na conta, conta desligada ou removida).
pub fn close(app: &AppHandle, id: Uuid) {
    let Some(state) = app.state::<Lives>().windows.lock().unwrap().remove(&id) else { return };
    close_chats(app, &state.chats);
    for slot in &state.slots {
        if let Some(webview) = app.get_webview(&slot.label) {
            webkit::close_page(&webview);
            let _ = webview.close();
        }
    }
    if let Some(window) = app.get_window(&window_label(id)) {
        let _ = window.destroy();
    }
    emit_summary(app);
}

fn with_slot(app: &AppHandle, id: Uuid, label: &str, change: impl FnOnce(&mut Slot)) {
    let lives = app.state::<Lives>();
    let mut windows = lives.windows.lock().unwrap();
    if let Some(slot) = windows.get_mut(&id).and_then(|w| w.slots.iter_mut().find(|s| s.label == label)) {
        change(slot);
    }
}

/// Fora da tela todos os players ficam empilhados à vista; na tela, só a aba escolhida aparece (as
/// outras seguem tocando escondidas).
fn layout(app: &AppHandle, id: Uuid) {
    let Some(window) = app.get_window(&window_label(id)) else { return };
    let (Ok(scale), Ok(size)) = (window.scale_factor(), window.inner_size()) else { return };
    let size = size.to_logical::<f64>(scale);
    let inset = window.ns_window().map_or(0.0, deck::titlebar_inset_of);
    if let Some(ui) = app.get_webview(&ui_label(id)) {
        let _ = ui.set_bounds(Rect {
            position: LogicalPosition::new(0.0, inset).into(),
            size: LogicalSize::new(size.width, size.height - inset).into(),
        });
    }
    let content = Rect {
        position: LogicalPosition::new(0.0, inset + TAB_BAR).into(),
        size: LogicalSize::new(size.width, (size.height - inset - TAB_BAR).max(0.0)).into(),
    };
    let lives = app.state::<Lives>();
    let windows = lives.windows.lock().unwrap();
    let Some(state) = windows.get(&id) else { return };
    let points = state.points && !state.chats.is_empty();
    for (index, slot) in state.slots.iter().enumerate() {
        let Some(webview) = app.get_webview(&slot.label) else { continue };
        let _ = webview.set_bounds(content);
        let visible = !points && slot.url.is_some() && (!state.revealed || index == state.selected);
        let _ = if visible { webview.show() } else { webview.hide() };
    }
    if points {
        let height = (size.height - inset - TAB_BAR).max(0.0);
        let width = size.width / state.chats.len() as f64;
        for (index, chat) in state.chats.iter().enumerate() {
            let Some(webview) = app.get_webview(&chat.label) else { continue };
            let _ = webview.set_bounds(Rect {
                position: LogicalPosition::new(index as f64 * width, inset + TAB_BAR).into(),
                size: LogicalSize::new(width, height).into(),
            });
            // A Kick só mostra o botão dos pontos com a página larga (desktop): o zoom encolhe a página
            // para ela caber na coluna com esse layout.
            if chat.site == Site::Kick {
                let _ = webview.set_zoom((width / KICK_DESKTOP_WIDTH).clamp(0.35, 1.0));
            }
            let _ = webview.show();
        }
    }
    drop(windows);
    let _ = app.emit_to(ui_label(id).as_str(), "lives-ui", ui_state(app, id));
}

pub fn park(app: &AppHandle, id: Uuid) {
    let chats = match app.state::<Lives>().windows.lock().unwrap().get_mut(&id) {
        Some(state) => {
            state.revealed = false;
            state.points = false;
            std::mem::take(&mut state.chats)
        }
        None => Vec::new(),
    };
    // As páginas dos pontos custam ~200 MB cada: só existem com a janela na tela.
    close_chats(app, &chats);
    if let Some(window) = app.get_window(&window_label(id)) {
        let (x, y) = parked_position(app);
        let _ = window.set_decorations(false);
        let _ = window.set_position(LogicalPosition::new(x, y));
    }
    layout(app, id);
}

/// "Mostrar as lives": traz a janela para perto do deck (para entrar na Twitch/Kick ou ver os players).
pub fn reveal(app: &AppHandle, id: Uuid, order: usize) {
    {
        let lives = app.state::<Lives>();
        let mut windows = lives.windows.lock().unwrap();
        let Some(state) = windows.get_mut(&id) else { return };
        state.revealed = true;
        if state.slots.get(state.selected).is_none_or(|s| s.url.is_none()) {
            state.selected = state.slots.iter().position(|s| s.url.is_some()).unwrap_or(0);
        }
    }
    if let (Some(window), Some(main)) = (app.get_window(&window_label(id)), deck::main_window(app)) {
        let _ = window.set_decorations(true);
        if let (Ok(scale), Ok(position), Ok(size)) = (main.scale_factor(), main.outer_position(), main.outer_size()) {
            let position = position.to_logical::<f64>(scale);
            let size = size.to_logical::<f64>(scale);
            let x = position.x + ((size.width - WINDOW_SIZE.0) / 2.0).max(0.0) + order as f64 * 40.0;
            let y = position.y + ((size.height - WINDOW_SIZE.1) / 2.0).max(0.0) + order as f64 * 40.0;
            let _ = window.set_position(LogicalPosition::new(x, y));
        }
        let _ = window.set_focus();
    }
    layout(app, id);
}

pub fn reveal_all(app: &AppHandle) {
    let ids: Vec<Uuid> = app.state::<Lives>().windows.lock().unwrap().keys().copied().collect();
    let order = app.state::<Deck>().accounts();
    let mut ids: Vec<Uuid> = order.iter().map(|a| a.id).filter(|id| ids.contains(id)).collect();
    ids.dedup();
    for (index, id) in ids.into_iter().enumerate() {
        reveal(app, id, index);
    }
}

fn navigate(app: &AppHandle, slot: &mut Slot, url: Option<String>, channel: Option<String>) {
    if slot.channel != channel {
        slot.reloads = 0;
        slot.retries = 0;
    }
    slot.loaded = false;
    slot.video_time = None;
    slot.stalls = 0;
    slot.playing = None;
    slot.channel = channel;
    slot.url = url.clone();
    if let Some(webview) = app.get_webview(&slot.label) {
        let target = url.unwrap_or_else(|| "about:blank".to_owned());
        if let Ok(target) = target.parse() {
            let _ = webview.navigate(target);
        }
    }
}

/// Põe nas vagas de cada site as lives que devem tocar agora. Uma live que continua não recarrega.
fn apply(app: &AppHandle, id: Uuid) {
    let Some(limits) = app.state::<Deck>().read(|c| Some(c.accounts.iter().find(|a| a.id == id)?.lives.clone())) else {
        return;
    };
    let channels = [official::channels(app, Site::Twitch), official::channels(app, Site::Kick)];
    let lives = app.state::<Lives>();
    let live = lives.live.lock().unwrap().clone();
    let answered = lives.answered.lock().unwrap().clone();
    let mut windows = lives.windows.lock().unwrap();
    let Some(state) = windows.get_mut(&id) else { return };
    for (index, site) in [Site::Twitch, Site::Kick].into_iter().enumerate() {
        let no_login = site == Site::Twitch && state.twitch_login == Some(false);
        if !answered.contains(&site) && !no_login {
            continue;
        }
        let count = state.slots.iter().filter(|s| s.site == Some(site)).count();
        let limit = limits.limit(site);
        let wanted = if no_login {
            vec![None; count]
        } else {
            let status: HashMap<String, Option<bool>> = channels[index]
                .iter()
                .map(|c| (c.clone(), live.get(&(site, c.clone())).copied().flatten()))
                .collect();
            live_check::assign(&channels[index], &status, &limit.choices(count), &limit.closed)
        };
        // A vaga N é a N-ésima página do site na janela; um canal que continua na vaga não recarrega.
        for (slot, target) in state.slots.iter_mut().filter(|s| s.site == Some(site)).zip(wanted) {
            let channel = target.as_ref().map(|(c, _)| c.clone());
            slot.why = target.map(|(_, why)| why);
            if slot.channel != channel {
                let url = channel.as_ref().map(|c| player_url(site, c));
                navigate(app, slot, url, channel);
            }
        }
    }
    drop(windows);
    layout(app, id);
    emit_summary(app);
}

/// A lista oficial do jogo mudou: atualiza quem está no ar e as vagas. Se o número de vagas mudou (a
/// lista cresceu ou encolheu), as janelas reabrem; contas ainda sem janela ganham uma.
pub fn official_changed(app: &AppHandle) {
    let lives = app.state::<Lives>();
    for site in [Site::Twitch, Site::Kick] {
        let Some(status) = official::live_status(app, site) else { continue };
        let mut live = lives.live.lock().unwrap();
        live.extend(status.into_iter().map(|(c, on)| ((site, c), Some(on))));
        drop(live);
        lives.answered.lock().unwrap().insert(site);
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let app = &handle;
        let wanted = [official::slots(app, Site::Twitch), official::slots(app, Site::Kick)];
        let stale = app.state::<Lives>().windows.lock().unwrap().values().any(|w| {
            [Site::Twitch, Site::Kick]
                .iter()
                .zip(wanted)
                .any(|(site, n)| w.slots.iter().filter(|s| s.site == Some(*site)).count() != n)
        });
        if stale {
            let ids: Vec<Uuid> = app.state::<Lives>().windows.lock().unwrap().keys().copied().collect();
            for id in ids {
                close(app, id);
            }
        }
        let ids = app.state::<Deck>().read(|c| {
            let wanted = c.lives_accounts();
            c.accounts.iter().filter(|a| wanted.contains(&a.id) && !a.off).map(|a| a.id).collect::<Vec<_>>()
        });
        for id in ids {
            open(app, id);
        }
        apply_all(app);
        crate::central::emit(app);
    });
}

pub fn apply_all(app: &AppHandle) {
    let ids: Vec<Uuid> = app.state::<Lives>().windows.lock().unwrap().keys().copied().collect();
    for id in ids {
        apply(app, id);
    }
}

pub fn refresh_logins(app: &AppHandle) {
    let ids: Vec<Uuid> = app.state::<Lives>().windows.lock().unwrap().keys().copied().collect();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for id in ids {
            refresh_login(&app, id).await;
        }
    });
}

pub async fn has_cookie(app: &AppHandle, id: Uuid, domain: &str, name: &str) -> bool {
    webkit::cookies(app, id, domain)
        .await
        .is_ok_and(|cookies| cookies.iter().any(|(n, v)| n == name && !v.is_empty()))
}

/// Com o login na conta, a página de entrar daquele site sai. Sem o da Twitch, os players dela esperam
/// (sem login a Twitch não dá pontos); os da Kick tocam, mas os pontos só vêm com login.
async fn refresh_login(app: &AppHandle, id: Uuid) {
    let twitch = has_cookie(app, id, "twitch.tv", "auth-token").await;
    let kick = has_cookie(app, id, "kick.com", "session_token").await;
    let changed = {
        let lives = app.state::<Lives>();
        let mut windows = lives.windows.lock().unwrap();
        let Some(state) = windows.get_mut(&id) else { return };
        let changed = state.twitch_login != Some(twitch) || state.kick_login != Some(kick);
        state.twitch_login = Some(twitch);
        state.kick_login = Some(kick);
        changed
    };
    if !changed {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        {
            let lives = handle.state::<Lives>();
            let mut windows = lives.windows.lock().unwrap();
            if let Some(state) = windows.get_mut(&id) {
                for slot in state.slots.iter_mut().filter(|s| s.login.is_some()) {
                    let url = match slot.login {
                        Some(Site::Twitch) => (!twitch).then(|| TWITCH_LOGIN.to_owned()),
                        _ => (!kick).then(|| KICK_LOGIN.to_owned()),
                    };
                    if slot.url != url {
                        navigate(&handle, slot, url, None);
                    }
                }
            }
        }
        apply(&handle, id);
    });
}

/// As páginas de live carregadas de um site na conta: (rótulo da webview, canal).
pub fn live_pages(app: &AppHandle, id: Uuid, site: Site) -> Vec<(String, String)> {
    let lives = app.state::<Lives>();
    let windows = lives.windows.lock().unwrap();
    windows
        .get(&id)
        .map(|w| {
            w.slots
                .iter()
                .filter(|s| s.site == Some(site) && s.loaded)
                .filter_map(|s| Some((s.label.clone(), s.channel.clone()?)))
                .collect()
        })
        .unwrap_or_default()
}

async fn check_loop(app: AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let lives = app.state::<Lives>();
        if lives.windows.lock().unwrap().is_empty() {
            continue;
        }
        let due = lives.last_check.lock().unwrap().is_none_or(|t| t.elapsed() >= CHECK_EVERY);
        if !due && !lives.check_now.swap(false, Ordering::Relaxed) {
            continue;
        }
        *lives.last_check.lock().unwrap() = Some(Instant::now());
        check(&app).await;
    }
}

async fn check(app: &AppHandle) {
    // Quem o jogo já diz se está no ar não precisa de conferência na rede; só os extras de fora da lista.
    let list = |site| {
        if official::slots(app, site) == 0 {
            return Vec::new();
        }
        let known = official::live_status(app, site).unwrap_or_default();
        official::channels(app, site).into_iter().filter(|c| !known.contains_key(c)).collect::<Vec<_>>()
    };
    let (twitch, kick) = (list(Site::Twitch), list(Site::Kick));
    let twitch_task = tauri::async_runtime::spawn_blocking(move || live_check::twitch_live(&twitch));
    let mut results: Vec<((Site, String), Option<bool>)> = live_check::kick_live(app, &kick)
        .await
        .into_iter()
        .map(|(c, v)| ((Site::Kick, c), v))
        .collect();
    app.state::<Lives>().answered.lock().unwrap().insert(Site::Kick);
    if let Ok(map) = twitch_task.await {
        results.extend(map.into_iter().map(|(c, v)| ((Site::Twitch, c), v)));
        app.state::<Lives>().answered.lock().unwrap().insert(Site::Twitch);
    }
    for site in [Site::Twitch, Site::Kick] {
        if let Some(status) = official::live_status(app, site) {
            results.extend(status.into_iter().map(|(c, on)| ((site, c), Some(on))));
        }
    }
    app.state::<Lives>().live.lock().unwrap().extend(results);

    let ids: Vec<Uuid> = app.state::<Lives>().windows.lock().unwrap().keys().copied().collect();
    for id in ids {
        refresh_login(app, id).await;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || apply_all(&handle));
}

/// A cada 30 s: o vídeo de cada live anda? A travada recarrega depois de 1 min, depois 2, depois a
/// cada 4 enquanto continuar travada (pausada, congelada, caiu e voltou, presa no Cloudflare).
async fn watch_loop(app: AppHandle) {
    loop {
        tokio::time::sleep(WATCH_EVERY).await;
        let targets: Vec<(Uuid, String, bool)> = app
            .state::<Lives>()
            .windows
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(id, w)| {
                w.slots
                    .iter()
                    .filter(|s| s.site.is_some() && s.channel.is_some())
                    .map(|s| (*id, s.label.clone(), s.loaded))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (id, label, loaded) in targets {
            let result = if !loaded {
                "carregando".to_owned()
            } else if let Some(webview) = app.get_webview(&label) {
                match webkit::call_async(&webview, PLAY_JS, Duration::from_secs(10)).await {
                    Ok(json) => serde_json::from_str::<String>(&json).unwrap_or_else(|_| "sem".into()),
                    Err(_) => "travada".to_owned(),
                }
            } else {
                continue;
            };
            let mut reload = None;
            with_slot(&app, id, &label, |slot| {
                let parts: Vec<&str> = result.split(' ').collect();
                let time = parts.get(2).and_then(|t| t.parse::<f64>().ok());
                let moving = parts.len() == 3
                    && parts[0] == "false"
                    && time.is_some_and(|t| slot.video_time.is_none_or(|before| t > before));
                if time.is_some() {
                    slot.video_time = time;
                }
                if moving {
                    slot.stalls = 0;
                    slot.retries = 0;
                    slot.playing = Some(true);
                    return;
                }
                slot.playing = Some(false);
                slot.stalls += 1;
                if slot.stalls >= 2 << slot.retries.min(2) {
                    slot.reloads += 1;
                    slot.retries += 1;
                    reload = Some((slot.url.clone(), slot.channel.clone()));
                }
            });
            if let Some((url, channel)) = reload {
                let handle = app.clone();
                let label = label.clone();
                let _ = app.run_on_main_thread(move || {
                    let lives = handle.state::<Lives>();
                    let mut windows = lives.windows.lock().unwrap();
                    if let Some(slot) = windows.get_mut(&id).and_then(|w| w.slots.iter_mut().find(|s| s.label == label)) {
                        navigate(&handle, slot, url, channel);
                    }
                });
            }
        }
        emit_summary(&app);
    }
}

/// Uma vaga de live da conta, para a central: o canal que toca, por quê, e se o vídeo anda.
#[derive(Clone, Debug)]
pub struct SlotView {
    pub channel: Option<String>,
    pub why: Option<live_check::Why>,
    pub playing: Option<bool>,
}

pub fn slot_views(app: &AppHandle, id: Uuid, site: Site) -> Vec<SlotView> {
    let lives = app.state::<Lives>();
    let windows = lives.windows.lock().unwrap();
    windows
        .get(&id)
        .map(|w| {
            w.slots
                .iter()
                .filter(|s| s.site == Some(site))
                .map(|s| SlotView { channel: s.channel.clone(), why: s.why.clone(), playing: s.playing })
                .collect()
        })
        .unwrap_or_default()
}

/// `Some(true)` no ar, `Some(false)` offline, `None` ainda não se sabe.
pub fn live_status(app: &AppHandle, site: Site, channel: &str) -> Option<bool> {
    app.state::<Lives>().live.lock().unwrap().get(&(site, channel.to_owned())).copied().flatten()
}

/// O login da conta em cada site, como a última conferência viu (`None`: ainda não conferiu).
pub fn login(app: &AppHandle, id: Uuid, site: Site) -> Option<bool> {
    let lives = app.state::<Lives>();
    let windows = lives.windows.lock().unwrap();
    let state = windows.get(&id)?;
    match site {
        Site::Twitch => state.twitch_login,
        Site::Kick => state.kick_login,
    }
}

/// Mostra a janela de lives na aba de entrar naquele site.
pub fn reveal_login(app: &AppHandle, id: Uuid, site: Site) {
    if let Some(state) = app.state::<Lives>().windows.lock().unwrap().get_mut(&id) {
        if let Some(index) = state.slots.iter().position(|s| s.login == Some(site) && s.url.is_some()) {
            state.selected = index;
        }
    }
    reveal(app, id, 0);
}

/// (site, canal, endereço, tocando) de cada vaga da janela da conta, para o autoteste.
pub fn snapshot(app: &AppHandle, id: Uuid) -> Vec<(Option<Site>, Option<String>, Option<String>, Option<bool>)> {
    let lives = app.state::<Lives>();
    let windows = lives.windows.lock().unwrap();
    windows
        .get(&id)
        .map(|w| w.slots.iter().map(|s| (s.site, s.channel.clone(), s.url.clone(), s.playing)).collect())
        .unwrap_or_default()
}

/// Quantas páginas de pontos estão abertas na conta (autoteste).
pub fn chat_labels(app: &AppHandle, id: Uuid) -> Vec<String> {
    app.state::<Lives>().windows.lock().unwrap().get(&id).map(|w| w.chats.iter().map(|c| c.label.clone()).collect()).unwrap_or_default()
}

// ---------- Pontos na mão ----------

const KICK_DESKTOP_WIDTH: f64 = 1100.0;
/// A página dos pontos não precisa do vídeo: os pontos vêm do player da live, que continua tocando.
const PAUSE_JS: &str = "addEventListener('play', (e) => { if (e.target instanceof HTMLMediaElement) { e.target.muted = true; e.target.pause(); } }, true);";
const CHAT_COLUMN: f64 = 420.0;
const KICK_POINTS_CSS: &str = "#chatroom-messages, [data-testid=\"pinned-message-modal\"], [aria-label=\"Expand leaderboard\"] { visibility: hidden !important; }";
const TWITCH_POINTS_CSS: &str = ".chat-scrollable-area__message-container, .pinned-chat__message { visibility: hidden !important; }";

fn chat_url(site: Site, channel: &str) -> String {
    match site {
        Site::Kick => format!("https://kick.com/{channel}"),
        Site::Twitch => format!("https://www.twitch.tv/popout/{channel}/chat?popout="),
    }
}

fn chat_css(site: Site, full_page: bool) -> &'static str {
    match (site, full_page) {
        (_, true) => "",
        (Site::Kick, false) => KICK_POINTS_CSS,
        (Site::Twitch, false) => TWITCH_POINTS_CSS,
    }
}

fn chat_script(css: &str) -> String {
    format!(
        "let s = document.getElementById('deck-chat'); if (!s) {{ s = document.createElement('style'); s.id = 'deck-chat'; (document.head || document.documentElement).appendChild(s); }} s.textContent = {}; return 1;",
        serde_json::to_string(css).unwrap_or_default()
    )
}

fn close_chats(app: &AppHandle, chats: &[Chat]) {
    for chat in chats {
        if let Some(webview) = app.get_webview(&chat.label) {
            webkit::close_page(&webview);
            let _ = webview.close();
        }
    }
}

/// Liga ou desliga o modo "Pontos" na janela da conta (na tela). Main thread.
pub fn set_points(app: &AppHandle, id: Uuid, on: bool) {
    if on {
        reveal(app, id, 0);
    }
    let (old, lives_open) = {
        let lives = app.state::<Lives>();
        let mut windows = lives.windows.lock().unwrap();
        let Some(state) = windows.get_mut(&id) else { return };
        state.points = on;
        let open: Vec<(Site, String)> = state
            .slots
            .iter()
            .filter_map(|s| Some((s.site?, s.channel.clone()?)))
            .collect();
        (std::mem::take(&mut state.chats), open)
    };
    close_chats(app, &old);
    let mut chats = Vec::new();
    if on {
        let Some(window) = app.get_window(&window_label(id)) else { return };
        let mtm = MainThreadMarker::new().expect("pontos abertos fora da main thread");
        for (index, (site, channel)) in lives_open.iter().enumerate() {
            let label = format!("chat-{}-{index}", id.simple());
            let config = webkit::configuration(mtm, Some(id), false, true);
            webkit::add_block_list(&config);
            let site = *site;
            let url: tauri::Url = match chat_url(site, channel).parse() {
                Ok(url) => url,
                Err(_) => continue,
            };
            let builder = WebviewBuilder::new(&label, WebviewUrl::External(url))
                .with_webview_configuration(config)
                .user_agent(USER_AGENT)
                .initialization_script(PAUSE_JS)
                .on_page_load(move |webview, payload| {
                    if matches!(payload.event(), PageLoadEvent::Finished) {
                        let app = webview.app_handle().clone();
                        tauri::async_runtime::spawn(async move {
                            let full = app.state::<Lives>().windows.lock().unwrap().get(&id).is_some_and(|w| w.full_page);
                            let _ = webkit::call_async(&webview, &chat_script(chat_css(site, full)), Duration::from_secs(10)).await;
                        });
                    }
                });
            if let Ok(webview) = window.add_child(builder, LogicalPosition::new(0.0, TAB_BAR), LogicalSize::new(CHAT_COLUMN, 400.0)) {
                webkit::set_page_muted(&webview, true);
                chats.push(Chat { label, site });
            }
        }
        let height = app.state::<Deck>().read(|c| c.lives.points_height);
        let columns = chats.len().max(2) as f64;
        let _ = window.set_size(LogicalSize::new(columns * CHAT_COLUMN, height));
    }
    if let Some(state) = app.state::<Lives>().windows.lock().unwrap().get_mut(&id) {
        state.chats = chats;
    }
    layout(app, id);
}

/// "Página inteira": tira o corte das páginas dos pontos, se algo do chat ficar escondido.
fn set_full_page(app: &AppHandle, id: Uuid, full: bool) {
    let chats = {
        let lives = app.state::<Lives>();
        let mut windows = lives.windows.lock().unwrap();
        let Some(state) = windows.get_mut(&id) else { return };
        state.full_page = full;
        state.chats.clone()
    };
    for chat in chats {
        if let Some(webview) = app.get_webview(&chat.label) {
            let script = chat_script(chat_css(chat.site, full));
            tauri::async_runtime::spawn(async move {
                let _ = webkit::call_async(&webview, &script, Duration::from_secs(10)).await;
            });
        }
    }
    layout(app, id);
}

/// A altura da janela no modo "Pontos" fica guardada (vale para todas as contas).
fn remember_points_height(app: &AppHandle, id: Uuid) {
    let points = app.state::<Lives>().windows.lock().unwrap().get(&id).is_some_and(|w| w.points && w.revealed);
    if !points {
        return;
    }
    let Some(window) = app.get_window(&window_label(id)) else { return };
    let (Ok(scale), Ok(size)) = (window.scale_factor(), window.outer_size()) else { return };
    let height = size.to_logical::<f64>(scale).height.clamp(300.0, 3000.0);
    app.state::<Deck>().update(|c| c.lives.points_height = height);
}

#[tauri::command]
pub fn lives_ui_points(app: AppHandle, webview: tauri::Webview) {
    let Some(id) = account_of_window(webview.label()) else { return };
    let on = !app.state::<Lives>().windows.lock().unwrap().get(&id).is_some_and(|w| w.points);
    set_points(&app, id, on);
}

#[tauri::command]
pub fn lives_ui_full_page(app: AppHandle, webview: tauri::Webview) {
    let Some(id) = account_of_window(webview.label()) else { return };
    let full = !app.state::<Lives>().windows.lock().unwrap().get(&id).is_some_and(|w| w.full_page);
    set_full_page(&app, id, full);
}

// ---------- Fechar e abrir lives numa conta ----------

/// Tira um canal da lista desta conta (ou esvazia todas as vagas do site, com `None`).
pub fn close_live(app: &AppHandle, id: Uuid, site: Site, channel: Option<String>) {
    let slots = official::slots(app, site);
    app.state::<Deck>().update(|c| {
        let Some(account) = c.account_mut(id) else { return };
        let limit = account.lives.limit_mut(site);
        match &channel {
            Some(ch) => {
                if !limit.closed.contains(ch) {
                    limit.closed.push(ch.clone());
                }
            }
            None => (0..slots).for_each(|i| limit.set_choice(i, SlotChoice::Off)),
        }
    });
    apply(app, id);
    let text = match channel {
        Some(ch) => format!("{}: {ch} saiu da lista desta conta", site.name()),
        None => format!("{}: todas as vagas vazias nesta conta", site.name()),
    };
    automation::note(app, id, text);
}

pub fn reopen_live(app: &AppHandle, id: Uuid, site: Site, channel: Option<String>) {
    app.state::<Deck>().update(|c| {
        let Some(account) = c.account_mut(id) else { return };
        let limit = account.lives.limit_mut(site);
        match &channel {
            Some(ch) => limit.closed.retain(|c| c != ch),
            None => {
                limit.closed.clear();
                for choice in limit.chosen.iter_mut().filter(|c| **c == SlotChoice::Off) {
                    *choice = SlotChoice::Auto;
                }
            }
        }
    });
    app.state::<Lives>().check_now.store(true, Ordering::Relaxed);
    apply(app, id);
    let text = match channel {
        Some(ch) => format!("{}: {ch} volta para a lista (toca se estiver no ar)", site.name()),
        None => format!("{}: as lives voltam a abrir", site.name()),
    };
    automation::note(app, id, text);
}

/// O que toca na vaga `slot` do site nesta conta (a escolha vale na hora).
pub fn choose_slot(app: &AppHandle, id: Uuid, site: Site, slot: usize, choice: SlotChoice) {
    app.state::<Deck>().update(|c| {
        if let Some(account) = c.account_mut(id) {
            let limit = account.lives.limit_mut(site);
            // Um canal escolhido numa vaga sai das outras, e volta para a lista se estava fechado.
            if let SlotChoice::Channel(ch) = &choice {
                limit.closed.retain(|c| c != ch);
                for other in limit.chosen.iter_mut() {
                    if *other == SlotChoice::Channel(ch.clone()) {
                        *other = SlotChoice::Auto;
                    }
                }
            }
            limit.set_choice(slot, choice);
        }
    });
    apply(app, id);
}


/// Liga ou desliga as lives numa conta (o "Lives da Twitch/Kick nesta conta").
pub fn toggle_account(app: &AppHandle, id: Uuid) {
    let on = app.state::<Deck>().update(|c| {
        let on = !c.lives_accounts().contains(&id);
        c.set_lives_account(id, on);
        on
    });
    if on {
        open(app, id);
    } else {
        close(app, id);
    }
    deck::refresh(app);
}

/// A config das lives mudou: as janelas reabrem com os canais e as vagas novas.
pub fn restart(app: &AppHandle) {
    crate::points::restart(app);
    let lives = app.state::<Lives>();
    lives.live.lock().unwrap().clear();
    lives.answered.lock().unwrap().clear();
    let ids = app.state::<Deck>().read(|c| c.lives_accounts());
    let open_now: Vec<Uuid> = lives.windows.lock().unwrap().keys().copied().collect();
    for id in open_now {
        close(app, id);
    }
    for id in ids {
        open(app, id);
    }
}

// ---------- O que o deck mostra ----------

fn site_summary(
    state: &LiveWindow,
    site: Site,
    limit: &LiveLimit,
    max: usize,
    answered: bool,
    points: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if max == 0 {
        return None;
    }
    let open: Vec<&Slot> = state.slots.iter().filter(|s| s.site == Some(site) && s.channel.is_some()).collect();
    let name = site.name();
    Some(if site == Site::Twitch && state.twitch_login == Some(false) {
        "Twitch: entre na Twitch (Lives → Mostrar as lives)".to_owned()
    } else if !open.is_empty() {
        let lives: Vec<String> = open
            .iter()
            .map(|s| {
                let mark = match s.playing {
                    Some(true) => " ▶",
                    Some(false) => " ‖",
                    None => "",
                };
                let reloads = if s.reloads > 0 { format!(" ⟳{}", s.reloads) } else { String::new() };
                let channel = s.channel.as_deref().unwrap_or("");
                let pts = points(channel).map(|p| format!(" ({p})")).unwrap_or_default();
                format!("{channel}{mark}{reloads}{pts}")
            })
            .collect();
        format!("{name} no ar: {}", lives.join(", "))
    } else if (0..max).all(|i| limit.choice(i) == SlotChoice::Off) {
        format!("{name}: todas fechadas nesta conta")
    } else if !answered {
        format!("{name}: conferindo quem está no ar…")
    } else {
        format!("{name}: nenhuma live da lista no ar")
    })
}

pub fn summary(app: &AppHandle, id: Uuid) -> Option<String> {
    let (limits, maxes, chest) = app.state::<Deck>().read(|c| {
        let account = c.accounts.iter().find(|a| a.id == id)?;
        Some((account.lives.clone(), c.lives.claim_chest))
    })
    .map(|(limits, chest)| (limits, [official::slots(app, Site::Twitch), official::slots(app, Site::Kick)], chest))?;
    // Os pontos de cada live só aparecem com o resgate (ou o baú, na Twitch) ligado naquele site.
    let shows_points = |site: Site| match site {
        Site::Twitch => limits.redeem_twitch.is_some() || chest,
        Site::Kick => limits.redeem_kick.is_some(),
    };
    let extra: Vec<String> = [Site::Twitch, Site::Kick]
        .into_iter()
        .filter(|s| shows_points(*s))
        .filter_map(|s| crate::points::account_text(app, id, s))
        .collect();
    let lives = app.state::<Lives>();
    let answered = lives.answered.lock().unwrap().clone();
    let windows = lives.windows.lock().unwrap();
    let state = windows.get(&id)?;
    let mut parts: Vec<String> = [Site::Twitch, Site::Kick]
        .into_iter()
        .zip(maxes)
        .filter_map(|(site, max)| {
            let points = |channel: &str| shows_points(site).then(|| crate::points::channel_text(app, id, site, channel)).flatten();
            site_summary(state, site, limits.limit(site), max, answered.contains(&site), &points)
        })
        .collect();
    parts.extend(extra);
    Some(parts.join(" · "))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLivesStatus {
    id: Uuid,
    playing: usize,
    redeemable: usize,
    /// Falta login num site que tem vagas.
    warn: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LivesStatus {
    on: bool,
    playing: usize,
    open: usize,
    redeemable: usize,
    text: String,
    accounts: Vec<AccountLivesStatus>,
}

pub fn emit_summary(app: &AppHandle) {
    let open_ids: Vec<Uuid> = app.state::<Lives>().windows.lock().unwrap().keys().copied().collect();
    let ids: Vec<Uuid> = app.state::<Deck>().accounts().iter().map(|a| a.id).filter(|id| open_ids.contains(id)).collect();
    let mut per_site = [(0usize, 0usize); 2];
    let mut accounts = Vec::new();
    for id in &ids {
        for (index, site) in [Site::Twitch, Site::Kick].into_iter().enumerate() {
            let slots = slot_views(app, *id, site);
            per_site[index].0 += slots.iter().filter(|s| s.channel.is_some()).count();
            per_site[index].1 += slots.len();
        }
        let (playing, _, redeemable, warn) = crate::central::account_counts(app, *id);
        accounts.push(AccountLivesStatus { id: *id, playing, redeemable, warn });
    }
    let playing: usize = per_site.iter().map(|s| s.0).sum();
    let open: usize = per_site.iter().map(|s| s.1).sum();
    let redeemable: usize = accounts.iter().map(|a| a.redeemable).sum();
    let mut parts: Vec<String> = [Site::Twitch, Site::Kick]
        .into_iter()
        .zip(per_site)
        .filter(|(_, (_, total))| *total > 0)
        .map(|(site, (on, total))| format!("{} {on}/{total}", site.name()))
        .collect();
    let text = if ids.is_empty() {
        "Lives desligadas".to_owned()
    } else {
        let mut text = format!("{} tocando", parts.join(" · "));
        if redeemable > 0 {
            text.push_str(&format!(" · {redeemable} {} para resgatar", if redeemable == 1 { "canal pronto" } else { "canais prontos" }));
        }
        text
    };
    parts.clear();
    let status = LivesStatus { on: !ids.is_empty(), playing, open, redeemable, text, accounts };
    let _ = app.emit_to("deck", "lives", status);
    crate::central::emit(app);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiTab {
    title: String,
    selected: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    name: String,
    tabs: Vec<UiTab>,
    revealed: bool,
    summary: String,
    points: bool,
    full_page: bool,
    can_points: bool,
}

fn ui_state(app: &AppHandle, id: Uuid) -> Option<UiState> {
    let name = app.state::<Deck>().accounts().into_iter().find(|a| a.id == id)?.name;
    let summary = summary(app, id).unwrap_or_default();
    let lives = app.state::<Lives>();
    let windows = lives.windows.lock().unwrap();
    let state = windows.get(&id)?;
    let tabs = state
        .slots
        .iter()
        .enumerate()
        .filter(|(_, s)| s.url.is_some())
        .map(|(index, s)| UiTab {
            title: match (s.site, &s.channel, s.login) {
                (Some(site), Some(channel), _) => format!("{} · {channel}", site.name()),
                (_, _, Some(site)) => format!("{} · entrar", site.name()),
                _ => String::new(),
            },
            selected: index == state.selected,
        })
        .collect();
    let can_points = state.slots.iter().any(|s| s.site.is_some() && s.channel.is_some());
    Some(UiState {
        name,
        tabs,
        revealed: state.revealed,
        summary,
        points: state.points,
        full_page: state.full_page,
        can_points,
    })
}

#[tauri::command]
pub fn lives_ui_state(app: AppHandle, webview: tauri::Webview) -> Option<UiState> {
    ui_state(&app, account_of_window(webview.label())?)
}

/// A aba `index` entre as abertas (as que têm página).
#[tauri::command]
pub fn lives_ui_select(app: AppHandle, webview: tauri::Webview, index: usize) {
    let Some(id) = account_of_window(webview.label()) else { return };
    if app.state::<Lives>().windows.lock().unwrap().get(&id).is_some_and(|w| w.points) {
        set_points(&app, id, false);
    }
    if let Some(state) = app.state::<Lives>().windows.lock().unwrap().get_mut(&id) {
        if let Some((slot, _)) = state.slots.iter().enumerate().filter(|(_, s)| s.url.is_some()).nth(index) {
            state.selected = slot;
        }
    }
    layout(&app, id);
}

#[tauri::command]
pub fn lives_ui_park(app: AppHandle, webview: tauri::Webview) {
    if let Some(id) = account_of_window(webview.label()) {
        park(&app, id);
    }
}
