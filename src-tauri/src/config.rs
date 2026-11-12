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
        Account { id: Uuid::new_v4(), name, off: false, muted: false }
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
    }
}
