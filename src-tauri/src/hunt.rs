//! A extensão "/hunt for PokeIdle" (mostra, acima do pokémon em campo, onde caçar). Ela é só scripts e
//! CSS na página do jogo, sem API de navegador, então o deck faz o papel do navegador: baixa a versão
//! publicada na loja do Firefox, guarda na pasta de dados e injeta nas contas. O código dela não vem com
//! o deck (a licença é "todos os direitos reservados"); quem liga é que instala, como num navegador.

use crate::deck::{self, Deck};
use serde::Deserialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Manager};

const ADDON_API: &str = "https://addons.mozilla.org/api/v5/addons/addon/hunt-for-pokeidle/";
/// A lista pública da Shiny WishList que a extensão mostra; a ponte dela roda fora da página, que a CSP
/// do jogo barraria.
const WISHLIST_URLS: [&str; 2] = ["https://pokeidleguide.io/api/wishlist", "https://pokeidle-gg.pages.dev/api/wishlist"];
const TIMEOUT: Duration = Duration::from_secs(30);

/// A ponte da lista: a página pede por `postMessage` e o deck busca.
const BRIDGE_JS: &str = r#"(() => {
  if (window.top !== window) return;
  addEventListener('message', (ev) => {
    if (ev.source !== window || ev.data?.tipo !== 'pih:wishlist:pedir') return;
    const reply = (body) => postMessage({ tipo: 'pih:wishlist:lista', id: ev.data.id, ...body }, location.origin);
    const invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (typeof invoke !== 'function') return reply({ erro: 'sem o deck' });
    invoke('hunt_wishlist').then((dados) => reply({ dados }), (erro) => reply({ erro: String(erro) }));
  });
})();"#;

#[derive(Clone)]
pub struct Bundle {
    pub version: String,
    /// Os scripts do mundo da página, na ordem do manifest, já com o CSS.
    scripts: Vec<String>,
}

#[derive(Default)]
pub struct Hunt {
    bundle: Mutex<Option<Bundle>>,
}

#[derive(Deserialize)]
struct Manifest {
    name: String,
    version: String,
    #[serde(default)]
    content_scripts: Vec<ContentScript>,
}

#[derive(Deserialize)]
struct ContentScript {
    #[serde(default)]
    matches: Vec<String>,
    #[serde(default)]
    js: Vec<String>,
    #[serde(default)]
    css: Vec<String>,
    #[serde(default)]
    world: Option<String>,
}

fn root(app: &AppHandle) -> PathBuf {
    deck::data_dir(app).join("extensions").join("hunt-for-pokeidle")
}

/// Lê a versão instalada (a mais nova na pasta). Chamado no início.
pub fn load(app: &AppHandle) {
    let newest = std::fs::read_dir(root(app))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read_bundle(&e.path()).ok())
        .max_by(|a, b| compare_versions(&a.version, &b.version));
    *app.state::<Hunt>().bundle.lock().unwrap() = newest;
}

fn read_bundle(dir: &Path) -> Result<Bundle, String> {
    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if !manifest.name.to_lowercase().contains("hunt") {
        return Err(format!("não é a /hunt: {}", manifest.name));
    }
    let read = |file: &str| -> Result<String, String> {
        let path = dir.join(file);
        if !path.starts_with(dir) {
            return Err(format!("caminho inválido: {file}"));
        }
        std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))
    };
    let game = |c: &ContentScript| c.matches.iter().any(|m| m.contains("pokeidle.io"));
    let mut scripts = Vec::new();
    for entry in manifest.content_scripts.iter().filter(|c| game(c)) {
        for file in &entry.css {
            let css = serde_json::to_string(&read(file)?).unwrap_or_default();
            scripts.push(format!(
                "(() => {{ if (window.top !== window) return; const s = document.createElement('style'); \
                 s.dataset.deck = 'hunt'; s.textContent = {css}; \
                 (document.head || document.documentElement).appendChild(s); }})();"
            ));
        }
        // O que roda no mundo isolado é a ponte da lista, que o deck substitui por `BRIDGE_JS`.
        if entry.world.as_deref() == Some("MAIN") {
            for file in &entry.js {
                // No Windows os scripts de início entram também nos iframes (Cloudflare, anúncios): a extensão
                // inteira rodaria em cada um. Os arquivos dela são IIFEs, então o bloco não muda o escopo.
                scripts.push(format!("if (window.top === window) {{\n{}\n}}", read(file)?));
            }
        }
    }
    if scripts.is_empty() {
        return Err("a extensão não tem scripts para o jogo".into());
    }
    scripts.push(BRIDGE_JS.to_owned());
    Ok(Bundle { version: manifest.version, scripts })
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a).cmp(&parts(b))
}

/// Os scripts a injetar numa conta nova: vazio com a /hunt desligada ou ainda não baixada.
pub fn scripts(app: &AppHandle) -> Vec<String> {
    if !app.state::<Deck>().read(|c| c.hunt) {
        return Vec::new();
    }
    app.state::<Hunt>().bundle.lock().unwrap().as_ref().map(|b| b.scripts.clone()).unwrap_or_default()
}

pub fn installed_version(app: &AppHandle) -> Option<String> {
    app.state::<Hunt>().bundle.lock().unwrap().as_ref().map(|b| b.version.clone())
}

fn agent() -> ureq::Agent {
    crate::live_check::agent()
}

/// Baixa a versão publicada se for mais nova que a instalada. Devolve a versão em uso.
pub fn install(app: &AppHandle) -> Result<String, String> {
    let info: serde_json::Value = agent().get(ADDON_API).call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())?;
    let version = info["current_version"]["version"].as_str().ok_or("a loja não disse a versão")?.to_owned();
    let url = info["current_version"]["file"]["url"].as_str().ok_or("a loja não deu o arquivo")?.to_owned();
    if !url.starts_with("https://addons.mozilla.org/") {
        return Err(format!("endereço inesperado: {url}"));
    }
    if installed_version(app).is_some_and(|v| compare_versions(&v, &version) != std::cmp::Ordering::Less) {
        return Ok(installed_version(app).unwrap_or(version));
    }
    let mut bytes = Vec::new();
    agent()
        .get(&url)
        .timeout(TIMEOUT)
        .call()
        .map_err(|e| e.to_string())?
        .into_reader()
        .take(20 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let target = root(app).join(&version);
    let staging = root(app).join(format!(".{version}-baixando"));
    let _ = std::fs::remove_dir_all(&staging);
    unzip(&bytes, &staging)?;
    let bundle = read_bundle(&staging)?;
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&staging, &target).map_err(|e| e.to_string())?;
    for old in std::fs::read_dir(root(app)).into_iter().flatten().flatten() {
        if old.path() != target {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    *app.state::<Hunt>().bundle.lock().unwrap() = Some(bundle);
    Ok(version)
}

fn unzip(bytes: &[u8], into: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = file.enclosed_name() else { continue };
        let path = into.join(name);
        if file.is_dir() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        std::io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Liga ou desliga. Ligar pela primeira vez baixa a extensão; as contas reabrem com ou sem ela.
pub fn set(app: &AppHandle, on: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if on {
            if let Err(err) = install(&app) {
                use tauri_plugin_dialog::DialogExt;
                app.dialog()
                    .message(format!("Não deu para baixar a extensão /hunt: {err}"))
                    .title("Onde caçar (/hunt)")
                    .show(|_| {});
                return;
            }
        }
        app.state::<Deck>().update(|c| c.hunt = on);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || crate::accounts::respawn_all(&handle));
    });
}

/// No início, com a /hunt ligada: baixa a versão nova se saiu uma (vale a partir da próxima abertura).
pub fn update_later(app: &AppHandle) {
    if !app.state::<Deck>().read(|c| c.hunt) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(err) = install(&app) {
            eprintln!("[hunt] não conferiu a versão nova: {err}");
        }
    });
}

#[tauri::command]
pub fn hunt_wishlist() -> Result<serde_json::Value, String> {
    let mut last = String::new();
    for url in WISHLIST_URLS {
        match agent().get(url).timeout(Duration::from_secs(10)).call() {
            Ok(response) => return response.into_json().map_err(|e| e.to_string()),
            Err(err) => last = err.to_string(),
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::compare_versions;
    use std::cmp::Ordering;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("1.0.19", "1.0.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.19", "1.0.19"), Ordering::Equal);
        assert_eq!(compare_versions("1.1", "1.0.20"), Ordering::Greater);
    }
}
