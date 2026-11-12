//! Autotestes. `DECK_SELFTEST=1`, depois `2` com o mesmo `DECK_DATA_DIR`: isolamento das contas.
//! `shell`: layouts, ampliar, zoom, desligar, renomear, reordenar, remover e restaurar.
//! `auto`: menos animações, eco, ULTRA e recompra, na tela de login (sem precisar de conta). Marca cada conta com um cookie e um item de localStorage no pokeidle.io e confere
//! quem enxerga o quê, antes e depois de reabrir o app.

use crate::{config::DeckConfig, deck, webkit};
use std::{fmt::Write as _, time::Duration};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const MARK_JS: &str = "localStorage.setItem('deckMarker', ID);\n\
    document.cookie = 'deck_marker=' + ID + '; max-age=31536000; path=/; secure; samesite=lax';\n\
    return true;";

const READ_JS: &str = "return {\n\
    ls: localStorage.getItem('deckMarker'),\n\
    cookie: (document.cookie.match(/(?:^|; )deck_marker=([^;]+)/) || [])[1] || null,\n\
  };";

struct Report {
    text: String,
    failed: bool,
}

impl Report {
    fn check(&mut self, ok: bool, what: impl AsRef<str>) {
        let line = format!("{} {}", if ok { "OK  " } else { "FALHOU" }, what.as_ref());
        println!("[selftest] {line}");
        let _ = writeln!(self.text, "{line}");
        self.failed |= !ok;
    }
}

pub fn start(app: AppHandle, phase: String) {
    tauri::async_runtime::spawn(async move {
        let mut report = Report { text: String::new(), failed: false };
        match phase.as_str() {
            "1" => phase_one(&app, &mut report).await,
            "2" => phase_two(&app, &mut report).await,
            "menu" => return menu_over_webviews(&app).await,
            "shell" => shell(&app, &mut report).await,
            "auto" => automations(&app, &mut report).await,
            other => report.check(false, format!("fase desconhecida: {other}")),
        }
        let path = deck::data_dir(&app).join(format!("selftest-{phase}.txt"));
        let _ = std::fs::write(&path, &report.text);
        println!("[selftest] resultado em {}", path.display());
        app.exit(if report.failed { 1 } else { 0 });
    });
}

async fn on_main<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce(&AppHandle) -> T + Send + 'static,
) -> T {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&handle));
    })
    .expect("main thread");
    rx.await.expect("main thread respondeu")
}

async fn eval(app: &AppHandle, id: Uuid, script: &str) -> Result<serde_json::Value, String> {
    let webview = app
        .get_webview(&crate::accounts::label(&id))
        .ok_or_else(|| "webview não existe".to_owned())?;
    let json = webkit::call_async(&webview, script, Duration::from_secs(20)).await?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

async fn wait_loaded(app: &AppHandle, id: Uuid) -> bool {
    for _ in 0..60 {
        let state = eval(app, id, "return location.host + '|' + document.readyState").await;
        if state.as_ref().ok().and_then(|v| v.as_str()) == Some("pokeidle.io|complete") {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    false
}

async fn mark(app: &AppHandle, report: &mut Report, id: Uuid) {
    let loaded = wait_loaded(app, id).await;
    report.check(loaded, format!("{id} carregou o pokeidle.io"));
    let script = MARK_JS.replace("ID", &format!("'{id}'"));
    let marked = eval(app, id, &script).await;
    report.check(marked.is_ok(), format!("{id} marcada ({marked:?})"));
}

async fn sees_only_itself(app: &AppHandle, report: &mut Report, id: Uuid, when: &str) {
    let seen = eval(app, id, READ_JS).await;
    let own = id.to_string();
    let ok = matches!(&seen, Ok(v) if v["ls"] == own.as_str() && v["cookie"] == own.as_str());
    report.check(ok, format!("{when}: {id} vê só a própria marca ({seen:?})"));
}

/// Cache de disco pode ser regravado por uma carga que estava em andamento; o que importa é não sobrar login.
fn is_cache_only(record: &str) -> bool {
    record.contains("DiskCache") && !record.contains("Cookies") && !record.contains("Storage")
}

async fn store_exists(app: &AppHandle, id: Uuid) -> bool {
    let ids = app.fetch_data_store_identifiers().await.unwrap_or_default();
    ids.contains(id.as_bytes())
}

fn panes_fill_without_holes(app: &AppHandle, report: &mut Report, expected: usize) {
    let layout = deck::view(app).map(|l| serde_json::to_value(l).unwrap());
    let panes = layout
        .as_ref()
        .and_then(|l| l["panes"].as_array().cloned())
        .unwrap_or_default();
    let full_width = panes.last().map(|p| p["rect"]["w"].as_f64()).flatten();
    let window_width = app
        .get_window("main")
        .and_then(|w| Some(w.inner_size().ok()?.to_logical::<f64>(w.scale_factor().ok()?).width));
    let ok = panes.len() == expected
        && (expected != 3 || full_width.zip(window_width).is_some_and(|(a, b)| (a - b).abs() < 1.0));
    report.check(ok, format!("layout com {expected} contas sem buraco ({} painéis)", panes.len()));
}

async fn phase_one(app: &AppHandle, report: &mut Report) {
    let first = app.state::<deck::Deck>().accounts()[0].id;
    let second = on_main(app, |app| crate::accounts::add(app)).await;
    let Some(second) = second else {
        return report.check(false, "não criou a 2ª conta");
    };
    panes_fill_without_holes(app, report, 2);
    mark(app, report, first).await;
    mark(app, report, second).await;

    let third = on_main(app, |app| crate::accounts::add(app)).await;
    let Some(third) = third else {
        return report.check(false, "não criou a 3ª conta");
    };
    panes_fill_without_holes(app, report, 3);
    mark(app, report, third).await;
    for id in [first, second, third] {
        sees_only_itself(app, report, id, "3 contas").await;
    }
    let promise = eval(app, first, "return await new Promise((r) => setTimeout(() => r({ answer: 42 }), 500));").await;
    report.check(
        matches!(&promise, Ok(v) if v["answer"] == 42),
        format!("JS com await devolve o valor resolvido ({promise:?})"),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    for id in [first, second, third] {
        let last = app.state::<deck::Deck>().last_report(id);
        let ok = last.as_ref().is_some_and(|r| r["url"].as_str().is_some_and(|u| u.starts_with("https://pokeidle.io")));
        report.check(ok, format!("{id} reporta pelo IPC a cada segundo ({last:?})"));
    }

    on_main(app, move |app| crate::accounts::remove(app, second, true)).await;
    panes_fill_without_holes(app, report, 2);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let records = webkit::data_records(app, second).await;
    report.check(records.as_ref().is_ok_and(|r| r.iter().all(|r| is_cache_only(r))), format!("conta removida {second} ficou sem login/cookies ({records:?})"));
    let _ = std::fs::write(deck::data_dir(app).join("selftest-removed.txt"), second.to_string());
    report.check(store_exists(app, first).await && store_exists(app, third).await, "data stores das outras continuam");
    for id in [first, third] {
        on_main(app, move |app| {
            if let Some(webview) = app.get_webview(&crate::accounts::label(&id)) {
                let _ = webview.reload();
            }
        })
        .await;
        let _ = wait_loaded(app, id).await;
        sees_only_itself(app, report, id, "depois de remover a 2ª").await;
    }
    // Dá tempo ao processo de rede do WebKit de gravar os cookies antes de sair.
    tokio::time::sleep(Duration::from_secs(3)).await;
}

async fn phase_two(app: &AppHandle, report: &mut Report) {
    let config = DeckConfig::load(&deck::data_dir(app).join("deck.json"));
    let ids: Vec<Uuid> = config.accounts.iter().map(|a| a.id).collect();
    report.check(ids.len() == 2, format!("deck.json voltou com 2 contas ({})", ids.len()));
    panes_fill_without_holes(app, report, ids.len());
    for &id in &ids {
        let _ = wait_loaded(app, id).await;
        sees_only_itself(app, report, id, "depois de reabrir").await;
    }
    let removed = std::fs::read_to_string(deck::data_dir(app).join("selftest-removed.txt"))
        .ok()
        .and_then(|s| Uuid::parse_str(s.trim()).ok());
    tokio::time::sleep(Duration::from_secs(3)).await;
    if let Some(removed) = removed {
        report.check(!store_exists(app, removed).await, format!("data store da removida {removed} apagado do disco no início"));
    }
    for &id in &ids {
        on_main(app, move |app| crate::accounts::remove(app, id, true)).await;
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let records = webkit::data_records(app, ids[0]).await;
    report.check(
        records.as_ref().is_ok_and(|r| r.iter().all(|r| is_cache_only(r))),
        format!("limpeza: {} sem login/cookies ({records:?})", ids[0]),
    );
    let left = app.state::<deck::Deck>().accounts();
    report.check(left.len() == 1 && left[0].id == ids[1], "a última conta fica no deck");
}

/// Fotografa só a janela do deck (`screencapture -l`, nunca a tela inteira) para conferir os
/// cabeçalhos. O menu nativo é outra janela, acima de todas as do app, e não sai nessa foto.
async fn menu_over_webviews(app: &AppHandle) {
    let first = app.state::<deck::Deck>().accounts()[0].id;
    act(app, "deck:add".into()).await;
    act(app, "deck:add".into()).await;
    let third = view(app).panes[2].id;
    act(app, format!("acc:power:{third}")).await;
    on_main(app, move |app| crate::accounts::rename(app, first, "Main")).await;
    let _ = wait_loaded(app, first).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let shot = deck::data_dir(app).join("deck.png");
    let window_number = on_main(app, |app| {
        let window = app.get_window("main")?;
        for webview in window.webviews() {
            println!("[selftest] webview {} pos={:?} size={:?}", webview.label(), webview.position(), webview.size());
        }
        let pointer = window.ns_window().ok()?;
        Some(unsafe { (*(pointer as *const objc2_app_kit::NSWindow)).windowNumber() })
    })
    .await;
    if let Some(number) = window_number {
        let status = std::process::Command::new("screencapture")
            .args(["-x", "-o", "-l", &number.to_string()])
            .arg(&shot)
            .status();
        println!("[selftest] janela {number} em {} ({status:?})", shot.display());
    }
    act(app, "deck:layout:focus".into()).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    if let Some(number) = window_number {
        let shot = deck::data_dir(app).join("deck-focus.png");
        let _ = std::process::Command::new("screencapture").args(["-x", "-o", "-l", &number.to_string()]).arg(&shot).status();
    }
    act(app, "deck:layout:grid".into()).await;
    act(app, format!("acc:restock:{first}")).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let restock = app
        .get_webview_window(&format!("restock-{}", first.simple()))
        .and_then(|w| w.ns_window().ok())
        .map(|p| unsafe { (*(p as *const objc2_app_kit::NSWindow)).windowNumber() });
    if let Some(number) = restock {
        let shot = deck::data_dir(app).join("restock.png");
        let _ = std::process::Command::new("screencapture").args(["-x", "-o", "-l", &number.to_string()]).arg(&shot).status();
    }
    app.exit(0);
}

/// O que a WKWebView da conta está fazendo de verdade: escondida, zoom aplicado e endereço.
async fn native(app: &AppHandle, id: Uuid) -> Option<(bool, f64, String)> {
    let webview = app.get_webview(&crate::accounts::label(&id))?;
    let url = webview.url().map(|u| u.to_string()).unwrap_or_default();
    let (tx, rx) = tokio::sync::oneshot::channel();
    webview
        .with_webview(move |platform| unsafe {
            let view = &*(platform.inner() as *const objc2_web_kit::WKWebView);
            let _ = tx.send((view.isHidden(), view.pageZoom()));
        })
        .ok()?;
    let (hidden, zoom) = rx.await.ok()?;
    Some((hidden, zoom as f64, url))
}

async fn act(app: &AppHandle, action: String) {
    on_main(app, move |app| crate::menus::dispatch(app, &action)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
}

fn view(app: &AppHandle) -> deck::DeckView {
    deck::view(app).expect("deck sem janela")
}

async fn shell(app: &AppHandle, report: &mut Report) {
    let v = view(app);
    report.check(
        v.panes.len() == 1 && v.panes[0].rect.is_some_and(|r| (r.w - v.width).abs() < 1.0),
        "começa com 1 conta ocupando a largura toda",
    );
    let first = v.panes[0].id;
    for _ in 0..3 {
        act(app, "deck:add".into()).await;
    }
    let v = view(app);
    report.check(v.panes.len() == 4 && !v.can_add, format!("4 contas e \"+ Nova conta\" desabilitado ({})", v.panes.len()));
    act(app, "deck:add".into()).await;
    report.check(view(app).panes.len() == 4, "não passa de 4");
    let ids: Vec<Uuid> = view(app).panes.iter().map(|p| p.id).collect();

    act(app, "deck:layout:columns".into()).await;
    let widths: Vec<f64> = view(app).panes.iter().filter_map(|p| p.rect.map(|r| r.w)).collect();
    report.check(
        widths.len() == 4 && widths.iter().all(|w| (w - widths[0]).abs() < 0.5),
        format!("colunas: 4 larguras iguais ({widths:?})"),
    );

    act(app, "deck:zoom:set:0.5".into()).await;
    let zoom = native(app, ids[1]).await.map(|n| n.1);
    report.check(zoom.is_some_and(|z| (z - 0.5).abs() < 0.01), format!("zoom 50% aplicado nas webviews ({zoom:?})"));

    act(app, format!("acc:focus:{}", ids[1])).await;
    let v = view(app);
    let shown: Vec<usize> = v.panes.iter().enumerate().filter(|(_, p)| p.rect.is_some()).map(|(i, _)| i).collect();
    let others_hidden = native(app, ids[0]).await.is_some_and(|n| n.0) && native(app, ids[2]).await.is_some_and(|n| n.0);
    let zoom = native(app, ids[1]).await.map(|n| n.1);
    report.check(
        shown == vec![1] && others_hidden && zoom.is_some_and(|z| (z - 1.0).abs() < 0.01),
        format!("ampliada: só a 2ª aparece, as outras escondidas, zoom 100% ({shown:?}, {zoom:?})"),
    );
    let alive = eval(app, ids[0], "return document.readyState").await;
    report.check(alive.is_ok(), format!("conta escondida continua respondendo ({alive:?})"));
    act(app, format!("acc:focus:{}", ids[1])).await;
    let v = view(app);
    let zoom = native(app, ids[1]).await.map(|n| n.1);
    report.check(
        v.layout == crate::config::LayoutMode::Columns && zoom.is_some_and(|z| (z - 0.5).abs() < 0.01),
        format!("voltar: colunas de novo e zoom 50% ({:?}, {zoom:?})", v.layout),
    );

    act(app, format!("acc:power:{}", ids[2])).await;
    let n = native(app, ids[2]).await;
    let pane = view(app).panes.into_iter().find(|p| p.id == ids[2]);
    report.check(
        n.as_ref().is_some_and(|n| n.0 && n.2.starts_with("about:blank")) && pane.is_some_and(|p| p.off && p.rect.is_some()),
        format!("desligar: fica no deck, página em branco e escondida ({n:?})"),
    );
    act(app, format!("acc:power:{}", ids[2])).await;
    let loaded = wait_loaded(app, ids[2]).await;
    let hidden = native(app, ids[2]).await.map(|n| n.0);
    report.check(loaded && hidden == Some(false), format!("ligar de novo: o jogo volta ({hidden:?})"));

    on_main(app, move |app| crate::prompt::rename(app, first, "Renomear conta")).await;
    let mut field = Err(String::new());
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(prompt) = app.get_webview_window("prompt") {
            field = webkit::call_async(prompt.as_ref(), "return document.querySelector('#value').value", Duration::from_secs(5)).await;
            if field.as_deref().is_ok_and(|v| v.contains("Conta 1")) {
                break;
            }
        }
    }
    report.check(field.as_deref().is_ok_and(|v| v.contains("Conta 1")), format!("janela do nome abre com o nome atual ({field:?})"));
    if let Some(prompt) = app.get_webview_window("prompt") {
        let fits = webkit::call_async(
            prompt.as_ref(),
            "const r = document.querySelector('.actions').getBoundingClientRect(); return [Math.round(r.bottom), innerHeight]",
            Duration::from_secs(5),
        )
        .await;
        let ok = fits.as_deref().ok().and_then(|j| serde_json::from_str::<[f64; 2]>(j).ok()).is_some_and(|[bottom, height]| bottom <= height);
        report.check(ok, format!("os botões cabem na janela do nome (fundo, altura: {fits:?})"));
    }
    if let Some(prompt) = app.get_webview_window("prompt") {
        let _ = webkit::call_async(
            prompt.as_ref(),
            "document.querySelector('#value').value = '  Main  '; document.querySelector('#form').requestSubmit(); return true",
            Duration::from_secs(5),
        )
        .await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    report.check(
        view(app).panes[0].name == "Main" && app.get_webview_window("prompt").is_none(),
        format!("enviar o nome renomeia e fecha a janela ({})", view(app).panes[0].name),
    );
    act(app, format!("acc:swap:{first}:{}", ids[3])).await;
    let order: Vec<Uuid> = view(app).panes.iter().map(|p| p.id).collect();
    report.check(order[3] == first && order[0] == ids[3], "trocar de lugar: a 1ª e a 4ª trocam");
    act(app, format!("acc:swap:{first}:{}", ids[3])).await;

    let kept = ids[3];
    let _ = wait_loaded(app, kept).await;
    let marked = eval(app, kept, &MARK_JS.replace("ID", &format!("'{kept}'"))).await;
    report.check(marked.is_ok(), "marca na conta que vai ser removida mantendo os dados");
    on_main(app, move |app| crate::accounts::remove(app, kept, false)).await;
    let v = view(app);
    report.check(v.panes.len() == 3 && v.removed == 1 && v.can_add, "remover mantendo: sai do deck e vai para as removidas");
    act(app, format!("deck:restore:{kept}")).await;
    let back = view(app).panes.iter().any(|p| p.id == kept);
    let _ = wait_loaded(app, kept).await;
    report.check(back, "restaurar: volta com o mesmo id");
    sees_only_itself(app, report, kept, "restaurada").await;

    act(app, "deck:layout:grid".into()).await;
    let removed: Vec<Uuid> = view(app).panes.iter().map(|p| p.id).collect();
    for id in removed {
        on_main(app, move |app| crate::accounts::remove(app, id, true)).await;
    }
    let v = view(app);
    report.check(v.panes.len() == 1 && v.removed == 0, "a última conta não sai do deck");
}

const MOTION_PROBE_JS: &str = "let active = 0;\n\
    const walk = (rules) => { for (const r of rules) { if (r instanceof CSSMediaRule && r.media.mediaText === 'all') active++; if (r.cssRules) walk(r.cssRules); } };\n\
    for (const s of document.styleSheets) { try { walk(s.cssRules); } catch {} }\n\
    return { matches: matchMedia('(prefers-reduced-motion: reduce)').matches, active };";

const ECHO_TARGET_JS: &str = "document.getElementById('deck-eco')?.remove();\n\
    const b = document.createElement('button'); b.id = 'deck-eco'; b.textContent = 'Teste do eco';\n\
    b.dataset.count = '0'; b.onclick = () => { b.dataset.count = String(Number(b.dataset.count) + 1); };\n\
    document.body.append(b); return true;";

async fn automations(app: &AppHandle, report: &mut Report) {
    let a = app.state::<deck::Deck>().accounts()[0].id;
    let Some(b) = on_main(app, crate::accounts::add).await else {
        return report.check(false, "não criou a 2ª conta");
    };
    for id in [a, b] {
        let loaded = wait_loaded(app, id).await;
        report.check(loaded, format!("{id} carregou"));
    }
    tokio::time::sleep(Duration::from_secs(3)).await;

    let on = eval(app, a, MOTION_PROBE_JS).await;
    report.check(
        matches!(&on, Ok(v) if v["matches"] == true && v["active"].as_u64().unwrap_or(0) > 50),
        format!("menos animações (de fábrica): matchMedia e as regras @media do CSS ligadas ({on:?})"),
    );
    act(app, "deck:motion".into()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let off = eval(app, a, MOTION_PROBE_JS).await;
    report.check(
        matches!(&off, Ok(v) if v["active"] == 0),
        format!("desligar menos animações volta o CSS ({off:?})"),
    );
    act(app, "deck:motion".into()).await;

    for id in [a, b] {
        let _ = eval(app, id, ECHO_TARGET_JS).await;
    }
    let click = crate::automation::Click {
        path: "body > button".into(),
        tag: "button".into(),
        text: "Teste do eco".into(),
        fx: 0.5,
        fy: 0.5,
    };
    crate::automation::echo_from(app, a, click.clone()).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let count = |id| async move { eval(app, id, "return document.getElementById('deck-eco').dataset.count").await };
    let off_b = count(b).await;
    report.check(off_b.as_ref().is_ok_and(|v| v == "0"), format!("eco desligado: nada se repete ({off_b:?})"));
    act(app, "deck:echo".into()).await;
    crate::automation::echo_from(app, a, click).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (on_a, on_b) = (count(a).await, count(b).await);
    report.check(
        on_b.as_ref().is_ok_and(|v| v == "1") && on_a.as_ref().is_ok_and(|v| v == "0"),
        format!("eco ligado: o clique da 1ª se repete só na 2ª (1ª {on_a:?}, 2ª {on_b:?})"),
    );
    act(app, "deck:echo".into()).await;

    let ultra_state = |id| async move { eval(app, id, crate::automation::ULTRA_STATE_JS).await };
    act(app, "deck:ultra".into()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let chosen = ultra_state(b).await;
    report.check(
        chosen.as_ref().is_ok_and(|v| v == "escolhido" || v == "ligado"),
        format!("ULTRA fora do jogo fica escolhido para quando entrar ({chosen:?})"),
    );
    on_main(app, move |app| crate::accounts::reload(app, b)).await;
    let _ = wait_loaded(app, b).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after_reload = ultra_state(b).await;
    report.check(
        after_reload.as_ref().is_ok_and(|v| v == "escolhido" || v == "ligado"),
        format!("conta recarregada continua no ULTRA ({after_reload:?})"),
    );
    act(app, "deck:ultra".into()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let cleared = ultra_state(b).await;
    report.check(cleared.as_ref().is_ok_and(|v| v == "fora" || v == "normal"), format!("desligar o ULTRA ({cleared:?})"));

    let mut config = crate::config::RestockConfig::default();
    config.on = true;
    config.balls.keep = 12_345;
    crate::automation::set_restock(app, a, config).await;
    let summary = eval(app, a, "return window.__deckRecompra.summary()").await;
    report.check(
        matches!(&summary, Ok(v) if v["configured"] == true && v["on"] == true),
        format!("recompra: a configuração chega no recompra.js da página ({:?})", summary.as_ref().map(|v| (&v["configured"], &v["on"]))),
    );
    let saved = app.state::<deck::Deck>().accounts()[0].restock.clone();
    report.check(saved.on && saved.balls.keep == 12_345, "recompra: salva no deck.json");
    let restored = eval(app, b, "return window.__deckRecompra.summary().on").await;
    report.check(restored.as_ref().is_ok_and(|v| v == false), "recompra: a outra conta continua desligada");

    act(app, format!("acc:restock:{a}")).await;
    let mut title = String::new();
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(window) = app.get_webview_window(&format!("restock-{}", a.simple())) {
            if let Ok(t) = webkit::call_async(window.as_ref(), "return document.getElementById('balls-keep').value", Duration::from_secs(5)).await {
                title = t;
                if title.contains("12.345") {
                    break;
                }
            }
        }
    }
    report.check(title.contains("12.345"), format!("janela da recompra abre com a config da conta ({title})"));
}
