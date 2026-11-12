/// Como uma página de conta é criada: o motor (WebKit no macOS, WebView2 no Windows) traduz isso.
#[derive(Clone, Copy, Default)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct Page {
    /// Vídeo com som toca sem clique (as lives).
    pub autoplay: bool,
    /// Página escondida continua rodando no mesmo ritmo (jogo em segundo plano, lives fora da tela).
    pub keep_running_hidden: bool,
    /// Bloqueia anúncios e rastreadores (as lives).
    pub block_ads: bool,
}

/// Anúncios e rastreadores das lives (a mesma lista do original). Vídeo, Kick, Twitch e o Cloudflare
/// (anti-robô) passam.
pub const BLOCKED: [&str; 29] = [
    "googlesyndication.com", "doubleclick.net", "googleadservices.com", "adtrafficquality.google",
    "imasdk.googleapis.com", "openx.net", "openxcdn.net", "criteo.com", "criteo.net", "dnacdn.net",
    "creativecdn.com", "id5-sync.com", "amazon-adsystem.com", "googletagmanager.com",
    "google-analytics.com", "tiktok.com", "tiktokw.us", "facebook.net", "facebook.com",
    "analytics.yahoo.com", "quora.com", "litix.io", "datazoom.io", "datadoghq-browser-agent.com",
    "cloudflareinsights.com", "scorecardresearch.com", "onetrust.com", "stripe.com", "stripe.network",
];

/// O JS que `call_async` roda: o corpo vira uma função async e o valor volta em texto JSON.
pub fn async_body(script: &str) -> String {
    format!(
        "const __deckValue = await (async () => {{\n{script}\n}})();\n\
         return JSON.stringify(__deckValue === undefined ? null : __deckValue);"
    )
}
