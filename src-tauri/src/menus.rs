//! Menus nativos: a barra do macOS, os da barra de cima do deck e o de cada conta. HTML não desenha por
//! cima das webviews das contas, então nada disso pode ser dropdown da página.

use crate::{
    accounts, automation, lives,
    config::Site,
    config::{LayoutMode, MAX_ACCOUNTS, ZOOM_LEVELS},
    deck::{self, Deck},
};
use std::sync::Mutex;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, LogicalPosition, Manager, Window, Wry,
};
use uuid::Uuid;

/// O que a barra de menus mostra da lista de contas; ela só é refeita quando isto muda.
#[derive(Default)]
pub struct AppMenuState(Mutex<Option<String>>);

fn item(app: &AppHandle, id: impl AsRef<str>, text: &str, enabled: bool, accelerator: Option<&str>) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id.as_ref(), text, enabled, accelerator)
}

fn restore_submenu(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
    let (removed, can_add) = app
        .state::<Deck>()
        .read(|c| (c.removed.clone(), c.accounts.len() < MAX_ACCOUNTS));
    let submenu = Submenu::with_id(app, "deck:restore-menu", "Restaurar conta removida", !removed.is_empty() && can_add)?;
    for account in &removed {
        submenu.append(&item(app, format!("deck:restore:{}", account.id), &account.name, true, None)?)?;
    }
    Ok(submenu)
}

fn layout_items(app: &AppHandle, with_accelerators: bool) -> tauri::Result<Vec<CheckMenuItem<Wry>>> {
    let (layout, stacked, count) = app.state::<Deck>().read(|c| (c.layout, c.stacked, c.accounts.len()));
    let accel = |key: &'static str| with_accelerators.then_some(key);
    Ok(vec![
        CheckMenuItem::with_id(app, "deck:layout:grid", "Grade", true, layout == LayoutMode::Grid, accel("CmdOrCtrl+1"))?,
        CheckMenuItem::with_id(app, "deck:layout:columns", "Colunas", true, layout == LayoutMode::Columns, accel("CmdOrCtrl+2"))?,
        CheckMenuItem::with_id(app, "deck:layout:focus", "Uma ampliada", true, layout == LayoutMode::Focus, accel("CmdOrCtrl+3"))?,
        CheckMenuItem::with_id(app, "deck:stack", "Duas contas empilhadas", count == 2, stacked, None::<&str>)?,
    ])
}

pub fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let can_add = app.state::<Deck>().read(|c| c.accounts.len() < MAX_ACCOUNTS);
    let about = PredefinedMenuItem::about(app, Some("Sobre o PokeIdle Deck"), None)?;
    let app_submenu = Submenu::with_items(
        app,
        "PokeIdle Deck",
        true,
        &[
            &about,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("Ocultar o PokeIdle Deck"))?,
            &PredefinedMenuItem::hide_others(app, Some("Ocultar os outros"))?,
            &PredefinedMenuItem::show_all(app, Some("Mostrar todos"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("Sair do PokeIdle Deck"))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Editar",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("Desfazer"))?,
            &PredefinedMenuItem::redo(app, Some("Refazer"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("Recortar"))?,
            &PredefinedMenuItem::copy(app, Some("Copiar"))?,
            &PredefinedMenuItem::paste(app, Some("Colar"))?,
            &PredefinedMenuItem::select_all(app, Some("Selecionar tudo"))?,
        ],
    )?;
    let accounts_menu = Submenu::with_items(
        app,
        "Contas",
        true,
        &[
            &item(app, "deck:add", "+ Nova conta", can_add, Some("CmdOrCtrl+N"))?,
            &restore_submenu(app)?,
        ],
    )?;
    let view = Submenu::new(app, "Visualização", true)?;
    for entry in layout_items(app, true)? {
        view.append(&entry)?;
    }
    view.append_items(&[
        &PredefinedMenuItem::separator(app)?,
        &item(app, "deck:zoom:in", "Aumentar o zoom", true, Some("CmdOrCtrl+="))?,
        &item(app, "deck:zoom:out", "Diminuir o zoom", true, Some("CmdOrCtrl+-"))?,
        &item(app, "deck:zoom:reset", "Zoom 100%", true, Some("CmdOrCtrl+0"))?,
        &PredefinedMenuItem::separator(app)?,
        &item(app, "deck:fullscreen", "Tela cheia", true, Some("F11"))?,
        &PredefinedMenuItem::separator(app)?,
    ])?;
    for (index, account) in app.state::<Deck>().accounts().iter().enumerate() {
        view.append(&item(
            app,
            format!("deck:focus:{}", index + 1),
            &format!("Ampliar {}", account.name),
            true,
            Some(&format!("CmdOrCtrl+Alt+{}", index + 1)),
        )?)?;
    }
    let window = Submenu::with_items(
        app,
        "Janela",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("Minimizar"))?,
            &PredefinedMenuItem::maximize(app, Some("Zoom"))?,
            &PredefinedMenuItem::close_window(app, Some("Fechar"))?,
        ],
    )?;
    Menu::with_items(app, &[&app_submenu, &edit, &accounts_menu, &view, &window])
}

/// Refaz a barra de menus quando muda o que ela mostra (contas, removidas, layout). Main thread.
pub fn sync_app_menu(app: &AppHandle) {
    let signature = app.state::<Deck>().read(|c| {
        format!(
            "{:?}|{:?}|{:?}|{}",
            c.accounts.iter().map(|a| &a.name).collect::<Vec<_>>(),
            c.removed.iter().map(|a| (a.id, &a.name)).collect::<Vec<_>>(),
            c.layout,
            c.stacked
        )
    });
    let state = app.state::<AppMenuState>();
    let mut last = state.0.lock().unwrap();
    if last.as_deref() == Some(signature.as_str()) {
        return;
    }
    match app_menu(app).and_then(|menu| app.set_menu(menu)) {
        Ok(_) => *last = Some(signature),
        Err(err) => eprintln!("[deck] barra de menus: {err}"),
    }
}

fn popup_at(app: &AppHandle, window: &Window, menu: &Menu<Wry>, x: f64, y: f64) -> Result<(), String> {
    let inset = app.state::<Deck>().inset();
    window
        .popup_menu_at(menu, LogicalPosition::new(x, y + inset))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn deck_menu(app: AppHandle, window: Window, x: f64, y: f64) -> Result<(), String> {
    let build = || -> tauri::Result<Menu<Wry>> {
        let (stacked, count) = app.state::<Deck>().read(|c| (c.stacked, c.accounts.len()));
        Menu::with_items(
            &app,
            &[
                &restore_submenu(&app)?,
                &PredefinedMenuItem::separator(&app)?,
                &CheckMenuItem::with_id(&app, "deck:stack", "Duas contas empilhadas", count == 2, stacked, None::<&str>)?,
            ],
        )
    };
    let menu = build().map_err(|e| e.to_string())?;
    popup_at(&app, &window, &menu, x, y)
}

#[tauri::command]
pub fn zoom_menu(app: AppHandle, window: Window, x: f64, y: f64) -> Result<(), String> {
    let zoom = app.state::<Deck>().read(|c| c.zoom);
    let build = || -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(&app)?;
        for level in ZOOM_LEVELS {
            menu.append(&CheckMenuItem::with_id(
                &app,
                format!("deck:zoom:set:{level}"),
                format!("{:.0}%", level * 100.0),
                true,
                (level - zoom).abs() < 0.001,
                None::<&str>,
            )?)?;
        }
        Ok(menu)
    };
    let menu = build().map_err(|e| e.to_string())?;
    popup_at(&app, &window, &menu, x, y)
}

#[tauri::command]
pub fn ultra_menu(app: AppHandle, window: Window, x: f64, y: f64) -> Result<(), String> {
    let (ultra, reduce) = app.state::<Deck>().read(|c| (c.ultra, c.reduce_motion));
    let build = || -> tauri::Result<Menu<Wry>> {
        Menu::with_items(
            &app,
            &[
                &item(&app, "deck:ultra", if ultra { "Desligar o ULTRA" } else { "Ligar o ULTRA (em todas as contas)" }, true, None)?,
                &PredefinedMenuItem::separator(&app)?,
                &CheckMenuItem::with_id(&app, "deck:motion", "Menos animações", true, reduce, None::<&str>)?,
            ],
        )
    };
    let menu = build().map_err(|e| e.to_string())?;
    popup_at(&app, &window, &menu, x, y)
}

#[tauri::command]
pub fn account_menu(app: AppHandle, window: Window, id: Uuid) -> Result<(), String> {
    let accounts = app.state::<Deck>().accounts();
    let Some(index) = accounts.iter().position(|a| a.id == id) else { return Ok(()) };
    let account = &accounts[index];
    let focused = app
        .state::<Deck>()
        .read(|c| c.layout == LayoutMode::Focus && c.focused == Some(id));
    let entry = |action: &str, text: &str, enabled: bool| item(&app, format!("acc:{action}:{id}"), text, enabled, None);
    let lives_on = app.state::<Deck>().read(|c| c.lives_accounts().contains(&id));
    let lives_submenu = || -> tauri::Result<Submenu<Wry>> {
        let submenu = Submenu::new(&app, "Fechar ou abrir lives", lives::has_window(&app, id))?;
        for (index, site) in [Site::Kick, Site::Twitch].into_iter().enumerate() {
            if index > 0 {
                submenu.append(&PredefinedMenuItem::separator(&app)?)?;
            }
            submenu.append(&item(&app, "lives:header", &format!("Lives da {}:", site.name()), false, None)?)?;
            let open = lives::open_channels(&app, id, site);
            let site_key = site.name().to_lowercase();
            for channel in &open {
                submenu.append(&item(&app, format!("lives:close:{id}:{site_key}:{channel}"), &format!("Fechar a live de {channel}"), true, None)?)?;
            }
            if !open.is_empty() {
                submenu.append(&item(&app, format!("lives:close:{id}:{site_key}:*"), &format!("Fechar todas (nenhuma live da {} nesta conta)", site.name()), true, None)?)?;
            }
            let limit = account.lives.limit(site).clone();
            if limit.limited() {
                for channel in &limit.closed {
                    submenu.append(&item(&app, format!("lives:reopen:{id}:{site_key}:{channel}"), &format!("Abrir de novo a live de {channel}"), true, None)?)?;
                }
                submenu.append(&item(&app, format!("lives:reopen:{id}:{site_key}:*"), "Abrir todas de novo", true, None)?)?;
            } else if open.is_empty() {
                submenu.append(&item(&app, "lives:none", &format!("Nenhuma live da {} aberta nesta conta agora", site.name()), false, None)?)?;
            }
        }
        Ok(submenu)
    };
    let swap_submenu = || -> tauri::Result<Submenu<Wry>> {
        let submenu = Submenu::new(&app, "Trocar de lugar com", accounts.len() > 1)?;
        for (position, other) in accounts.iter().enumerate().filter(|(_, a)| a.id != id) {
            submenu.append(&item(
                &app,
                format!("acc:swap:{id}:{}", other.id),
                &format!("{} ({}ª)", other.name, position + 1),
                true,
                None,
            )?)?;
        }
        Ok(submenu)
    };
    let build = || -> tauri::Result<Menu<Wry>> {
        Menu::with_items(
            &app,
            &[
                &entry("focus", if focused { "Voltar ao layout de antes" } else { "Ampliar no deck" }, true)?,
                &entry("reload", "Recarregar a página", !account.off)?,
                &entry("mute", if account.muted { "Ligar o som" } else { "Desligar o som" }, true)?,
                &entry("rename", "Renomear…", true)?,
                &entry("restock", "Recompra…", true)?,
                &CheckMenuItem::with_id(&app, format!("lives:toggle:{id}"), "Lives da Twitch/Kick nesta conta", true, lives_on, None::<&str>)?,
                &lives_submenu()?,
                &entry("power", if account.off { "Ligar esta conta" } else { "Desligar esta conta (fica no deck, sem rodar nada)" }, true)?,
                &PredefinedMenuItem::separator(&app)?,
                &swap_submenu()?,
                &PredefinedMenuItem::separator(&app)?,
                &entry("inspect", "Inspecionar página", !account.off)?,
                &entry("remove", "Remover conta…", accounts.len() > 1)?,
            ],
        )
    };
    let menu = build().map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    dispatch(app, event.id().as_ref());
}

/// Ações por texto, as mesmas dos menus e dos botões da página do deck ("acc:power:<id>",
/// "deck:layout:grid"...). Main thread.
pub fn dispatch(app: &AppHandle, action: &str) {
    let mut parts = action.splitn(3, ':');
    let (scope, verb, rest) = (parts.next(), parts.next(), parts.next());
    let deck = app.state::<Deck>();
    match (scope, verb) {
        (Some("acc"), Some(verb)) => {
            let mut ids = rest.unwrap_or_default().split(':').map(Uuid::parse_str);
            let Some(Ok(id)) = ids.next() else { return };
            let other = ids.next().and_then(Result::ok);
            match verb {
                "focus" => {
                    deck.update(|c| c.toggle_focus(id));
                    deck::refresh(app);
                }
                "power" => {
                    let off = deck.read(|c| c.accounts.iter().find(|a| a.id == id).map(|a| a.off));
                    if let Some(off) = off {
                        accounts::set_off(app, id, !off);
                    }
                }
                "mute" => accounts::toggle_mute(app, id),
                "reload" => accounts::reload(app, id),
                "rename" => crate::prompt::rename(app, id, "Renomear conta"),
                "swap" => {
                    if let Some(other) = other {
                        accounts::swap(app, id, other);
                    }
                }
                "inspect" => accounts::inspect(app, id),
                "restock" => automation::open_restock(app, id),
                "remove" => accounts::confirm_remove(app, id),
                _ => {}
            }
        }
        (Some("lives"), Some(verb)) => lives_action(app, verb, rest.unwrap_or_default()),
        (Some("deck"), Some("add")) => {
            accounts::add(app);
        }
        (Some("deck"), Some("restore")) => {
            if let Some(id) = rest.and_then(|r| Uuid::parse_str(r).ok()) {
                accounts::restore(app, id);
            }
        }
        (Some("deck"), Some("stack")) => {
            deck.update(|c| c.stacked = !c.stacked);
            deck::refresh(app);
        }
        (Some("deck"), Some("layout")) => {
            let layout = match rest {
                Some("grid") => LayoutMode::Grid,
                Some("columns") => LayoutMode::Columns,
                Some("focus") => LayoutMode::Focus,
                _ => return,
            };
            deck.update(|c| c.set_layout(layout));
            deck::refresh(app);
        }
        (Some("deck"), Some("zoom")) => {
            deck.update(|c| match rest {
                Some("in") => c.step_zoom(1),
                Some("out") => c.step_zoom(-1),
                Some("reset") => c.zoom = 1.0,
                Some(set) => {
                    if let Some(level) = set.strip_prefix("set:").and_then(|l| l.parse::<f64>().ok()) {
                        c.zoom = crate::config::closest_zoom(level);
                    }
                }
                None => {}
            });
            deck::refresh(app);
        }
        (Some("deck"), Some("fullscreen")) => toggle_fullscreen(app),
        (Some("deck"), Some("focus")) => {
            let index = rest.and_then(|r| r.parse::<usize>().ok()).and_then(|n| n.checked_sub(1));
            let id = index.and_then(|i| deck.read(|c| c.accounts.get(i).map(|a| a.id)));
            if let Some(id) = id {
                deck.update(|c| c.focus(id));
                deck::refresh(app);
            }
        }
        (Some("deck"), Some("ultra")) => automation::set_ultra(app, !deck.read(|c| c.ultra)),
        (Some("deck"), Some("motion")) => automation::set_reduce_motion(app, !deck.read(|c| c.reduce_motion)),
        (Some("deck"), Some("echo")) => automation::set_echo(app, !app.state::<automation::Automation>().echo()),
        _ => {}
    }
}

fn lives_action(app: &AppHandle, verb: &str, rest: &str) {
    let mut parts = rest.splitn(3, ':');
    let id = parts.next().and_then(|s| Uuid::parse_str(s).ok());
    let site = match parts.next() {
        Some("twitch") => Some(Site::Twitch),
        Some("kick") => Some(Site::Kick),
        _ => None,
    };
    let channel = parts.next().filter(|c| *c != "*").map(String::from);
    match (verb, id, site) {
        ("toggle", Some(id), _) => lives::toggle_account(app, id),
        ("close", Some(id), Some(site)) => lives::close_live(app, id, site, channel),
        ("reopen", Some(id), Some(site)) => lives::reopen_live(app, id, site, channel),
        ("show", _, _) => lives::reveal_all(app),
        ("points", Some(id), _) => lives::set_points(app, id, true),
        ("config", _, _) => crate::lives_config::open(app),
        ("log", _, _) => crate::points::open_log_window(app),
        _ => {}
    }
}

#[tauri::command]
pub fn lives_menu(app: AppHandle, window: Window, x: f64, y: f64) -> Result<(), String> {
    let accounts = app.state::<Deck>().accounts();
    let on = app.state::<Deck>().read(|c| c.lives_accounts());
    let build = || -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(&app)?;
        menu.append(&item(&app, "lives:header", "Marque a conta vinculada à sua Twitch/Kick (a main):", false, None)?)?;
        for account in &accounts {
            let detail = if account.off {
                " — conta desligada (as lives voltam ao ligar)".to_owned()
            } else {
                lives::summary(&app, account.id).map(|s| format!(" — {s}")).unwrap_or_default()
            };
            menu.append(&CheckMenuItem::with_id(
                &app,
                format!("lives:toggle:{}", account.id),
                format!("{}{detail}", account.name),
                true,
                on.contains(&account.id),
                None::<&str>,
            )?)?;
        }
        menu.append(&PredefinedMenuItem::separator(&app)?)?;
        let any = accounts.iter().any(|a| lives::has_window(&app, a.id));
        for account in accounts.iter().filter(|a| lives::has_window(&app, a.id)) {
            menu.append(&item(&app, format!("lives:points:{}", account.id), &format!("Resgatar pontos na mão: {}", account.name), true, None)?)?;
        }
        menu.append(&item(&app, "lives:show", "Mostrar as lives (para entrar na Twitch/Kick ou ver os players)", any, None)?)?;
        let today = crate::points::today_count(&app);
        menu.append(&item(&app, "lives:log", &format!("Registro dos resgates… ({today} hoje)"), true, None)?)?;
        menu.append(&item(&app, "lives:config", "Config…", true, None)?)?;
        Ok(menu)
    };
    let menu = build().map_err(|e| e.to_string())?;
    popup_at(&app, &window, &menu, x, y)
}

pub fn toggle_fullscreen(app: &AppHandle) {
    if let Some(window) = deck::main_window(app) {
        let on = window.is_fullscreen().unwrap_or(false);
        let _ = window.set_fullscreen(!on);
    }
}

#[tauri::command]
pub fn deck_state(app: AppHandle) -> Option<deck::DeckView> {
    deck::view(&app)
}

#[tauri::command]
pub fn deck_action(app: AppHandle, action: String) {
    dispatch(&app, &action);
}

/// Teclas que a página do jogo repassa (só o F11; a página não manda em mais nada do deck).
#[tauri::command]
pub fn page_key(app: AppHandle, key: String) {
    if key == "F11" {
        toggle_fullscreen(&app);
    }
}
