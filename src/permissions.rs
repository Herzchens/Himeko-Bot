use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UserLevel {
    Unknown = 0,
    Allowed = 1,
    Owner = 2,
}

impl UserLevel {
    pub fn of(user_id: u64, config: &Config) -> Self {
        if user_id == config.permissions.owner_id {
            Self::Owner
        } else if config.permissions.allowed_users.contains(&user_id) {
            Self::Allowed
        } else {
            Self::Unknown
        }
    }

    pub fn can_use_tts(self) -> bool {
        self >= Self::Allowed
    }

    pub fn can_use_ai(self) -> bool {
        self >= Self::Allowed
    }

    pub fn can_echo(self) -> bool {
        self == Self::Owner
    }

    pub fn can_join(self) -> bool {
        self >= Self::Allowed
    }

    pub fn can_preempt(self) -> bool {
        self == Self::Owner
    }

    pub fn can_control_session(self, requester_id: u64, session_owner_id: u64) -> bool {
        self.can_preempt() || (self.can_join() && requester_id == session_owner_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BotConfig, PermissionsConfig, TtsConfig};
    use std::collections::HashMap;

    fn test_config() -> Config {
        Config {
            bot: BotConfig {
                token: "test".to_string(),
                application_id: 1,
            },
            permissions: PermissionsConfig {
                owner_id: 100,
                allowed_users: vec![200, 300],
            },
            tts: TtsConfig {
                provider: "msedge".to_string(),
                msedge: vec![
                    HashMap::from([("female".to_string(), "f".to_string())]),
                    HashMap::from([("male".to_string(), "m".to_string())]),
                    HashMap::from([("en_female".to_string(), "enf".to_string())]),
                    HashMap::from([("en_male".to_string(), "enm".to_string())]),
                ],
                supertonic: None,
                openai: None,
                vieneu: None,
                default_gender: "female".to_string(),
                rate: 0,
                pitch: 0,
                max_chars: 0,
                audio_format: "audio-24khz-48kbitrate-mono-mp3".to_string(),
            },
            abbreviations: HashMap::new(),
            ai: crate::config::AiConfig::default(),
            rank: crate::config::RankConfig::default(),
            voice_status: crate::config::VoiceStatusConfig::default(),
            console_chat: crate::config::ConsoleChatConfig::default(),
            logging: crate::config::LoggingConfig {
                webhook_url: "".to_string(),
                control_channel_id: 0,
            },
        }
    }

    #[test]
    fn owner_has_full_access() {
        let level = UserLevel::of(100, &test_config());
        assert_eq!(level, UserLevel::Owner);
        assert!(level.can_use_tts());
        assert!(level.can_use_ai());
        assert!(level.can_echo());
        assert!(level.can_join());
        assert!(level.can_preempt());
        assert!(level.can_control_session(100, 200));
    }

    #[test]
    fn allowed_user_has_limited_access_and_controls_only_own_session() {
        let level = UserLevel::of(200, &test_config());
        assert_eq!(level, UserLevel::Allowed);
        assert!(level.can_use_tts());
        assert!(level.can_use_ai());
        assert!(!level.can_echo());
        assert!(level.can_join());
        assert!(!level.can_preempt());
        assert!(level.can_control_session(200, 200));
        assert!(!level.can_control_session(200, 300));
    }

    #[test]
    fn unknown_user_has_no_access() {
        let level = UserLevel::of(999, &test_config());
        assert_eq!(level, UserLevel::Unknown);
        assert!(!level.can_use_tts());
        assert!(!level.can_use_ai());
        assert!(!level.can_echo());
        assert!(!level.can_join());
        assert!(!level.can_preempt());
        assert!(!level.can_control_session(999, 999));
    }
}
