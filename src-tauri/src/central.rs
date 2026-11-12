//! A Central das lives: cada canal da lista com o que toca nele, o saldo da conta e o resgate em um
//! clique; as vagas, que tocam o canal escolhido ou a primeira da lista no ar.

use crate::{
    accounts, automation,
    config::{Reward, Site, SlotChoice},
    deck::Deck,
    live_check::Why,
    lives, lives_config, points,
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

const LABEL: &str = "central";

#[derive(Default)]
pub struct Central {
    selected: Mutex<Option<Uuid>>,
}

pub fn open(app: &AppHandle, account: Option<Uuid>) {
    if account.is_some() {
        *app.state::<Central>().selected.lock().unwrap() = account;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_focus();
        emit(app);
        return;
    }
    let result = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("central.html".into()))
        .title("Lives")
        .inner_size(1180.0, 820.0)
        .min_inner_size(820.0, 520.0)
        .center()
        .build();
    if let Err(err) = result {
        eprintln!("[central] não abriu: {err}");
    }
    refresh(app);
}

/// Lê de novo o saldo dos canais da conta escolhida (a central acabou de abrir ou trocou de conta).
fn refresh(app: &AppHandle) {
    let Some(id) = selected(app) else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for site in [Site::Twitch, Site::Kick] {
            let channels = app.state::<Deck>().read(|c| c.lives.channels(site).to_vec());
            points::scan_account(&app, id, site, &channels).await;
        }
        emit(&app);
    });
}

fn selected(app: &AppHandle) -> Option<Uuid> {
    let accounts = app.state::<Deck>().accounts();
    let chosen = *app.state::<Central>().selected.lock().unwrap();
    let lives_on = app.state::<Deck>().read(|c| c.lives_accounts());
    chosen
        .filter(|id| accounts.iter().any(|a| a.id == *id))
        .or_else(|| lives_on.first().copied())
        .or_else(|| accounts.first().map(|a| a.id))
}

pub fn emit(app: &AppHandle) {
    if app.get_webview_window(LABEL).is_some() {
        let _ = app.emit_to(LABEL, "central", view(app));
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    id: Uuid,
    name: String,
    lives_on: bool,
    off: bool,
    playing: usize,
    redeemable: usize,
    warn: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardView {
    kind: Reward,
    title: String,
    cost: i64,
    blocked: Option<String>,
    affordable: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    channel: String,
    live: Option<bool>,
    /// playing, standin, queued, offline, closed, unknown
    state: &'static str,
    slot: Option<usize>,
    stand_in_for: Option<String>,
    playing: Option<bool>,
    points: Option<i64>,
    owner: bool,
    ready: bool,
    rewards: Vec<RewardView>,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotPicker {
    index: usize,
    choice: SlotChoice,
    current: Option<String>,
    /// O escolhido saiu do ar e outro toca no lugar.
    warn: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteView {
    site: Site,
    name: &'static str,
    login: Option<bool>,
    auto_redeem: Option<Reward>,
    slots: Vec<SlotPicker>,
    notes: Vec<String>,
    rows: Vec<Row>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    text: String,
    meta: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    id: Uuid,
    name: String,
    lives_on: bool,
    off: bool,
    playing: usize,
    slots: usize,
    redeemable: usize,
    chest: Option<String>,
    claim_chest: bool,
    sites: Vec<SiteView>,
    feed: Vec<FeedItem>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    tabs: Vec<Tab>,
    account: Option<AccountView>,
}

fn site_view(app: &AppHandle, id: Uuid, site: Site) -> SiteView {
    let (channels, limit, auto_redeem) = app.state::<Deck>().read(|c| {
        let account = c.accounts.iter().find(|a| a.id == id);
        (
            c.lives.channels(site).to_vec(),
            account.map(|a| a.lives.limit(site).clone()).unwrap_or_default(),
            account.and_then(|a| match site {
                Site::Twitch => a.lives.redeem_twitch,
                Site::Kick => a.lives.redeem_kick,
            }),
        )
    });
    let slots = lives::slot_views(app, id, site);
    let mut notes = Vec::new();
    let pickers: Vec<SlotPicker> = slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let choice = limit.choice(index);
            let warn = matches!(slot.why, Some(Why::StandIn(_))) || matches!(&choice, SlotChoice::Channel(c) if slot.channel.as_ref() != Some(c));
            if let (SlotChoice::Channel(chosen), true) = (&choice, warn) {
                notes.push(match &slot.channel {
                    Some(now) => format!("{chosen} (escolhido na vaga {}) está fora do ar: {now} toca no lugar até ele voltar.", index + 1),
                    None => format!("{chosen} (escolhido na vaga {}) está fora do ar e nenhuma outra da lista está no ar.", index + 1),
                });
            }
            SlotPicker { index, choice, current: slot.channel.clone(), warn }
        })
        .collect();
    let rows = channels
        .iter()
        .map(|channel| {
            let live = lives::live_status(app, site, channel);
            let slot = slots.iter().position(|s| s.channel.as_ref() == Some(channel));
            let stand_in_for = slot.and_then(|i| match &slots[i].why {
                Some(Why::StandIn(of)) => Some(of.clone()),
                _ => None,
            });
            let state = if slot.is_some() {
                if stand_in_for.is_some() { "standin" } else { "playing" }
            } else if limit.closed.contains(channel) {
                "closed"
            } else {
                match live {
                    Some(true) => "queued",
                    Some(false) => "offline",
                    None => "unknown",
                }
            };
            let info = points::channel_info(app, id, site, channel);
            let (points, owner, error) = info.as_ref().map(|i| (i.points, i.owner, i.error.clone())).unwrap_or((None, false, None));
            let rewards: Vec<RewardView> = info
                .map(|i| {
                    i.rewards
                        .iter()
                        .map(|r| RewardView {
                            kind: r.kind,
                            title: r.title.clone(),
                            cost: r.cost,
                            blocked: r.blocked.clone(),
                            affordable: r.blocked.is_none() && (owner || points.is_some_and(|p| p >= r.cost)),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Row {
                channel: channel.clone(),
                live,
                state,
                slot,
                stand_in_for,
                playing: slot.and_then(|i| slots[i].playing),
                points,
                owner,
                ready: rewards.iter().any(|r| r.affordable),
                rewards,
                error,
            }
        })
        .collect();
    SiteView {
        site,
        name: site.name(),
        login: lives::login(app, id, site),
        auto_redeem,
        slots: pickers,
        notes,
        rows,
    }
}

/// "dd/MM/yy HH:mm  Conta · Site · texto" → texto e "Site · HH:mm".
fn feed_item(line: &str) -> FeedItem {
    let (stamp, rest) = line.split_once("  ").unwrap_or(("", line));
    let mut parts = rest.splitn(3, " · ");
    let (_, site, text) = (parts.next(), parts.next().unwrap_or(""), parts.next().unwrap_or(rest));
    let hour = stamp.rsplit(' ').next().unwrap_or("");
    FeedItem { text: text.to_owned(), meta: format!("{site} · {hour}") }
}

/// (tocando, vagas, canais prontos para resgatar, falta login) de uma conta.
pub fn account_counts(app: &AppHandle, id: Uuid) -> (usize, usize, usize, bool) {
    counts(app, id)
}

fn counts(app: &AppHandle, id: Uuid) -> (usize, usize, usize, bool) {
    let mut playing = 0;
    let mut slots = 0;
    let mut redeemable = 0;
    let mut warn = false;
    for site in [Site::Twitch, Site::Kick] {
        let view = site_view(app, id, site);
        slots += view.slots.len();
        playing += view.slots.iter().filter(|s| s.current.is_some()).count();
        redeemable += view.rows.iter().filter(|r| r.ready).count();
        warn |= !view.slots.is_empty() && view.login == Some(false);
    }
    (playing, slots, redeemable, warn)
}

pub fn view(app: &AppHandle) -> View {
    let accounts = app.state::<Deck>().accounts();
    let lives_on = app.state::<Deck>().read(|c| c.lives_accounts());
    let claim_chest = app.state::<Deck>().read(|c| c.lives.claim_chest);
    let tabs = accounts
        .iter()
        .map(|a| {
            let (playing, _, redeemable, warn) = counts(app, a.id);
            Tab { id: a.id, name: a.name.clone(), lives_on: lives_on.contains(&a.id), off: a.off, playing, redeemable, warn }
        })
        .collect();
    let account = selected(app).and_then(|id| accounts.iter().find(|a| a.id == id)).map(|a| {
        let (playing, slots, redeemable, _) = counts(app, a.id);
        AccountView {
            id: a.id,
            name: a.name.clone(),
            lives_on: lives_on.contains(&a.id),
            off: a.off,
            playing,
            slots,
            redeemable,
            chest: points::last_chest(app, a.id),
            claim_chest,
            sites: [Site::Twitch, Site::Kick].into_iter().map(|s| site_view(app, a.id, s)).collect(),
            feed: points::recent_log(app, &a.name, 12).iter().map(|l| feed_item(l)).collect(),
        }
    });
    View { tabs, account }
}

fn site_of(name: &str) -> Option<Site> {
    match name {
        "Twitch" | "twitch" => Some(Site::Twitch),
        "Kick" | "kick" => Some(Site::Kick),
        _ => None,
    }
}

#[tauri::command]
pub fn central_state(app: AppHandle) -> View {
    view(&app)
}

#[tauri::command]
pub fn central_select(app: AppHandle, id: Uuid) {
    *app.state::<Central>().selected.lock().unwrap() = Some(id);
    emit(&app);
    refresh(&app);
}

#[tauri::command]
pub fn central_choose(app: AppHandle, id: Uuid, site: String, slot: usize, choice: SlotChoice) {
    if let Some(site) = site_of(&site) {
        lives::choose_slot(&app, id, site, slot, choice);
        emit(&app);
    }
}

#[tauri::command]
pub async fn central_redeem(app: AppHandle, id: Uuid, site: String, channel: String, kind: Reward) -> Result<String, String> {
    let site = site_of(&site).ok_or("site desconhecido")?;
    let name = app.state::<Deck>().accounts().into_iter().find(|a| a.id == id).map(|a| a.name).unwrap_or_default();
    let result = points::redeem(&app, &name, id, site, &channel, kind).await;
    if let Ok(text) = &result {
        automation::note(&app, id, text.clone());
    }
    result
}

#[tauri::command]
pub fn central_channel(app: AppHandle, id: Uuid, site: String, channel: String, open: bool) {
    let Some(site) = site_of(&site) else { return };
    if open {
        lives::reopen_live(&app, id, site, Some(channel));
    } else {
        lives::close_live(&app, id, site, Some(channel));
    }
    emit(&app);
}

#[tauri::command]
pub fn central_action(app: AppHandle, action: String, id: Option<Uuid>, site: Option<String>) {
    let site = site.as_deref().and_then(site_of);
    match (action.as_str(), id) {
        ("toggle-lives", Some(id)) => {
            lives::toggle_account(&app, id);
            refresh(&app);
        }
        ("players", Some(id)) => lives::reveal(&app, id, 0),
        ("login", Some(id)) => {
            if let Some(site) = site {
                lives::reveal_login(&app, id, site);
            }
        }
        ("turn-on", Some(id)) => accounts::set_off(&app, id, false),
        ("config", _) => lives_config::open(&app),
        ("log", _) => points::open_log_window(&app),
        ("refresh", _) => refresh(&app),
        _ => {}
    }
    emit(&app);
}
