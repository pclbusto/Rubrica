use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
