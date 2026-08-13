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
    #[serde(default)]
    pub voice_status: VoiceStatusConfig,
    #[serde(default)]
    pub console_chat: ConsoleChatConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
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
                if is_female {
                    "F2".to_string()
                } else {
                    "M1".to_string()
                }
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
                if is_female {
                    "nova".to_string()
                } else {
                    "onyx".to_string()
                }
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
                if is_female {
                    "truc_ly".to_string()
                } else {
                    "nam_phuong".to_string()
                }
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
        let mut autostart = true;

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
            if let Some(val) = map.get("autostart") {
                autostart = val.as_bool().unwrap_or(true);
            }
        }

        Some(SupertonicConfig {
            server_url: server_url?,
            voice_female: voice_female?,
            voice_male: voice_male?,
            lang: lang.unwrap_or_else(|| "vi".to_string()),
            steps,
            speed,
            autostart,
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
        let mut autostart = true;
        let mut mode = None;
        let mut temperature = None;
        let mut device = None;
        let mut pitch = None;

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
            if let Some(val) = map.get("autostart") {
                autostart = val.as_bool().unwrap_or(true);
            }
            if let Some(val) = map.get("mode") {
                mode = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("temperature") {
                temperature = val.as_f64().map(|v| v as f32);
            }
            if let Some(val) = map.get("device") {
                device = val.as_str().map(String::from);
            }
            if let Some(val) = map.get("pitch") {
                pitch = val.as_i64().map(|v| v as i32);
            }
        }

        Some(VieneuConfig {
            server_url: server_url?,
            voice_female: voice_female?,
            voice_male: voice_male?,
            speed,
            autostart,
            mode,
            temperature,
            device,
            pitch,
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
    #[serde(default = "default_true")]
    pub autostart: bool,
    pub mode: Option<String>,
    pub temperature: Option<f32>,
    pub device: Option<String>,
    pub pitch: Option<i32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SupertonicConfig {
    pub server_url: String,
    pub voice_female: String,
    pub voice_male: String,
    pub lang: String,
    pub steps: Option<u8>,
    pub speed: Option<f32>,
    #[serde(default = "default_true")]
    pub autostart: bool,
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
            .map_err(|e| anyhow::anyhow!("failed to read config file '{path}': {e}"))?;
        let config: Config = serde_yaml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse config: {e}"))?;
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
        if !matches!(
            self.tts.provider.as_str(),
            "msedge" | "gtts" | "supertonic" | "openai" | "vieneu"
        ) {
            anyhow::bail!("unsupported tts.provider: {}", self.tts.provider);
        }
        if !matches!(self.tts.default_gender.as_str(), "female" | "male") {
            anyhow::bail!("tts.default_gender must be either 'female' or 'male'");
        }
        if self.tts.provider == "gtts" && (self.tts.rate != 0 || self.tts.pitch != 0) {
            anyhow::bail!("gTTS does not support tts.rate or tts.pitch; both must be 0");
        }
        match self.tts.provider.as_str() {
            "supertonic" if self.tts.get_supertonic_config().is_none() => {
                anyhow::bail!("tts.supertonic must be configured when provider is 'supertonic'");
            }
            "openai" if self.tts.get_openai_config().is_none() => {
                anyhow::bail!("tts.openai must be configured when provider is 'openai'");
            }
            "vieneu" if self.tts.get_vieneu_config().is_none() => {
                anyhow::bail!("tts.vieneu must be configured when provider is 'vieneu'");
            }
            _ => {}
        }
        if self.ai.enabled && !matches!(self.ai.provider.as_str(), "gemini" | "groq") {
            anyhow::bail!("unsupported ai.provider: {}", self.ai.provider);
        }
        self.rank.validate()?;
        if self.voice_status.enabled {
            if self.voice_status.channel_id == 0 {
                anyhow::bail!("voice_status.channel_id must be set when voice status is enabled");
            }
            if self.voice_status.interval_secs == 0 {
                anyhow::bail!("voice_status.interval_secs must be greater than zero");
            }
            if self.voice_status.steps.is_empty() {
                anyhow::bail!("voice_status.steps cannot be empty when voice status is enabled");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub guilds: HashMap<String, GuildRankConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuildRankConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub target_role_id: u64,
    #[serde(default)]
    pub leaderboard_channel_id: u64,
    #[serde(default = "default_stars_per_rank")]
    pub stars_per_rank: u8,
    #[serde(default = "default_ranks")]
    pub ranks: Vec<String>,
}

impl Default for GuildRankConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_role_id: 0,
            leaderboard_channel_id: 0,
            stars_per_rank: default_stars_per_rank(),
            ranks: Vec::new(),
        }
    }
}

fn default_stars_per_rank() -> u8 {
    3
}

fn default_ranks() -> Vec<String> {
    Vec::new()
}

impl GuildRankConfig {
    pub fn max_level(&self) -> u8 {
        self.ranks
            .len()
            .checked_mul(self.stars_per_rank as usize)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(u8::MAX)
    }

    pub fn level_to_display(&self, level: u8) -> anyhow::Result<(&str, u8)> {
        if self.stars_per_rank == 0 || level == 0 || level > self.max_level() {
            anyhow::bail!("level {level} is out of bounds");
        }
        let rank_index = (level - 1) / self.stars_per_rank;
        let stars = (level - 1) % self.stars_per_rank + 1;
        let rank = self
            .ranks
            .get(rank_index as usize)
            .ok_or_else(|| anyhow::anyhow!("rank index {rank_index} is out of bounds"))?;
        Ok((rank, stars))
    }

    fn validate(&self, guild_id: u64) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if guild_id == 0 {
            anyhow::bail!("rank guild id must be non-zero");
        }
        if self.target_role_id == 0 {
            anyhow::bail!("rank target_role_id must be set for guild {guild_id}");
        }
        if self.target_role_id == guild_id {
            anyhow::bail!("rank target_role_id for guild {guild_id} cannot be the @everyone role");
        }
        if self.leaderboard_channel_id == 0 {
            anyhow::bail!("rank leaderboard_channel_id must be set for guild {guild_id}");
        }
        if self.stars_per_rank == 0 {
            anyhow::bail!("rank stars_per_rank must be greater than zero for guild {guild_id}");
        }
        if self.ranks.is_empty() {
            anyhow::bail!("rank ranks cannot be empty for guild {guild_id}");
        }
        let total_levels = self
            .ranks
            .len()
            .checked_mul(self.stars_per_rank as usize)
            .ok_or_else(|| anyhow::anyhow!("rank level count overflow for guild {guild_id}"))?;
        if total_levels > u8::MAX as usize {
            anyhow::bail!(
                "rank configuration for guild {guild_id} supports at most {} total levels",
                u8::MAX
            );
        }

        let mut canonical_ranks = std::collections::HashSet::new();
        for rank in &self.ranks {
            if rank.is_empty() || rank.trim() != rank {
                anyhow::bail!(
                    "rank names for guild {guild_id} must be non-empty and have no surrounding whitespace"
                );
            }
            let canonical = rank.to_uppercase();
            if !canonical_ranks.insert(canonical) {
                anyhow::bail!("rank names for guild {guild_id} must be unique ignoring case");
            }
            let longest_prefix = format!("{rank} {} SAO", self.stars_per_rank);
            if longest_prefix.chars().count() > 32 {
                anyhow::bail!(
                    "rank nickname prefix for guild {guild_id} exceeds Discord's 32-character nickname limit"
                );
            }
        }
        Ok(())
    }
}

impl RankConfig {
    fn legacy_guild_config(&self) -> GuildRankConfig {
        GuildRankConfig {
            enabled: self.enabled,
            target_role_id: self.target_role_id,
            leaderboard_channel_id: self.leaderboard_channel_id,
            stars_per_rank: self.stars_per_rank,
            ranks: self.ranks.clone(),
        }
    }

    pub fn guild_config(&self, guild_id: u64) -> Option<GuildRankConfig> {
        if !self.enabled || guild_id == 0 {
            return None;
        }
        if let Some(config) = self.guilds.get(&guild_id.to_string()) {
            return config.enabled.then(|| config.clone());
        }
        (self.guild_id == guild_id).then(|| self.legacy_guild_config())
    }

    pub fn configured_guilds(&self) -> anyhow::Result<Vec<(u64, GuildRankConfig)>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let mut configured = Vec::new();
        for (raw_id, config) in &self.guilds {
            let guild_id = raw_id
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("invalid rank guild id: {raw_id}"))?;
            let canonical_id = guild_id.to_string();
            if guild_id == 0 || raw_id != &canonical_id {
                anyhow::bail!(
                    "rank guild id key must use canonical non-zero decimal form: {raw_id}"
                );
            }
            if config.enabled {
                configured.push((guild_id, config.clone()));
            }
        }
        if self.guild_id != 0 && !self.guilds.contains_key(&self.guild_id.to_string()) {
            configured.push((self.guild_id, self.legacy_guild_config()));
        }
        configured.sort_by_key(|(guild_id, _)| *guild_id);
        configured.dedup_by_key(|(guild_id, _)| *guild_id);
        Ok(configured)
    }

    pub fn legacy_guild_id(&self) -> u64 {
        self.guild_id
    }

    pub fn max_level(&self) -> u8 {
        self.legacy_guild_config().max_level()
    }

    pub fn level_to_display(&self, level: u8) -> anyhow::Result<(&str, u8)> {
        if self.stars_per_rank == 0 || level == 0 || level > self.max_level() {
            anyhow::bail!("level {level} is out of bounds");
        }
        let rank_index = (level - 1) / self.stars_per_rank;
        let stars = (level - 1) % self.stars_per_rank + 1;
        let rank = self
            .ranks
            .get(rank_index as usize)
            .ok_or_else(|| anyhow::anyhow!("rank index {rank_index} is out of bounds"))?;
        Ok((rank, stars))
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let configured = self.configured_guilds()?;
        if configured.is_empty() {
            anyhow::bail!("rank must configure at least one guild when enabled");
        }
        for (guild_id, config) in configured {
            config.validate(guild_id)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VoiceStatusConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub channel_id: u64,
    #[serde(default = "default_voice_status_interval")]
    pub interval_secs: u64,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub random: bool,
}

fn default_voice_status_interval() -> u64 {
    300
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConsoleChatConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub default_channel_id: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LoggingConfig {
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub control_channel_id: u64,
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    fn valid_config() -> Config {
        serde_yaml::from_str(
            r#"
bot:
  token: test-token
  application_id: 1
permissions:
  owner_id: 1
tts:
  provider: msedge
  msedge: []
"#,
        )
        .expect("test config must parse")
    }

    #[test]
    fn valid_minimal_config_passes_validation() {
        valid_config().validate().expect("valid config must pass");
    }

    #[test]
    fn rejects_unknown_tts_provider() {
        let mut config = valid_config();
        config.tts.provider = "msedeg".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_invalid_default_gender() {
        let mut config = valid_config();
        config.tts.default_gender = "other".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_unsupported_gtts_rate_and_pitch() {
        let mut config = valid_config();
        config.tts.provider = "gtts".to_string();
        config.tts.rate = 1;
        assert!(config.validate().is_err());

        config.tts.rate = 0;
        config.tts.pitch = 1;
        assert!(config.validate().is_err());

        config.tts.pitch = 0;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_unknown_enabled_ai_provider() {
        let mut config = valid_config();
        config.ai.enabled = true;
        config.ai.provider = "unknown".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_rank_ids_and_zero_stars() {
        let mut config = valid_config();
        config.rank.enabled = true;
        config.rank.ranks = vec!["RANK".to_string()];
        assert!(config.validate().is_err());

        config.rank.guild_id = 1;
        config.rank.target_role_id = 2;
        config.rank.leaderboard_channel_id = 3;
        config.rank.stars_per_rank = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_rank_level_count_above_u8_capacity() {
        let mut config = valid_config();
        config.rank.enabled = true;
        config.rank.guild_id = 1;
        config.rank.target_role_id = 2;
        config.rank.leaderboard_channel_id = 3;
        config.rank.stars_per_rank = 3;
        config.rank.ranks = (0..86).map(|index| format!("RANK {index}")).collect();
        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_rank_math_never_panics_or_wraps() {
        let rank = RankConfig {
            enabled: true,
            guild_id: 1,
            target_role_id: 2,
            leaderboard_channel_id: 3,
            stars_per_rank: 3,
            ranks: (0..100).map(|index| format!("RANK {index}")).collect(),
            guilds: HashMap::new(),
        };
        assert_eq!(rank.max_level(), u8::MAX);

        let zero_stars = RankConfig {
            stars_per_rank: 0,
            ranks: vec!["RANK".to_string()],
            ..RankConfig::default()
        };
        assert!(zero_stars.level_to_display(1).is_err());
    }

    #[test]
    fn rejects_invalid_enabled_voice_status() {
        let mut config = valid_config();
        config.voice_status.enabled = true;
        config.voice_status.steps = vec!["ready".to_string()];
        assert!(config.validate().is_err());

        config.voice_status.channel_id = 1;
        config.voice_status.interval_secs = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn multi_guild_rank_config_is_isolated_and_explicit_map_overrides_legacy() {
        let mut config = valid_config();
        config.rank.enabled = true;
        config.rank.guild_id = 10;
        config.rank.target_role_id = 100;
        config.rank.leaderboard_channel_id = 101;
        config.rank.ranks = vec!["LEGACY".to_string()];
        config.rank.guilds.insert(
            "10".to_string(),
            GuildRankConfig {
                target_role_id: 200,
                leaderboard_channel_id: 201,
                ranks: vec!["OVERRIDE".to_string()],
                ..GuildRankConfig::default()
            },
        );
        config.rank.guilds.insert(
            "20".to_string(),
            GuildRankConfig {
                target_role_id: 300,
                leaderboard_channel_id: 301,
                ranks: vec!["SECOND".to_string()],
                ..GuildRankConfig::default()
            },
        );
        assert_eq!(config.rank.guild_config(10).unwrap().target_role_id, 200);
        assert_eq!(config.rank.guild_config(20).unwrap().target_role_id, 300);
        assert!(config.rank.guild_config(30).is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn noncanonical_rank_guild_key_is_rejected() {
        let mut config = valid_config();
        config.rank.enabled = true;
        config.rank.guilds.insert(
            "00123".into(),
            GuildRankConfig {
                enabled: true,
                target_role_id: 2,
                leaderboard_channel_id: 3,
                stars_per_rank: 3,
                ranks: vec!["RANK".into()],
            },
        );
        assert!(config.rank.configured_guilds().is_err());
        assert!(config.validate().is_err());
    }

    #[test]
    fn canonical_rank_guild_key_has_consistent_direct_and_enumerated_lookup() {
        let mut config = valid_config();
        config.rank.enabled = true;
        config.rank.guilds.insert(
            "123".into(),
            GuildRankConfig {
                enabled: true,
                target_role_id: 20,
                leaderboard_channel_id: 30,
                stars_per_rank: 3,
                ranks: vec!["EXPLICIT".into()],
            },
        );
        let configured = config.rank.configured_guilds().unwrap();
        let direct = config.rank.guild_config(123).unwrap();
        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].0, 123);
        assert_eq!(configured[0].1.target_role_id, direct.target_role_id);
        assert_eq!(configured[0].1.ranks, direct.ranks);
    }

    #[test]
    fn rank_config_rejects_everyone_duplicate_and_unrepresentable_names() {
        let everyone = GuildRankConfig {
            enabled: true,
            target_role_id: 123,
            leaderboard_channel_id: 3,
            stars_per_rank: 3,
            ranks: vec!["RANK".into()],
        };
        assert!(everyone.validate(123).is_err());

        let duplicate = GuildRankConfig {
            enabled: true,
            target_role_id: 2,
            leaderboard_channel_id: 3,
            stars_per_rank: 3,
            ranks: vec!["Bronze".into(), "BRONZE".into()],
        };
        assert!(duplicate.validate(123).is_err());

        let whitespace = GuildRankConfig {
            enabled: true,
            target_role_id: 2,
            leaderboard_channel_id: 3,
            stars_per_rank: 3,
            ranks: vec![" BRONZE".into()],
        };
        assert!(whitespace.validate(123).is_err());

        let too_long = GuildRankConfig {
            enabled: true,
            target_role_id: 2,
            leaderboard_channel_id: 3,
            stars_per_rank: 255,
            ranks: vec!["X".repeat(30)],
        };
        assert!(too_long.validate(123).is_err());
    }

    #[test]
    fn unknown_rank_fields_are_parse_errors() {
        assert!(
            serde_yaml::from_str::<GuildRankConfig>("enabled: true\ntarget_rol_id: 2\n").is_err()
        );
        assert!(serde_yaml::from_str::<RankConfig>("enabled: false\nguildz: {}\n").is_err());
    }

    #[test]
    fn repository_config_example_parses_and_validates_after_required_placeholders_are_replaced() {
        let mut config: Config = serde_yaml::from_str(include_str!("../config.example.yml"))
            .expect("config.example.yml must remain parseable");
        config.bot.token = "test-token".to_string();
        config.bot.application_id = 1;
        config
            .validate()
            .expect("config.example.yml must satisfy runtime validation after required placeholders are replaced");
    }

    #[test]
    fn invalid_multi_guild_rank_entry_is_rejected() {
        let mut config = valid_config();
        config.rank.enabled = true;
        config.rank.guilds.insert(
            "20".to_string(),
            GuildRankConfig {
                target_role_id: 0,
                leaderboard_channel_id: 301,
                ranks: vec!["SECOND".to_string()],
                ..GuildRankConfig::default()
            },
        );
        assert!(config.validate().is_err());
    }
}
