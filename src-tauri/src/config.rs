use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};
use uuid::Uuid;

pub const MAX_ACCOUNTS: usize = 4;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Também é o identificador do `WKWebsiteDataStore`: nunca muda, nem ao reordenar.
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckConfig {
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Contas removidas cujo data store foi mantido, para "Restaurar conta removida…".
    #[serde(default)]
    pub removed: Vec<Account>,
    /// Data stores de contas removidas com "apagar dados", apagados do disco no próximo início.
    #[serde(default)]
    pub pending_delete: Vec<Uuid>,
    #[serde(default)]
    pub stacked: bool,
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
        config
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)
    }

    pub fn new_account(&self) -> Account {
        let taken = |name: &str| {
            self.accounts
                .iter()
                .chain(&self.removed)
                .any(|a| a.name == name)
        };
        let name = (1..)
            .map(|n| format!("Conta {n}"))
            .find(|name| !taken(name))
            .expect("sequência infinita");
        Account { id: Uuid::new_v4(), name }
    }
}
