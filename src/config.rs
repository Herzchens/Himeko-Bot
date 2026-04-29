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
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_ai_model")]
    pub model: String,
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

fn default_voice_en_female() -> String {
    "en-US-JennyNeural".to_string()
}

fn default_voice_en_male() -> String {
    "en-US-GuyNeural".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct TtsConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    pub voice_female: String,
    pub voice_male: String,
    #[serde(default = "default_voice_en_female")]
    pub voice_en_female: String,
    #[serde(default = "default_voice_en_male")]
    pub voice_en_male: String,
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

fn default_gender() -> String {
    "female".to_string()
}

fn default_provider() -> String {
    "msedge".to_string()
}



fn default_audio_format() -> String {
    "audio-24khz-48kbitrate-mono-mp3".to_string()
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
        Ok(())
    }
}
