//! Os pontos dos canais: o saldo de cada live aberta, o baú de bônus da Twitch e o resgate automático do
//! bônus do PokeIdle (Kick a cada 5 min, Twitch a cada 2). Os pedidos saem de dentro do player da live,
//! com o login da conta; na Twitch, o `fetch` da página carrega a checagem anti-robô.

use crate::{
    automation,
    config::{Reward, Site},
    deck::{self, Deck},
    live_check::TWITCH_CLIENT_ID,
    lives, webkit,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

const KICK_JS: &str = include_str!("../scripts/kick-points.js");
const TWITCH_JS: &str = include_str!("../scripts/twitch-points.js");
const KICK_EVERY: Duration = Duration::from_secs(5 * 60);
const TWITCH_EVERY: Duration = Duration::from_secs(2 * 60);
const FIRST_RUN: Duration = Duration::from_secs(60);
/// Resgate ou baú recusado: tenta de novo naquele canal depois disto.
const RETRY_WAIT: Duration = Duration::from_secs(30 * 60);
/// Se os pontos de um resgate voltam (site não vinculado no jogo), voltam dentro disto.
const REFUND_WINDOW: Duration = Duration::from_secs(30 * 60);
const LOG_KEEP: usize = 2000;
const LOG_MAX: usize = 4000;

fn word(reward: Reward) -> &'static str {
    match reward {
        Reward::Xp => r"\bxp\b",
        Reward::Capture => "captur",
        Reward::Lure => "lure|shiny",
    }
}

#[derive(Clone, Default)]
struct Channel {
    points: Option<i64>,
    cost: Option<i64>,
    paused: bool,
    redeemed_at: Option<Instant>,
    redeemed_clock: Option<String>,
    refused: Option<String>,
    retry_at: Option<Instant>,
    claimed_clock: Option<String>,
    claim_error: Option<String>,
    claim_retry_at: Option<Instant>,
    text: String,
}

#[derive(Default)]
struct Redeemer {
    /// Parou de gastar nesta conta e site (os pontos voltaram); salvar a config volta a resgatar.
    stopped: Option<String>,
    no_login: bool,
    channels: HashMap<String, Channel>,
}

#[derive(Default)]
pub struct Points {
    redeemers: Mutex<HashMap<(Uuid, Site), Redeemer>>,
    next: Mutex<HashMap<Site, Instant>>,
    log: Mutex<Vec<String>>,
}

fn log_path(app: &AppHandle) -> PathBuf {
    deck::data_dir(app).join("resgates.log")
}

/// "dd/MM/yy HH:mm" e "HH:mm" na hora local.
fn clock() -> (String, String) {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let hm = format!("{:02}:{:02}", tm.tm_hour, tm.tm_min);
    let date = format!("{:02}/{:02}/{:02}", tm.tm_mday, tm.tm_mon + 1, tm.tm_year % 100);
    (format!("{date} {hm}"), hm)
}

pub fn start(app: &AppHandle) {
    let points = app.state::<Points>();
    if let Ok(text) = std::fs::read_to_string(log_path(app)) {
        let lines: Vec<String> = text.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect();
        *points.log.lock().unwrap() = lines[lines.len().saturating_sub(LOG_KEEP)..].to_vec();
    }
    delay(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            for (site, every) in [(Site::Twitch, TWITCH_EVERY), (Site::Kick, KICK_EVERY)] {
                let due = {
                    let points = app.state::<Points>();
                    let mut next = points.next.lock().unwrap();
                    let due = next.get(&site).is_none_or(|t| Instant::now() >= *t);
                    if due {
                        next.insert(site, Instant::now() + every);
                    }
                    due
                };
                if due {
                    run(&app, site).await;
                }
            }
        }
    });
}

/// Uma janela de lives abriu: as conferências esperam 1 min, para os players carregarem.
pub fn delay(app: &AppHandle) {
    let at = Instant::now() + FIRST_RUN;
    let points = app.state::<Points>();
    let mut next = points.next.lock().unwrap();
    next.insert(Site::Twitch, at);
    next.insert(Site::Kick, at);
}

/// A config das lives foi salva: a conta que tinha parado volta a resgatar.
pub fn restart(app: &AppHandle) {
    app.state::<Points>().redeemers.lock().unwrap().clear();
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn run(app: &AppHandle, site: Site) {
    let (accounts, claim_chest) = app.state::<Deck>().read(|c| {
        let ids = c.lives_accounts();
        let accounts: Vec<_> = c.accounts.iter().filter(|a| ids.contains(&a.id) && !a.off).cloned().collect();
        (accounts, c.lives.claim_chest)
    });
    for account in accounts {
        let reward = match site {
            Site::Twitch => account.lives.redeem_twitch,
            Site::Kick => account.lives.redeem_kick,
        };
        let claim = site == Site::Twitch && claim_chest;
        if reward.is_none() && !claim {
            continue;
        }
        let pages = lives::live_pages(app, account.id, site);
        if pages.is_empty() {
            continue;
        }
        let cookies = match site {
            Site::Twitch => webkit::cookies(app, account.id, "twitch.tv").await,
            Site::Kick => webkit::cookies(app, account.id, "kick.com").await,
        }
        .unwrap_or_default();
        let find = |name: &str| cookies.iter().find(|(n, v)| n == name && !v.is_empty()).map(|(_, v)| v.clone());
        let (token, device) = match site {
            Site::Twitch => (find("auth-token"), find("unique_id")),
            Site::Kick => (find("session_token").map(|t| percent_decode(&t)), None),
        };
        let Some(token) = token else {
            app.state::<Points>().redeemers.lock().unwrap().entry((account.id, site)).or_default().no_login = true;
            continue;
        };
        for (label, channel) in pages {
            let args = arguments(app, account.id, site, &channel, reward, claim, &token, device.as_deref());
            let script = match site {
                Site::Twitch => TWITCH_JS,
                Site::Kick => KICK_JS,
            }
            .replace("__ARGS__", &args.to_string());
            let Some(webview) = app.get_webview(&label) else { continue };
            let result = webkit::call_async(&webview, &format!("return await (\n{script}\n);"), Duration::from_secs(30)).await;
            let value = result
                .ok()
                .and_then(|json| serde_json::from_str::<String>(&json).ok())
                .and_then(|inner| serde_json::from_str::<serde_json::Value>(&inner).ok());
            apply(app, &account.name, account.id, site, &channel, value);
        }
        if let Some(r) = app.state::<Points>().redeemers.lock().unwrap().get_mut(&(account.id, site)) {
            r.no_login = false;
        }
    }
    lives::emit_summary(app);
}

/// Roda o script da Kick numa live com o token dado e devolve o objeto que ele responde (autoteste).
pub async fn probe_kick(app: &AppHandle, label: &str, channel: &str, token: &str) -> Option<serde_json::Value> {
    let args = serde_json::json!({ "api": "https://kick.com", "channel": channel, "token": token, "word": word(Reward::Xp), "spend": false, "refund": null });
    let script = KICK_JS.replace("__ARGS__", &args.to_string());
    let webview = app.get_webview(label)?;
    let json = webkit::call_async(&webview, &format!("return await (\n{script}\n);"), Duration::from_secs(30)).await.ok()?;
    serde_json::from_str::<String>(&json).ok().and_then(|inner| serde_json::from_str(&inner).ok())
}

#[allow(clippy::too_many_arguments)]
fn arguments(
    app: &AppHandle,
    id: Uuid,
    site: Site,
    channel: &str,
    reward: Option<Reward>,
    claim: bool,
    token: &str,
    device: Option<&str>,
) -> serde_json::Value {
    let points = app.state::<Points>();
    let redeemers = points.redeemers.lock().unwrap();
    let redeemer = redeemers.get(&(id, site));
    let state = redeemer.and_then(|r| r.channels.get(channel)).cloned().unwrap_or_default();
    let now = Instant::now();
    let spend = redeemer.is_none_or(|r| r.stopped.is_none()) && state.retry_at.is_none_or(|t| now >= t);
    let refund = match (state.redeemed_at, state.points, state.cost) {
        (Some(at), Some(points), Some(cost)) if at.elapsed() < REFUND_WINDOW => serde_json::json!(points + cost / 2),
        _ => serde_json::Value::Null,
    };
    match site {
        Site::Kick => serde_json::json!({
            "api": "https://kick.com",
            "channel": channel,
            "token": token,
            "word": reward.map(word),
            "spend": spend,
            "refund": refund,
        }),
        Site::Twitch => serde_json::json!({
            "api": "https://gql.twitch.tv",
            "channel": channel,
            "token": token,
            "device": device,
            "clientId": TWITCH_CLIENT_ID,
            "word": reward.map(word),
            "spend": spend,
            "refund": refund,
            "claim": claim && state.claim_retry_at.is_none_or(|t| now >= t),
        }),
    }
}

fn apply(app: &AppHandle, account: &str, id: Uuid, site: Site, channel: &str, value: Option<serde_json::Value>) {
    let mut notes = Vec::new();
    let mut logs = Vec::new();
    {
        let points = app.state::<Points>();
        let mut redeemers = points.redeemers.lock().unwrap();
        let redeemer = redeemers.entry((id, site)).or_default();
        let stopped = redeemer.stopped.clone();
        let c = redeemer.channels.entry(channel.to_owned()).or_default();
        let Some(v) = value else {
            c.text = "a live não respondeu".to_owned();
            return;
        };
        let now = Instant::now();
        let (_, hm) = clock();

        if v.get("claimed") == Some(&serde_json::Value::Bool(true)) {
            c.claimed_clock = Some(hm.clone());
            c.claim_error = None;
            let balance = v["points"].as_i64().map(|p| format!(", saldo {p}")).unwrap_or_default();
            logs.push(format!("{channel}: pegou o baú (+50{balance})"));
        } else if let Some(error) = v.get("claimError").and_then(|e| e.as_str()) {
            c.claim_error = Some(error.to_owned());
            c.claim_retry_at = Some(now + RETRY_WAIT);
            logs.push(format!("{channel}: o baú não passou ({error}); tento de novo em 30 min"));
        }
        let chest = match (&c.claim_error, &c.claimed_clock) {
            (Some(error), _) => format!(", baú: {error}"),
            (None, Some(at)) => format!(", baú {at}"),
            _ => String::new(),
        };
        let error = v.get("error").and_then(|e| e.as_str());

        let Some(balance) = v.get("points").and_then(|p| p.as_i64()) else {
            c.text = error.unwrap_or("a live não respondeu").to_owned();
            return;
        };
        let Some(cost) = v.get("cost").and_then(|p| p.as_i64()) else {
            c.points = Some(balance);
            c.text = format!("{balance} pontos{chest}{}", error.map(|e| format!(", {e}")).unwrap_or_default());
            return;
        };
        let title = v.get("title").and_then(|t| t.as_str()).unwrap_or("o bônus").to_owned();

        // Pontos que voltaram depois de um resgate: o site não está vinculado no jogo, que devolve.
        let refund_at = match (c.redeemed_at, c.points) {
            (Some(at), Some(points)) if at.elapsed() < REFUND_WINDOW => Some(points + c.cost.unwrap_or(cost) / 2),
            _ => None,
        };
        let mut stop = None;
        if refund_at.is_some_and(|r| balance >= r) && stopped.is_none() {
            let place = match site {
                Site::Twitch => "Bônus Twitch",
                Site::Kick => "Bônus na Kick",
            };
            stop = Some(format!(
                "os pontos voltaram (vincule a {} no jogo, em {place}, e salve Lives → Config… de novo)",
                site.name()
            ));
            notes.push(format!(
                "{}: os pontos do resgate voltaram ({} não vinculada no jogo?); parei de resgatar nesta conta",
                site.name(),
                site.name()
            ));
            logs.push(format!(
                "{channel}: os pontos do último resgate voltaram (saldo {balance}; {} não vinculada no jogo?); parei de resgatar nesta conta",
                site.name()
            ));
        }

        let redeemed = v.get("redeemed").and_then(|r| r.as_bool());
        let refused = v
            .get("refused")
            .and_then(|r| r.as_str().map(String::from))
            .or_else(|| (redeemed == Some(false)).then(|| v["status"].to_string()));
        if redeemed == Some(true) {
            c.points = Some(balance - cost);
            c.redeemed_at = Some(now);
            c.redeemed_clock = Some(hm);
            c.refused = None;
            notes.push(format!("{}: resgatei {title} em {channel}", site.name()));
            logs.push(format!("{channel}: resgatou {title} ({cost} pontos; sobraram {})", balance - cost));
        } else if let Some(refused) = refused {
            c.points = Some(balance);
            c.retry_at = Some(now + RETRY_WAIT);
            notes.push(format!(
                "{}: o resgate em {channel} foi recusado (erro {refused}); tento de novo em 30 min",
                site.name()
            ));
            logs.push(format!(
                "{channel}: o resgate de {title} foi recusado (erro {refused}; saldo {balance}); tento de novo em 30 min"
            ));
            c.refused = Some(refused);
        } else {
            c.points = Some(balance);
        }
        c.cost = Some(cost);
        c.paused = v.get("paused").and_then(|p| p.as_bool()).unwrap_or(false);
        let mut text = format!("{}/{cost} pontos", c.points.unwrap_or(balance));
        if let Some(at) = &c.redeemed_clock {
            text.push_str(&format!(", resgatou {at}"));
        }
        if let Some(refused) = &c.refused {
            text.push_str(&format!(", resgate recusado ({refused})"));
        }
        if c.paused {
            text.push_str(", bônus pausado no canal");
        }
        text.push_str(&chest);
        c.text = text;
        if stop.is_some() {
            redeemer.stopped = stop;
        }
    }
    for note in notes {
        automation::note(app, id, note);
    }
    for line in logs {
        add_log(app, account, site, &line);
    }
}

fn add_log(app: &AppHandle, account: &str, site: Site, text: &str) {
    let (stamp, _) = clock();
    let line = format!("{stamp}  {account} · {} · {text}", site.name());
    let path = log_path(app);
    let points = app.state::<Points>();
    let mut log = points.log.lock().unwrap();
    log.push(line.clone());
    if log.len() > LOG_MAX {
        let cut = log.len() - LOG_KEEP;
        log.drain(..cut);
        let _ = std::fs::write(&path, log.join("\n") + "\n");
    } else if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{line}");
    }
    drop(log);
    let _ = app.emit_to("registro", "registro-line", line);
}

/// O que o menu Lives mostra ao lado de cada live: "230/300 pontos, resgatou 14:05".
pub fn channel_text(app: &AppHandle, id: Uuid, site: Site, channel: &str) -> Option<String> {
    let points = app.state::<Points>();
    let redeemers = points.redeemers.lock().unwrap();
    let text = redeemers.get(&(id, site))?.channels.get(channel)?.text.clone();
    (!text.is_empty()).then_some(text)
}

/// Por que esta conta não resgata agora (parou, ou falta o login), se for o caso.
pub fn account_text(app: &AppHandle, id: Uuid, site: Site) -> Option<String> {
    let points = app.state::<Points>();
    let redeemers = points.redeemers.lock().unwrap();
    let redeemer = redeemers.get(&(id, site))?;
    if let Some(reason) = &redeemer.stopped {
        return Some(format!("resgate da {} parado: {reason}", site.name()));
    }
    redeemer.no_login.then(|| format!("pontos da {}: entre na {} nas lives desta conta", site.name(), site.name()))
}

pub fn today_count(app: &AppHandle) -> usize {
    let (stamp, _) = clock();
    let today = &stamp[..8];
    app.state::<Points>().log.lock().unwrap().iter().filter(|l| l.starts_with(today)).count()
}

#[cfg(test)]
mod tests {
    #[test]
    fn kick_cookie_keeps_the_pipe() {
        assert_eq!(super::percent_decode("123%7Cabc%"), "123|abc%");
    }
}

// ---------- Registro dos resgates ----------

pub fn open_log_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("registro") {
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, "registro", WebviewUrl::App("registro.html".into()))
        .title("Registro dos resgates")
        .inner_size(820.0, 480.0)
        .min_inner_size(520.0, 300.0)
        .center()
        .build();
    if let Err(err) = result {
        eprintln!("[lives] não abriu o registro: {err}");
    }
}

#[derive(Serialize)]
pub struct LogView {
    lines: Vec<String>,
}

#[tauri::command]
pub fn registro_get(app: AppHandle) -> LogView {
    let mut lines = app.state::<Points>().log.lock().unwrap().clone();
    lines.reverse();
    LogView { lines }
}

#[tauri::command]
pub fn registro_open_file(app: AppHandle) {
    let path = log_path(&app);
    if !path.exists() {
        let _ = std::fs::write(&path, "");
    }
    let _ = std::process::Command::new("open").arg("-t").arg(path).spawn();
}
