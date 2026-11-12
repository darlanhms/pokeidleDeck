//! A RAM do deck separada em jogo · lives · navegador, como na barra de baixo do original. É a
//! "memória" do Monitor de Atividade (phys_footprint) de cada processo do WebKit que o deck abriu.

use crate::accounts;
use serde::Serialize;
use std::{collections::HashSet, time::Duration};
use tauri::{AppHandle, Emitter, Manager};

const EVERY: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub game: u64,
    pub lives: u64,
    pub other: u64,
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(EVERY).await;
            let memory = measure(&app).await;
            let _ = app.emit_to("deck", "memory", memory);
        }
    });
}

#[cfg(target_os = "macos")]
async fn measure(app: &AppHandle) -> Memory {
    let mut game = HashSet::new();
    let mut lives = HashSet::new();
    for webview in app.webviews().into_values() {
        let Some(pid) = crate::webkit::web_process(&webview).await.map(|p| p as i32) else { continue };
        if accounts::id_from_label(webview.label()).is_some() {
            game.insert(pid);
        } else if webview.label().starts_with("kick-") || webview.label().starts_with("live-") {
            lives.insert(pid);
        }
    }
    let lives: HashSet<i32> = lives.difference(&game).copied().collect();
    let me = std::process::id() as i32;
    let other: HashSet<i32> = webkit_processes_of(me)
        .into_iter()
        .chain([me])
        .filter(|pid| !game.contains(pid) && !lives.contains(pid))
        .collect();
    let sum = |pids: &HashSet<i32>| pids.iter().map(|&pid| footprint(pid)).sum();
    Memory { game: sum(&game), lives: sum(&lives), other: sum(&other) }
}

/// No WebView2 o jogo e as lives de uma conta dividem o navegador da pasta dela: tudo entra no jogo.
/// O resto (a interface do deck e o próprio deck) é o navegador.
#[cfg(windows)]
async fn measure(app: &AppHandle) -> Memory {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut accounts = HashSet::new();
    for webview in app.webviews().into_values() {
        if accounts::id_from_label(webview.label()).is_none() && !webview.label().starts_with("live-") {
            continue;
        }
        if let Some(pid) = crate::webkit::web_process(&webview).await {
            accounts.insert(pid);
        }
    }
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_memory());
    let me = Pid::from_u32(std::process::id());
    // A raiz de cada processo: o navegador da conta, o do deck, ou nenhum dos dois.
    let root = |pid: Pid| -> Option<Pid> {
        let mut current = pid;
        for _ in 0..8 {
            if current == me || accounts.contains(&current.as_u32()) {
                return Some(current);
            }
            current = system.process(current)?.parent()?;
        }
        None
    };
    let mut memory = Memory::default();
    for (pid, process) in system.processes() {
        match root(*pid) {
            Some(r) if r != me => memory.game += process.memory(),
            Some(_) => memory.other += process.memory(),
            None => {}
        }
    }
    memory
}

/// Os processos do WebKit (rede, GPU, páginas) que o macOS atribui ao deck. A função que diz isso é
/// privada da libSystem; sem ela, sobra só o processo do próprio deck no "navegador". Aberto pelo
/// terminal, o responsável é o terminal e não o deck: vale quem tem o mesmo responsável que o deck e
/// nasceu depois dele.
#[cfg(target_os = "macos")]
fn webkit_processes_of(me: i32) -> Vec<i32> {
    type Responsible = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"responsibility_get_pid_responsible_for_pid".as_ptr()) };
    if symbol.is_null() {
        return Vec::new();
    }
    let responsible: Responsible = unsafe { std::mem::transmute(symbol) };
    let owner = unsafe { responsible(me) };
    let born = started_at(me).unwrap_or(0);
    all_pids()
        .into_iter()
        .filter(|&pid| pid != me && unsafe { responsible(pid) } == owner)
        .filter(|&pid| name(pid).is_some_and(|n| n.starts_with("com.apple.WebKit")))
        .filter(|&pid| started_at(pid).is_some_and(|t| t >= born))
        .collect()
}

#[cfg(target_os = "macos")]
fn started_at(pid: i32) -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
    (read == size).then_some(info.pbi_start_tvsec)
}

#[cfg(target_os = "macos")]
fn all_pids() -> Vec<i32> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return Vec::new();
    }
    let mut pids = vec![0i32; count as usize + 64];
    let bytes = (pids.len() * std::mem::size_of::<i32>()) as libc::c_int;
    let filled = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
    pids.truncate(filled.max(0) as usize);
    pids.retain(|&pid| pid > 0);
    pids
}

#[cfg(target_os = "macos")]
fn name(pid: i32) -> Option<String> {
    let mut buffer = [0u8; 256];
    let len = unsafe { libc::proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    if len <= 0 {
        return None;
    }
    std::ffi::CStr::from_bytes_until_nul(&buffer).ok().map(|s| s.to_string_lossy().into_owned())
}

#[cfg(target_os = "macos")]
fn footprint(pid: i32) -> u64 {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, (&mut info as *mut libc::rusage_info_v4).cast())
    };
    if ok == 0 { info.ri_phys_footprint } else { 0 }
}
