use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub bot: BotConfig,
    pub permissions: PermissionsConfig,
    pub tts: TtsConfig,
    #[serde(default)]
    pub abbreviations: HashMap<String, String>,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub rank: RankConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_ai_model")]
    pub model: String,
    #[serde(default = "default_ai_provider")]
    pub provider: String,
    #[serde(default)]
    pub groq_api_key: String,
    #[serde(default = "default_groq_model")]
    pub groq_model: String,
    #[serde(default)]
    pub custom_answers: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub google_search: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProvider {
    Gemini,
    Groq,
}

impl AiConfig {
    pub fn resolve(&self) -> (AiProvider, &str, &str) {
        if self.provider.to_lowercase() == "groq" {
            (AiProvider::Groq, &self.groq_api_key, &self.groq_model)
        } else {
            (AiProvider::Gemini, &self.api_key, &self.model)
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_ai_provider() -> String {
    "gemini".to_string()
}

fn default_groq_model() -> String {
    "llama-3.3-70b-versatile".to_string()
}

fn default_ai_model() -> String {
    "gemini-flash-latest".to_string()
}

#[derive(Debug, Deserialize)]
pub struct BotConfig {
    pub token: String,
    pub application_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct PermissionsConfig {
    pub owner_id: u64,
    #[serde(default)]
    pub allowed_users: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TtsConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    pub msedge: Vec<HashMap<String, String>>,
    #[serde(default)]
    pub supertonic: Option<Vec<HashMap<String, serde_yaml::Value>>>,
    #[serde(default)]
    pub openai: Option<Vec<HashMap<String, serde_yaml::Value>>>,
    #[serde(default)]
    pub vieneu: Option<Vec<HashMap<String, serde_yaml::Value>>>,
    #[serde(default = "default_gender")]
    pub default_gender: String,
    #[serde(default)]
    pub rate: i32,
    #[serde(default)]
    pub pitch: i32,
    #[serde(default)]
    pub max_chars: usize,
    #[serde(default = "default_audio_format")]
    pub audio_format: String,
}

impl TtsConfig {
    pub fn get_msedge_voice(&self, key: &str) -> String {
        for map in &self.msedge {
            if let Some(val) = map.get(key) {
                return val.clone();
            }
        }
        match key {
            "female" => "vi-VN-HoaiMyNeural".to_string(),
            "male" => "vi-VN-NamMinhNeural".to_string(),
            "en_female" => "en-US-JennyNeural".to_string(),
            "en_male" => "en-US-GuyNeural".to_string(),
            _ => String::new(),
        }
    }

    pub fn get_active_voice(&self, is_female: bool) -> String {
        match self.provider.as_str() {
            "supertonic" => {
                if let Some(ref list) = self.supertonic {
                    let key = if is_female { "female" } else { "male" };
                    for map in list {
                        if let Some(val) = map.get(key) {
                            if let Some(s) = val.as_str() {
                                return s.to_string();
                            }
                        }
                    }
                }
                if is_female { "F2".to_string() } else { "M1".to_string() }
            }
            "openai" => {
                if let Some(ref list) = self.openai {
                    let key = if is_female { "female" } else { "male" };
                    for map in list {
                        if let Some(val) = map.get(key) {
                            if let Some(s) = val.as_str() {
                                return s.to_string();
                            }
                        }
                    }
                }
                if is_female { "nova".to_string() } else { "onyx".to_string() }
            }
            "vieneu" => {
                if let Some(ref list) = self.vieneu {
                    let key = if is_female { "female" } else { "male" };
                    for map in list {
                        if let Some(val) = map.get(key) {
                            if let Some(s) = val.as_str() {
                                return s.to_string();
                            }
                        }
                    }
                }
                if is_female { "truc_ly".to_string() } else { "nam_phuong".to_string() }
            }
            _ => {
                let key = if is_female { "female" } else { "male" };
                self.get_msedge_voice(key)
            }
        }
    }

    pub fn get_supertonic_config(&self) -> Option<SupertonicConfig> {
        let list = self.supertonic.as_ref()?;
        let mut server_url = None;
        let mut voice_female = None;
        let mut voice_male = None;
        let mut lang = None;
        let mut steps = None;
        let mut speed = None;

        for map in list {
            if let Some(val) = map.get("server_url") {
                server_url = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("female") {
                voice_female = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("male") {
                voice_male = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("lang") {
                lang = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("steps") {
                steps = val.as_u64().map(|v| v as u8);
            }
            if let Some(val) = map.get("speed") {
                speed = val.as_f64().map(|v| v as f32);
            }
        }

        Some(SupertonicConfig {
            server_url: server_url?,
            voice_female: voice_female?,
            voice_male: voice_male?,
            lang: lang.unwrap_or_else(|| "vi".to_string()),
            steps,
            speed,
        })
    }

    pub fn get_openai_config(&self) -> Option<OpenAiTtsConfig> {
        let list = self.openai.as_ref()?;
        let mut api_url = None;
        let mut api_key = None;
        let mut voice_female = None;
        let mut voice_male = None;
        let mut model = None;

        for map in list {
            if let Some(val) = map.get("api_url") {
                api_url = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("api_key") {
                api_key = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("female") {
                voice_female = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("male") {
                voice_male = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("model") {
                model = val.as_str().map(String::from);
            }
        }

        Some(OpenAiTtsConfig {
            api_url: api_url?,
            api_key: api_key.unwrap_or_default(),
            voice_female: voice_female?,
            voice_male: voice_male?,
            model: model.unwrap_or_else(|| "tts-1".to_string()),
        })
    }

    pub fn get_vieneu_config(&self) -> Option<VieneuConfig> {
        let list = self.vieneu.as_ref()?;
        let mut server_url = None;
        let mut voice_female = None;
        let mut voice_male = None;
        let mut speed = None;

        for map in list {
            if let Some(val) = map.get("server_url") {
                server_url = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("female") {
                voice_female = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("male") {
                voice_male = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("speed") {
                speed = val.as_f64().map(|v| v as f32);
            }
        }

        Some(VieneuConfig {
            server_url: server_url?,
            voice_female: voice_female?,
            voice_male: voice_male?,
            speed,
        })
    }
}

fn default_gender() -> String {
    "female".to_string()
}

fn default_provider() -> String {
    "msedge".to_string()
}

fn default_audio_format() -> String {
    "audio-24khz-48kbitrate-mono-mp3".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct VieneuConfig {
    pub server_url: String,
    pub voice_female: String,
    pub voice_male: String,
    pub speed: Option<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SupertonicConfig {
    pub server_url: String,
    pub voice_female: String,
    pub voice_male: String,
    pub lang: String,
    pub steps: Option<u8>,
    pub speed: Option<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OpenAiTtsConfig {
    pub api_url: String,
    pub api_key: String,
    pub voice_female: String,
    pub voice_male: String,
    pub model: String,
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config file '{}': {}", path, e))?;
        let config: Config = serde_yaml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse config: {}", e))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.bot.token.is_empty() || self.bot.token == "YOUR_DISCORD_BOT_TOKEN" {
            anyhow::bail!("bot.token must be set to a valid Discord bot token");
        }
        if self.bot.application_id == 0 || self.bot.application_id == 1234567890123456789 {
            anyhow::bail!("bot.application_id must be set to your bot's application ID");
        }
        if self.permissions.owner_id == 0 {
            anyhow::bail!("permissions.owner_id must be set");
        }
        if self.rank.enabled && self.rank.ranks.is_empty() {
            anyhow::bail!("rank.ranks cannot be empty if rank system is enabled");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RankConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub guild_id: u64,
    #[serde(default)]
    pub target_role_id: u64,
    #[serde(default)]
    pub leaderboard_channel_id: u64,
    #[serde(default = "default_stars_per_rank")]
    pub stars_per_rank: u8,
    #[serde(default = "default_ranks")]
    pub ranks: Vec<String>,
}

fn default_stars_per_rank() -> u8 {
    3
}

fn default_ranks() -> Vec<String> {
    vec![]
}

impl RankConfig {
    pub fn max_level(&self) -> u8 {
        (self.ranks.len() as u8) * self.stars_per_rank
    }

    pub fn level_to_display(&self, level: u8) -> anyhow::Result<(&str, u8)> {
        if level == 0 || level > self.max_level() {
            anyhow::bail!("level {} is out of bounds", level);
        }
        let rank_idx = (level - 1) / self.stars_per_rank;
        let stars = (level - 1) % self.stars_per_rank + 1;
        Ok((&self.ranks[rank_idx as usize], stars))
    }
}
