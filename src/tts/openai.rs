use super::TtsEngine;
use crate::config::OpenAiTtsConfig;
use reqwest::Client;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OpenAiEngine {
    client: Client,
    config: OpenAiTtsConfig,
}

impl OpenAiEngine {
    pub fn new(config: OpenAiTtsConfig) -> Self {
        Self::with_timeout(config, SYNTHESIS_TIMEOUT)
    }

    fn with_timeout(config: OpenAiTtsConfig, request_timeout: Duration) -> Self {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(request_timeout)
            .build()
            .expect("valid static OpenAI-compatible TTS HTTP client configuration");
        Self { client, config }
    }

    fn resolve_voice<'a>(&'a self, voice: &'a str) -> &'a str {
        if voice.contains('-') {
            let lower = voice.to_lowercase();
            let is_male = lower.contains("nam")
                || lower.contains("guy")
                || lower.contains("male") && !lower.contains("female");
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

        let mut request = self.client.post(&url).json(&payload);
        if !self.config.api_key.is_empty() {
            request = request.bearer_auth(&self.config.api_key);
        }

        let max_attempts = 3;
        for attempt in 1..=max_attempts {
            match request
                .try_clone()
                .ok_or_else(|| anyhow::anyhow!("failed to clone OpenAI TTS request"))?
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(bytes) if !bytes.is_empty() => {
                        tracing::debug!(
                            text_chars = text.chars().count(),
                            audio_len = bytes.len(),
                            "OpenAI-compatible TTS synthesis complete"
                        );
                        return Ok(bytes.to_vec());
                    }
                    Ok(_) if attempt < max_attempts => {
                        tracing::warn!(
                            "OpenAI-compatible TTS returned empty audio (attempt {attempt}/{max_attempts})"
                        );
                    }
                    Ok(_) => anyhow::bail!("OpenAI-compatible TTS returned empty audio"),
                    Err(error) if attempt < max_attempts => {
                        tracing::warn!(
                            "OpenAI-compatible TTS read failed (attempt {attempt}/{max_attempts}): {error}"
                        );
                    }
                    Err(error) => {
                        anyhow::bail!("OpenAI-compatible TTS failed to read bytes: {error}")
                    }
                },
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    if attempt >= max_attempts {
                        anyhow::bail!("OpenAI-compatible TTS returned {status}: {body}");
                    }
                    tracing::warn!(
                        "OpenAI-compatible TTS status error (attempt {attempt}/{max_attempts}): {status}"
                    );
                }
                Err(error) if attempt < max_attempts => {
                    tracing::warn!(
                        "OpenAI-compatible TTS request failed (attempt {attempt}/{max_attempts}): {error}"
                    );
                }
                Err(error) => anyhow::bail!(
                    "OpenAI-compatible TTS request failed after {attempt} attempts: {error}"
                ),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        anyhow::bail!("OpenAI-compatible TTS exhausted synthesis attempts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use tokio::net::TcpListener;

    fn config(api_url: String) -> OpenAiTtsConfig {
        OpenAiTtsConfig {
            api_url,
            api_key: String::new(),
            voice_female: "female".to_string(),
            voice_male: "male".to_string(),
            model: "tts-test".to_string(),
        }
    }

    #[tokio::test]
    async fn request_deadline_is_enforced_against_hanging_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let _socket = socket;
                    tokio::time::sleep(Duration::from_secs(5)).await;
                });
            }
        });

        let engine = OpenAiEngine::with_timeout(
            config(format!("http://{address}/v1")),
            Duration::from_millis(50),
        );
        let started = Instant::now();
        let result = engine.synthesize("hello", "female").await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        server.abort();
    }
}
