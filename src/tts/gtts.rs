use super::TtsEngine;
use reqwest::Client;

pub struct GttsEngine {
    client: Client,
}

impl GttsEngine {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for GttsEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let tl = if voice.starts_with("en-") || voice == "en" { "en" } else { "vi" };
        let url = format!(
            "https://translate.google.com/translate_tts?ie=UTF-8&tl={}&client=tw-ob&q={}",
            tl,
            urlencoding::encode(text)
        );

        let mut attempts = 0;
        let max_attempts = 3;

        loop {
            attempts += 1;
            match self.client.get(&url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.bytes().await {
                            Ok(bytes) => {
                                tracing::debug!(
                                    text_len = text.len(),
                                    audio_len = bytes.len(),
                                    "gTTS synthesis complete"
                                );
                                return Ok(bytes.to_vec());
                            }
                            Err(e) => {
                                if attempts >= max_attempts {
                                    return Err(anyhow::anyhow!("gTTS failed to read bytes: {}", e));
                                }
                                tracing::warn!("gTTS read failed (attempt {}/{}): {}", attempts, max_attempts, e);
                            }
                        }
                    } else {
                        if attempts >= max_attempts {
                            return Err(anyhow::anyhow!("gTTS returned error status: {}", response.status()));
                        }
                        tracing::warn!("gTTS status error (attempt {}/{}): {}", attempts, max_attempts, response.status());
                    }
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        return Err(anyhow::anyhow!("gTTS request failed after {} attempts: {}", attempts, e));
                    }
                    tracing::warn!("gTTS request failed (attempt {}/{}): {}. Retrying in 500ms...", attempts, max_attempts, e);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
