pub mod chunking;
pub mod engine;
pub mod gtts;
pub mod local_process;
pub mod openai;
pub mod scheduler;
pub mod supertonic;
pub mod vieneu;

#[async_trait::async_trait]
pub trait TtsEngine: Send + Sync {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>>;

    async fn synthesize_chunks(&self, text: &str, voice: &str) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(vec![self.synthesize(text, voice).await?])
    }
}
