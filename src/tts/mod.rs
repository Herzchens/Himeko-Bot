pub mod engine;

#[async_trait::async_trait]
pub trait TtsEngine: Send + Sync {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>>;
}
