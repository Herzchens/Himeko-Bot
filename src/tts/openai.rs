use super::TtsEngine;
use crate::config::OpenAiTtsConfig;
use reqwest::Client;

pub struct OpenAiEngine {
    client: Client,
    config: OpenAiTtsConfig,
}

impl OpenAiEngine {
    pub fn new(config: OpenAiTtsConfig) -> Self {
        Self {
            client: Client::new(),
            config,
        }
    }

    fn resolve_voice<'a>(&'a self, voice: &'a str) -> &'a str {
        if voice.contains('-') {
            let lower = voice.to_lowercase();
            let is_male = lower.contains("nam")
                || lower.contains("guy")
                || lower.contains("male")
                    && !lower.contains("female");
            if is_male {
                &self.config.voice_male
            } else {
                &self.config.voice_female
            }
        } else {
            voice
        }
    }
}

#[async_trait::async_trait]
impl TtsEngine for OpenAiEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let mut url = self.config.api_url.clone();
        if !url.ends_with("/audio/speech") {
            if !url.ends_with('/') {
                url.push('/');
            }
            url.push_str("audio/speech");
        }

        let voice_name = self.resolve_voice(voice);
        let payload = serde_json::json!({
            "model": self.config.model,
            "input": text,
            "voice": voice_name,
        });

        let mut req = self.client.post(&url).json(&payload);
        if !self.config.api_key.is_empty() {
            req = req.bearer_auth(&self.config.api_key);
        }

        let mut attempts = 0;
        let max_attempts = 3;

        loop {
            attempts += 1;
            match req.try_clone().ok_or_else(|| anyhow::anyhow!("failed to clone request"))?.send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.bytes().await {
                            Ok(bytes) => {
                                tracing::debug!(
                                    text_len = text.len(),
                                    audio_len = bytes.len(),
                                    "OpenAI TTS synthesis complete"
                                );
                                return Ok(bytes.to_vec());
                            }
                            Err(e) => {
                                if attempts >= max_attempts {
                                    anyhow::bail!("OpenAI TTS failed to read bytes: {e}");
                                }
                                tracing::warn!("OpenAI TTS read failed (attempt {attempts}/{max_attempts}): {e}");
                            }
                        }
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        if attempts >= max_attempts {
                            anyhow::bail!("OpenAI TTS returned {status}: {body}");
                        }
                        tracing::warn!("OpenAI TTS status error (attempt {attempts}/{max_attempts}): {status}");
                    }
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        anyhow::bail!("OpenAI TTS request failed after {attempts} attempts: {e}");
                    }
                    tracing::warn!("OpenAI TTS request failed (attempt {attempts}/{max_attempts}): {e}. Retrying in 500ms...");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
