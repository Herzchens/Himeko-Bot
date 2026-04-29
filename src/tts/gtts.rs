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
    async fn synthesize(&self, text: &str, _voice: &str) -> anyhow::Result<Vec<u8>> {
        // Ghi chú: gTTS API không hỗ trợ chọn giọng nam/nữ, chỉ hỗ trợ ngôn ngữ.
        // Tham số _voice sẽ bị bỏ qua.
        
        let url = format!(
            "https://translate.google.com/translate_tts?ie=UTF-8&tl=vi&client=tw-ob&q={}",
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
