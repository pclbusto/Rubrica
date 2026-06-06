use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Ruta canónica de la base de datos compartida entre el CLI y la app GTK.
/// Sigue el estándar XDG: `$XDG_DATA_HOME/rubrica/library.db`
/// (tipicamente `~/.local/share/rubrica/library.db`).
pub fn default_db_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".local").join("share"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("rubrica").join("library.db")
}

/// URL SQLite de la base de datos por defecto.
pub fn default_db_url() -> String {
    let path = default_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    format!("sqlite://{}", path.display())
}

/// Configuración exportable/importable de Rúbrica.
/// Por ahora solo contiene aliases, pero está pensado para extenderse
/// a defaults, preferencias de OPDS, etc.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct RubricaConfig {
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

impl RubricaConfig {
    /// Serializa la configuración a una string TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Deserializa desde una string TOML.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}
