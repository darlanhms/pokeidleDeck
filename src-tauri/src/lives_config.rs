//! "Lives → Config…": os canais em ordem de prioridade, quantas lives abrir por conta, o resgate
//! automático de cada conta e o baú da Twitch.

use crate::{
    config::{parse_channels, Reward, MAX_LIVE_SLOTS},
    deck::Deck,
    lives,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;

const LABEL: &str = "lives-config";

pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("lives-config.html".into()))
        .title("Config das lives")
        .inner_size(600.0, 760.0)
        .min_inner_size(520.0, 520.0)
        .minimizable(false)
        .center()
        .build();
    if let Err(err) = result {
        eprintln!("[lives] não abriu a config: {err}");
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRedeem {
    id: Uuid,
    #[serde(default)]
    name: String,
    redeem_twitch: Option<Reward>,
    redeem_kick: Option<Reward>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LivesForm {
    twitch: String,
    kick: String,
    twitch_slots: usize,
    kick_slots: usize,
    claim_chest: bool,
    accounts: Vec<AccountRedeem>,
}

#[tauri::command]
pub fn lives_config_get(app: AppHandle) -> LivesForm {
    app.state::<Deck>().read(|c| LivesForm {
        twitch: c.lives.twitch_channels.join("\n"),
        kick: c.lives.kick_channels.join("\n"),
        twitch_slots: c.lives.twitch_slots,
        kick_slots: c.lives.kick_slots,
        claim_chest: c.lives.claim_chest,
        accounts: c
            .accounts
            .iter()
            .map(|a| AccountRedeem {
                id: a.id,
                name: a.name.clone(),
                redeem_twitch: a.lives.redeem_twitch,
                redeem_kick: a.lives.redeem_kick,
            })
            .collect(),
    })
}

/// Salvar recomeça as lives abertas com os canais novos (e volta a resgatar numa conta que parou).
#[tauri::command]
pub fn lives_config_set(app: AppHandle, window: tauri::WebviewWindow, form: LivesForm) {
    app.state::<Deck>().update(|c| {
        c.lives.twitch_channels = parse_channels(&form.twitch);
        c.lives.kick_channels = parse_channels(&form.kick);
        c.lives.twitch_slots = form.twitch_slots.min(MAX_LIVE_SLOTS);
        c.lives.kick_slots = form.kick_slots.min(MAX_LIVE_SLOTS);
        c.lives.claim_chest = form.claim_chest;
        for account in &mut c.accounts {
            if let Some(choice) = form.accounts.iter().find(|r| r.id == account.id) {
                account.lives.redeem_twitch = choice.redeem_twitch;
                account.lives.redeem_kick = choice.redeem_kick;
            }
        }
    });
    lives::restart(&app);
    let _ = window.close();
}
