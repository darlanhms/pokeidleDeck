//! Quem está no ar: a Kick pela mesma API do site (um pedido por canal, de dentro do WebKit) e a Twitch numa consulta
//! GraphQL só (o client id público do site). `Some(true)` no ar, `Some(false)` offline, `None` não deu
//! para saber agora — e quem não deu para saber conta como no ar, para não perder pontos.

use crate::{accounts::USER_AGENT, webkit};
use std::{collections::HashMap, time::Duration};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl};

pub const TWITCH_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const TIMEOUT: Duration = Duration::from_secs(15);

fn agent() -> ureq::Agent {
    let builder = ureq::AgentBuilder::new().timeout(TIMEOUT).user_agent(USER_AGENT);
    match native_tls::TlsConnector::new() {
        Ok(tls) => builder.tls_connector(std::sync::Arc::new(tls)).build(),
        Err(_) => builder.build(),
    }
}

const KICK_CHECKER: &str = "kick-check";

/// A Kick barra (403) pedidos que não parecem de navegador, então a conferência roda dentro de uma
/// webview escondida aberta na própria API da Kick, com `fetch` da mesma origem.
pub async fn kick_live(app: &AppHandle, channels: &[String]) -> HashMap<String, Option<bool>> {
    let unknown = || channels.iter().map(|c| (c.clone(), None)).collect();
    if channels.is_empty() {
        return HashMap::new();
    }
    let Some(webview) = kick_checker(app, &channels[0]).await else { return unknown() };
    let script = format!(
        "const out = {{}};\n\
         await Promise.all({list}.map(async (c) => {{\n\
           try {{\n\
             const r = await fetch('/api/v2/channels/' + encodeURIComponent(c), {{ headers: {{ Accept: 'application/json' }}, cache: 'no-store' }});\n\
             out[c] = r.status === 404 ? 'false' : r.ok ? await r.text() : null;\n\
           }} catch (e) {{ out[c] = null; }}\n\
         }}));\n\
         return out;",
        list = serde_json::to_string(channels).unwrap_or_default()
    );
    // Logo depois de abrir, a página da Kick ainda pode recusar os pedidos: tudo volta vazio.
    let mut answers = HashMap::<String, Option<String>>::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let Ok(json) = webkit::call_async(&webview, &script, TIMEOUT).await else { continue };
        answers = serde_json::from_str(&json).unwrap_or_default();
        if answers.values().any(Option::is_some) {
            break;
        }
    }
    channels
        .iter()
        .map(|c| {
            let live = match answers.get(c).cloned().flatten() {
                Some(body) if body == "false" => Some(false),
                Some(body) => parse_kick(&body),
                None => None,
            };
            (c.clone(), live)
        })
        .collect()
}

async fn kick_checker(app: &AppHandle, first: &str) -> Option<Webview> {
    if app.get_webview(KICK_CHECKER).is_none() {
        let url: tauri::Url = format!("https://kick.com/api/v2/channels/{}", urlencode(first)).parse().ok()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = app.clone();
        app.run_on_main_thread(move || {
            let created = crate::deck::main_window(&handle).and_then(|window| {
                let webview = window
                    .add_child(
                        webkit::prepare(&handle, WebviewBuilder::new(KICK_CHECKER, WebviewUrl::External(url)), None, Default::default())
                            .user_agent(USER_AGENT),
                        LogicalPosition::new(0.0, 0.0),
                        LogicalSize::new(10.0, 10.0),
                    )
                    .ok()?;
                let _ = webview.hide();
                Some(webview)
            });
            let _ = tx.send(created.is_some());
        })
        .ok()?;
        if !rx.await.unwrap_or(false) {
            return None;
        }
    }
    for _ in 0..20 {
        let webview = app.get_webview(KICK_CHECKER)?;
        if webview.url().is_ok_and(|u| u.host_str() == Some("kick.com")) {
            let ready = webkit::call_async(&webview, "return document.readyState", Duration::from_secs(3)).await;
            if ready.as_deref() == Ok("\"complete\"") {
                return Some(webview);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    app.get_webview(KICK_CHECKER)
}

/// `livestream` é null com o canal offline.
pub fn parse_kick(json: &str) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let live = value.get("livestream")?;
    Some(live.is_object() && live.get("is_live") != Some(&serde_json::Value::Bool(false)))
}

pub fn twitch_live(channels: &[String]) -> HashMap<String, Option<bool>> {
    let unknown = || channels.iter().map(|c| (c.clone(), None)).collect();
    if channels.is_empty() {
        return HashMap::new();
    }
    let body = serde_json::json!({
        "query": "query($l:[String!]){users(logins:$l){login stream{id}}}",
        "variables": { "l": channels },
    });
    let response = agent()
        .post("https://gql.twitch.tv/gql")
        .set("Client-Id", TWITCH_CLIENT_ID)
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&body.to_string());
    match response.ok().and_then(|r| r.into_string().ok()) {
        Some(json) => parse_twitch(&json, channels).unwrap_or_else(unknown),
        None => unknown(),
    }
}

/// Canal que não existe vem `null` na lista e conta como offline.
pub fn parse_twitch(json: &str, channels: &[String]) -> Option<HashMap<String, Option<bool>>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let users = value.get("data")?.get("users")?.as_array()?;
    let live: HashMap<String, bool> = users
        .iter()
        .filter_map(|u| Some((u.get("login")?.as_str()?.to_lowercase(), u.get("stream")?.is_object())))
        .collect();
    Some(channels.iter().map(|c| (c.clone(), Some(live.get(c).copied().unwrap_or(false)))).collect())
}

/// Por que um canal está numa vaga: escolhido, automático ou no lugar do escolhido que saiu do ar.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "of")]
pub enum Why {
    Chosen,
    Auto,
    StandIn(String),
}

/// O canal de cada vaga. Primeiro os escolhidos que estão no ar; depois cada vaga automática (ou de
/// escolhido fora do ar) pega a primeira da lista no ar que ainda não está em outra vaga.
pub fn assign(
    channels: &[String],
    live: &HashMap<String, Option<bool>>,
    choices: &[crate::config::SlotChoice],
    closed: &[String],
) -> Vec<Option<(String, Why)>> {
    use crate::config::SlotChoice;
    let usable = |c: &String| !closed.contains(c) && live.get(c).copied().flatten() != Some(false);
    let mut out: Vec<Option<(String, Why)>> = vec![None; choices.len()];
    let mut used: Vec<String> = Vec::new();
    for (slot, choice) in choices.iter().enumerate() {
        if let SlotChoice::Channel(c) = choice {
            if usable(c) && !used.contains(c) {
                used.push(c.clone());
                out[slot] = Some((c.clone(), Why::Chosen));
            }
        }
    }
    for (slot, choice) in choices.iter().enumerate() {
        if out[slot].is_some() || *choice == SlotChoice::Off {
            continue;
        }
        if let Some(c) = channels.iter().find(|c| usable(c) && !used.contains(c)) {
            used.push(c.clone());
            let why = match choice {
                SlotChoice::Channel(chosen) => Why::StandIn(chosen.clone()),
                _ => Why::Auto,
            };
            out[slot] = Some((c.clone(), why));
        }
    }
    out
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kick_offline_is_null_livestream() {
        assert_eq!(parse_kick(r#"{"livestream":null}"#), Some(false));
        assert_eq!(parse_kick(r#"{"livestream":{"is_live":true}}"#), Some(true));
        assert_eq!(parse_kick(r#"{"slug":"x"}"#), None);
    }

    #[test]
    fn twitch_missing_user_is_offline() {
        let channels = vec!["a".to_owned(), "b".to_owned()];
        let parsed = parse_twitch(r#"{"data":{"users":[{"login":"A","stream":{"id":"1"}},null]}}"#, &channels).unwrap();
        assert_eq!(parsed["a"], Some(true));
        assert_eq!(parsed["b"], Some(false));
    }

    #[test]
    fn chosen_slot_falls_back_and_returns() {
        use crate::config::SlotChoice::*;
        let channels: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let choices = vec![Channel("c".into()), Auto];
        let mut live: HashMap<String, Option<bool>> = channels.iter().map(|c| (c.clone(), Some(true))).collect();
        let names = |v: Vec<Option<(String, Why)>>| v.into_iter().map(|s| s.map(|(c, w)| (c, w))).collect::<Vec<_>>();
        assert_eq!(
            names(assign(&channels, &live, &choices, &[])),
            vec![Some(("c".into(), Why::Chosen)), Some(("a".into(), Why::Auto))]
        );
        live.insert("c".into(), Some(false));
        assert_eq!(
            names(assign(&channels, &live, &choices, &[])),
            vec![Some(("a".into(), Why::StandIn("c".into()))), Some(("b".into(), Why::Auto))]
        );
        assert_eq!(
            names(assign(&channels, &live, &[Off, Auto], &["a".into()])),
            vec![None, Some(("b".into(), Why::Auto))]
        );
    }

    #[test]
    fn unknown_counts_as_live_and_order_wins() {
        use crate::config::SlotChoice::Auto;
        let channels: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let live = HashMap::from([("a".to_owned(), Some(false)), ("b".to_owned(), None)]);
        let got: Vec<String> = assign(&channels, &live, &[Auto, Auto], &[]).into_iter().flatten().map(|(c, _)| c).collect();
        assert_eq!(got, vec!["b", "c"]);
    }
}

