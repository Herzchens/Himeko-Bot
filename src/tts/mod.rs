pub mod engine;
pub mod gtts;
pub mod supertonic;
pub mod openai;
pub mod vieneu;

#[async_trait::async_trait]
pub trait TtsEngine: Send + Sync {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>>;
}
