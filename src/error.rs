#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("TTS synthesis failed: {0}")]
    Tts(String),

    #[error("voice channel error: {0}")]
    Voice(String),

    #[error("permission denied: {0}")]
    Permission(String),
}
