use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};
use uuid::Uuid;

pub const MAX_ACCOUNTS: usize = 4;

pub const ZOOM_LEVELS: [f64; 9] = [0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Também é o identificador do `WKWebsiteDataStore`: nunca muda, nem ao reordenar.
    pub id: Uuid,
    pub name: String,
    /// Desligada: continua no deck, mas não carrega nada.
    #[serde(default)]
    pub off: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub restock: RestockConfig,
    #[serde(default)]
    pub lives: AccountLives,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Site {
    Twitch,
    Kick,
}

impl Site {
    pub fn name(self) -> &'static str {
        match self {
            Site::Twitch => "Twitch",
            Site::Kick => "Kick",
        }
    }
}

/// O bônus do PokeIdle que a conta resgata com os pontos do canal (o custo de verdade vem do canal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reward {
    Xp,
    Capture,
    Lure,
}

/// O que toca numa vaga de live da conta.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "channel")]
pub enum SlotChoice {
    /// A primeira da lista que estiver no ar.
    #[default]
    Auto,
    /// Este canal; fora do ar, a vaga toca a primeira da lista no ar e volta quando ele voltar.
    Channel(String),
    /// Vaga vazia.
    Off,
}

/// As escolhas da conta num site: o que toca em cada vaga e os canais que ela tirou da lista.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LiveLimit {
    pub chosen: Vec<SlotChoice>,
    pub closed: Vec<String>,
}

impl LiveLimit {
    pub fn choice(&self, slot: usize) -> SlotChoice {
        self.chosen.get(slot).cloned().unwrap_or_default()
    }

    pub fn choices(&self, slots: usize) -> Vec<SlotChoice> {
        (0..slots).map(|i| self.choice(i)).collect()
    }

    pub fn set_choice(&mut self, slot: usize, choice: SlotChoice) {
        if self.chosen.len() <= slot {
            self.chosen.resize(slot + 1, SlotChoice::Auto);
        }
        self.chosen[slot] = choice;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AccountLives {
    pub twitch: LiveLimit,
    pub kick: LiveLimit,
    pub redeem_twitch: Option<Reward>,
    pub redeem_kick: Option<Reward>,
}

impl AccountLives {
    pub fn limit(&self, site: Site) -> &LiveLimit {
        match site {
            Site::Twitch => &self.twitch,
            Site::Kick => &self.kick,
        }
    }

    pub fn limit_mut(&mut self, site: Site) -> &mut LiveLimit {
        match site {
            Site::Twitch => &mut self.twitch,
            Site::Kick => &mut self.kick,
        }
    }
}

pub const MAX_LIVE_SLOTS: usize = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LivesConfig {
    /// Contas que mantêm lives abertas (a vinculada à Twitch/Kick, normalmente a main). `None` = de
    /// fábrica: a primeira conta.
    pub accounts: Option<Vec<Uuid>>,
    /// Em ordem de prioridade: abrem as primeiras que estiverem no ar.
    pub twitch_channels: Vec<String>,
    pub kick_channels: Vec<String>,
    pub twitch_slots: usize,
    /// 2 de fábrica: a Kick só dá pontos em 2 lives ao mesmo tempo por conta.
    pub kick_slots: usize,
    /// Pegar o baú de bônus da Twitch (+50 a cada ~15 min).
    pub claim_chest: bool,
    pub points_height: f64,
}

impl Default for LivesConfig {
    fn default() -> Self {
        let list = |s: &str| s.split(' ').map(String::from).collect();
        Self {
            accounts: None,
            // Os streamers oficiais do painel de bônus do jogo (outubro de 2026).
            twitch_channels: list("tftoddy clovaobengala axtlol patopapao matta guaxilol1 rodil cogu luccatv faasii biscoitao techzz dersin2k denissnidertv"),
            kick_channels: list("biscoitao yetz surskity"),
            twitch_slots: 2,
            kick_slots: 2,
            claim_chest: true,
            points_height: 640.0,
        }
    }
}

impl LivesConfig {
    pub fn channels(&self, site: Site) -> &[String] {
        match site {
            Site::Twitch => &self.twitch_channels,
            Site::Kick => &self.kick_channels,
        }
    }

    pub fn slots(&self, site: Site) -> usize {
        let max = match site {
            Site::Twitch => self.twitch_slots,
            Site::Kick => self.kick_slots,
        };
        max.min(MAX_LIVE_SLOTS).min(self.channels(site).len())
    }
}

/// Um canal por linha, como aparece no endereço (twitch.tv/rodil → rodil); aceita colar o link.
pub fn parse_channels(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split(|c: char| c == '\n' || c == '\r' || c == ',' || c == ' ') {
        let part = part.trim().trim_end_matches('/');
        let part = part.rsplit('/').next().unwrap_or("").trim_start_matches('@').to_lowercase();
        if !part.is_empty() && !out.contains(&part) {
            out.push(part);
        }
    }
    out
}

pub const MAX_STOCK: i64 = 10_000_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RestockRule {
    pub on: bool,
    /// Item do catálogo do jogo (a pokebola ou a poção; 0 na poção é "melhor custo-benefício").
    pub id: i64,
    pub keep: i64,
    pub below: i64,
}

/// A recompra de uma conta, no formato que o `recompra.js` lê em `configure`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RestockConfig {
    pub on: bool,
    pub balls: RestockRule,
    /// Contados juntos; compra sempre o mais barato.
    pub revives: RestockRule,
    /// Contadas juntas.
    pub potions: RestockRule,
    /// Pôr a poção escolhida em 1º na fila da automação "Usar Poções" (a ordem dela é a ordem de gasto).
    pub potion_first: bool,
    /// Nunca comprar a ponto de o ouro ficar abaixo disto.
    pub gold_floor: i64,
}

pub const ULTRA_BALL: i64 = 4;
pub const ULTIMATE_POTION: i64 = 204;

impl Default for RestockConfig {
    fn default() -> Self {
        Self {
            on: false,
            balls: RestockRule { on: true, id: ULTRA_BALL, keep: 30_000, below: 5_000 },
            revives: RestockRule { on: true, id: 0, keep: 500, below: 100 },
            potions: RestockRule { on: true, id: ULTIMATE_POTION, keep: 5_000, below: 2_000 },
            potion_first: true,
            gold_floor: 1_000_000,
        }
    }
}

impl RestockConfig {
    /// O que veio da janela ou de um deck.json editado à mão, dentro dos limites.
    pub fn normalized(mut self) -> Self {
        for rule in [&mut self.balls, &mut self.revives, &mut self.potions] {
            rule.keep = rule.keep.clamp(0, MAX_STOCK);
            rule.below = rule.below.clamp(0, MAX_STOCK);
            rule.id = rule.id.max(0);
        }
        self.gold_floor = self.gold_floor.max(0);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutMode {
    #[default]
    Grid,
    Columns,
    /// Uma conta ocupa o deck; as outras seguem rodando escondidas.
    Focus,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeckConfig {
    pub accounts: Vec<Account>,
    /// Contas removidas cujo data store foi mantido, para "Restaurar conta removida…".
    pub removed: Vec<Account>,
    /// Data stores de contas removidas com "apagar dados", apagados do disco no próximo início.
    pub pending_delete: Vec<Uuid>,
    pub layout: LayoutMode,
    /// Para onde "voltar" ao sair da conta ampliada.
    pub layout_before_focus: LayoutMode,
    pub focused: Option<Uuid>,
    /// Com 2 contas na grade: uma em cima da outra em vez de lado a lado.
    pub stacked: bool,
    pub zoom: f64,
    /// Ampliar põe o zoom em 100%; voltar devolve este (o que fazia as contas caberem na tela).
    pub zoom_outside_focus: Option<f64>,
    pub window: Option<WindowBounds>,
    pub maximized: bool,
    /// O Modo ULTRA Economia do jogo em todas as contas; a que recarrega volta a ele sozinha.
    pub ultra: bool,
    /// A página vê o "reduzir movimento" ligado e o jogo corta as animações contínuas.
    pub reduce_motion: bool,
    pub lives: LivesConfig,
}

impl Default for DeckConfig {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            removed: Vec::new(),
            pending_delete: Vec::new(),
            layout: LayoutMode::Grid,
            layout_before_focus: LayoutMode::Grid,
            focused: None,
            stacked: false,
            zoom: 1.0,
            zoom_outside_focus: None,
            window: None,
            maximized: false,
            ultra: false,
            reduce_motion: true,
            lives: LivesConfig::default(),
        }
    }
}

impl DeckConfig {
    pub fn load(path: &Path) -> Self {
        let mut config: DeckConfig = fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        config.accounts.truncate(MAX_ACCOUNTS);
        if config.accounts.is_empty() {
            let first = config.new_account();
            config.accounts.push(first);
        }
        config.zoom = closest_zoom(config.zoom);
        config.fix_focus();
        config
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn new_account(&self) -> Account {
        let taken = |name: &str| self.accounts.iter().chain(&self.removed).any(|a| a.name == name);
        let name = (1..)
            .map(|n| format!("Conta {n}"))
            .find(|name| !taken(name))
            .expect("sequência infinita");
        Account {
            id: Uuid::new_v4(),
            name,
            off: false,
            muted: false,
            restock: RestockConfig::default(),
            lives: AccountLives::default(),
        }
    }

    /// As contas com lives abertas, na ordem do deck.
    pub fn lives_accounts(&self) -> Vec<Uuid> {
        match &self.lives.accounts {
            Some(ids) => self.accounts.iter().filter(|a| ids.contains(&a.id)).map(|a| a.id).collect(),
            None => self.accounts.first().map(|a| vec![a.id]).unwrap_or_default(),
        }
    }

    pub fn set_lives_account(&mut self, id: Uuid, on: bool) {
        let mut ids = self.lives_accounts();
        ids.retain(|x| *x != id);
        if on {
            ids.push(id);
        }
        self.lives.accounts = Some(ids);
    }

    pub fn account_mut(&mut self, id: Uuid) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.id == id)
    }

    /// A conta ampliada sumiu (removida): volta para o layout de antes.
    pub fn fix_focus(&mut self) {
        let exists = self.focused.is_some_and(|id| self.accounts.iter().any(|a| a.id == id));
        if !exists {
            self.focused = None;
            if self.layout == LayoutMode::Focus {
                self.leave_focus();
            }
        }
    }

    pub fn set_layout(&mut self, layout: LayoutMode) {
        match layout {
            LayoutMode::Focus => {
                let Some(id) = self.focused.or_else(|| self.accounts.first().map(|a| a.id)) else { return };
                self.focus(id);
            }
            other if self.layout == LayoutMode::Focus => {
                self.layout_before_focus = other;
                self.leave_focus();
            }
            other => {
                self.layout = other;
                self.layout_before_focus = other;
            }
        }
    }

    pub fn focus(&mut self, id: Uuid) {
        if self.layout != LayoutMode::Focus {
            self.layout_before_focus = self.layout;
            self.zoom_outside_focus = Some(self.zoom);
            self.zoom = 1.0;
        }
        self.layout = LayoutMode::Focus;
        self.focused = Some(id);
    }

    pub fn toggle_focus(&mut self, id: Uuid) {
        if self.layout == LayoutMode::Focus && self.focused == Some(id) {
            self.leave_focus();
        } else {
            self.focus(id);
        }
    }

    fn leave_focus(&mut self) {
        self.layout = match self.layout_before_focus {
            LayoutMode::Focus => LayoutMode::Grid,
            other => other,
        };
        if let Some(zoom) = self.zoom_outside_focus.take() {
            self.zoom = zoom;
        }
    }

    pub fn step_zoom(&mut self, direction: i32) {
        let zoom = self.zoom;
        self.zoom = if direction < 0 {
            ZOOM_LEVELS.iter().rev().copied().find(|z| *z < zoom - 0.001).unwrap_or(ZOOM_LEVELS[0])
        } else {
            ZOOM_LEVELS.iter().copied().find(|z| *z > zoom + 0.001).unwrap_or(ZOOM_LEVELS[ZOOM_LEVELS.len() - 1])
        };
    }
}

pub fn closest_zoom(zoom: f64) -> f64 {
    ZOOM_LEVELS
        .iter()
        .copied()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_accounts() -> DeckConfig {
        let mut config = DeckConfig::default();
        let a = config.new_account();
        config.accounts.push(a);
        let b = config.new_account();
        config.accounts.push(b);
        config
    }

    #[test]
    fn focus_resets_zoom_and_leaving_restores_it() {
        let mut config = two_accounts();
        config.set_layout(LayoutMode::Columns);
        config.zoom = 0.5;
        let id = config.accounts[1].id;
        config.toggle_focus(id);
        assert_eq!((config.layout, config.zoom), (LayoutMode::Focus, 1.0));
        config.toggle_focus(id);
        assert_eq!((config.layout, config.zoom), (LayoutMode::Columns, 0.5));
    }

    #[test]
    fn removing_the_focused_account_leaves_focus() {
        let mut config = two_accounts();
        let id = config.accounts[0].id;
        config.focus(id);
        config.accounts.remove(0);
        config.fix_focus();
        assert_eq!(config.layout, LayoutMode::Grid);
        assert_eq!(config.focused, None);
    }

    #[test]
    fn zoom_steps_through_levels_and_stops_at_the_ends() {
        let mut config = DeckConfig::default();
        config.step_zoom(1);
        assert_eq!(config.zoom, 1.1);
        config.zoom = 0.5;
        config.step_zoom(-1);
        assert_eq!(config.zoom, 0.5);
    }

    #[test]
    fn old_config_without_new_fields_still_loads() {
        let config: DeckConfig = serde_json::from_str(
            r#"{"accounts":[{"id":"6fda33fe-b867-4342-9331-6e953f252a08","name":"Main"}],"stacked":true}"#,
        )
        .unwrap();
        assert_eq!(config.accounts[0].name, "Main");
        assert!(!config.accounts[0].off);
        assert_eq!(config.zoom, 1.0);
        assert!(config.reduce_motion);
        assert_eq!(config.accounts[0].restock.balls.id, ULTRA_BALL);
    }

    #[test]
    fn channels_accept_links_and_at_names() {
        assert_eq!(
            parse_channels("https://kick.com/Yetz/\n@rodil, rodil  twitch.tv/matta"),
            vec!["yetz", "rodil", "matta"]
        );
    }

    #[test]
    fn lives_default_to_the_first_account_until_chosen() {
        let mut config = two_accounts();
        let (a, b) = (config.accounts[0].id, config.accounts[1].id);
        assert_eq!(config.lives_accounts(), vec![a]);
        config.set_lives_account(b, true);
        config.set_lives_account(a, false);
        assert_eq!(config.lives_accounts(), vec![b]);
    }

    #[test]
    fn slot_choices_default_to_auto() {
        let mut limit = LiveLimit::default();
        limit.set_choice(1, SlotChoice::Channel("rodil".into()));
        assert_eq!(limit.choices(3), vec![SlotChoice::Auto, SlotChoice::Channel("rodil".into()), SlotChoice::Auto]);
        let json = serde_json::to_string(&limit.chosen).unwrap();
        assert_eq!(json, r#"[{"kind":"auto"},{"kind":"channel","channel":"rodil"}]"#);
    }

    #[test]
    fn restock_serializes_like_recompra_js_expects() {
        let json = serde_json::to_value(RestockConfig::default()).unwrap();
        assert_eq!(json["potionFirst"], true);
        assert_eq!(json["goldFloor"], 1_000_000);
        assert_eq!(json["potions"]["id"], ULTIMATE_POTION);
        let wild: RestockConfig = serde_json::from_str(r#"{"balls":{"keep":-5,"below":99999999999}}"#).unwrap();
        let fixed = wild.normalized();
        assert_eq!((fixed.balls.keep, fixed.balls.below), (0, MAX_STOCK));
    }
}
