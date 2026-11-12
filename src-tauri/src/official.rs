//! Os streamers oficiais do PokeIdle, como o próprio jogo lista no painel de bônus da Twitch e da Kick.
//! A lista chega pelo socket do jogo (`estado.twitch.oficiais`, `estado.kick.oficiais`) de qualquer conta
//! conectada, e vale para todas: é a mesma para todo mundo.

use crate::{config::Site, deck::Deck};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

/// Sem notícia do jogo há mais que isso (todas as contas fechadas ou desligadas): volta a conferir na rede.
const FRESH: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Streamer {
    pub channel: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub live: bool,
    /// O streamer ligou o canal no jogo: só assim os pontos de lá viram bônus.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub viewers: u64,
}

#[derive(Default)]
pub struct Official {
    lists: Mutex<HashMap<Site, (Vec<Streamer>, Instant)>>,
}

fn site_key(site: Site) -> &'static str {
    match site {
        Site::Twitch => "twitch",
        Site::Kick => "kick",
    }
}

fn parse(value: &serde_json::Value) -> Option<Vec<Streamer>> {
    let list: Vec<Streamer> = serde_json::from_value(value.clone()).ok()?;
    Some(
        list.into_iter()
            .map(|mut s| {
                s.channel = s.channel.trim().to_lowercase();
                s
            })
            .filter(|s| !s.channel.is_empty())
            .collect(),
    )
}

/// O relatório de uma conta trouxe a lista do jogo. Muda as lives só quando muda quem está no ar.
pub fn on_report(app: &AppHandle, report: &serde_json::Value) {
    let Some(official) = report.get("official").filter(|v| v.is_object()) else { return };
    let mut changed = false;
    {
        let state = app.state::<Official>();
        let mut lists = state.lists.lock().unwrap();
        for site in [Site::Twitch, Site::Kick] {
            let Some(list) = official.get(site_key(site)).and_then(parse) else { continue };
            let key = |l: &[Streamer]| l.iter().map(|s| (s.channel.clone(), s.live, s.active)).collect::<Vec<_>>();
            if lists.get(&site).is_none_or(|(old, at)| key(old) != key(&list) || at.elapsed() >= FRESH) {
                changed = true;
            }
            lists.insert(site, (list, Instant::now()));
        }
    }
    if changed {
        crate::lives::official_changed(app);
    }
}

/// A lista do jogo, se chegou há pouco.
pub fn streamers(app: &AppHandle, site: Site) -> Option<Vec<Streamer>> {
    let state = app.state::<Official>();
    let lists = state.lists.lock().unwrap();
    lists.get(&site).filter(|(_, at)| at.elapsed() < FRESH).map(|(list, _)| list.clone())
}

/// Os canais das lives: os oficiais com o bônus ligado (na ordem do jogo) e depois os extras da config.
pub fn channels(app: &AppHandle, site: Site) -> Vec<String> {
    let extras = app.state::<Deck>().read(|c| c.lives.channels(site).to_vec());
    merge(streamers(app, site).as_deref().unwrap_or_default(), &extras)
}

pub fn merge(official: &[Streamer], extras: &[String]) -> Vec<String> {
    let mut out: Vec<String> = official.iter().filter(|s| s.active).map(|s| s.channel.clone()).collect();
    for c in extras {
        if !out.contains(c) {
            out.push(c.clone());
        }
    }
    out
}

/// Quantas vagas abrir no site: as da config, até o tanto de canais que há.
pub fn slots(app: &AppHandle, site: Site) -> usize {
    let max = app.state::<Deck>().read(|c| c.lives.max_slots(site));
    max.min(channels(app, site).len())
}

/// No ar segundo o jogo: ao vivo e com o bônus ligado (o painel do jogo conta assim). Canal fora da lista
/// oficial fica de fora, para conferir na rede.
pub fn live_status(app: &AppHandle, site: Site) -> Option<HashMap<String, bool>> {
    streamers(app, site).map(|list| list.into_iter().map(|s| (s.channel, s.live && s.active)).collect())
}

pub fn viewers(app: &AppHandle, site: Site, channel: &str) -> Option<u64> {
    streamers(app, site)?.into_iter().find(|s| s.channel == channel).filter(|s| s.live).map(|s| s.viewers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(channel: &str, live: bool, active: bool) -> Streamer {
        Streamer { channel: channel.into(), name: None, live, active, viewers: 0 }
    }

    #[test]
    fn official_active_first_then_extras_without_repeats() {
        let official = [s("a", true, true), s("b", false, false), s("c", false, true)];
        let extras = vec!["c".to_string(), "x".to_string()];
        assert_eq!(merge(&official, &extras), vec!["a", "c", "x"]);
    }

    #[test]
    fn parse_lowercases_and_drops_empty() {
        let v = serde_json::json!([{ "channel": " Yetz ", "live": true, "active": true, "viewers": 3 }, { "channel": "" }]);
        let list = parse(&v).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].channel, "yetz");
    }
}
