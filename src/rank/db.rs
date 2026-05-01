use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct RankDatabase {
    pub initialized: bool,
    #[serde(default)]
    pub settings: RankSettings,
    #[serde(default)]
    pub users: HashMap<String, RankUserData>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RankSettings {
    pub autorename: bool,
}

impl Default for RankSettings {
    fn default() -> Self {
        Self { autorename: true }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct RankUserData {
    pub level: u8,
    pub original_name: String,
}

impl RankDatabase {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        if !std::path::Path::new(path).exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let db = serde_yaml::from_str(&raw)?;
        Ok(db)
    }

    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        let content = serde_yaml::to_string(self)?;
        let tmp_path = format!("{}.tmp", path);
        std::fs::write(&tmp_path, &content)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}
