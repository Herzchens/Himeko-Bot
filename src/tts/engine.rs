use super::chunking::{escape_xml, split_xml_bytes, EDGE_ESCAPED_CHUNK_BYTES};
use super::TtsEngine;
use crate::config::TtsConfig;
use msedge_tts::tts::client::tokio_runtime::connect_async;
use msedge_tts::tts::SpeechConfig;
use std::time::Duration;

const MAX_ATTEMPTS: usize = 3;
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_DELAY: Duration = Duration::from_millis(500);

pub struct MsEdgeEngine {
    config: TtsConfig,
}

impl MsEdgeEngine {
    pub fn new(config: TtsConfig) -> Self {
        Self { config }
    }

    fn build_speech_config(&self, voice: &str) -> SpeechConfig {
        SpeechConfig {
            voice_name: voice.to_string(),
            audio_format: self.config.audio_format.clone(),
            rate: self.config.rate,
            pitch: self.config.pitch,
            volume: 0,
        }
    }

    async fn synthesize_escaped_chunk(
        &self,
        escaped_text: &str,
        voice: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let speech_config = self.build_speech_config(voice);

        for attempt in 1..=MAX_ATTEMPTS {
            let result = tokio::time::timeout(ATTEMPT_TIMEOUT, async {
                let mut client = connect_async().await.map_err(anyhow::Error::from)?;
                let audio = client
                    .synthesize(escaped_text, &speech_config)
                    .await
                    .map_err(anyhow::Error::from)?;
                ensure_nonempty_audio(audio.audio_bytes)
            })
            .await;

            match result {
                Ok(Ok(bytes)) => {
                    tracing::debug!(
                        voice = %voice,
                        chars = escaped_text.chars().count(),
                        audio_len = bytes.len(),
                        attempt,
                        "TTS synthesis complete"
                    );
                    return Ok(bytes);
                }
                Ok(Err(error)) if attempt == MAX_ATTEMPTS => {
                    return Err(anyhow::anyhow!(
                        "TTS synthesis failed after {attempt} attempts: {error}"
                    ));
                }
                Err(_) if attempt == MAX_ATTEMPTS => {
                    return Err(anyhow::anyhow!(
                        "TTS synthesis timed out after {attempt} attempts"
                    ));
                }
                Ok(Err(error)) => {
                    tracing::warn!(
                        attempt,
                        max_attempts = MAX_ATTEMPTS,
                        error = %error,
                        "TTS synthesis failed; retrying"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        attempt,
                        max_attempts = MAX_ATTEMPTS,
                        timeout_secs = ATTEMPT_TIMEOUT.as_secs(),
                        "TTS synthesis timed out; retrying"
                    );
                }
            }

            tokio::time::sleep(RETRY_DELAY).await;
        }

        Err(anyhow::anyhow!("TTS synthesis exhausted retries"))
    }
}

fn ensure_nonempty_audio(bytes: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    if bytes.is_empty() {
        anyhow::bail!("Edge TTS returned empty audio");
    }
    Ok(bytes)
}

#[async_trait::async_trait]
impl TtsEngine for MsEdgeEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        self.synthesize_escaped_chunk(&escape_xml(text), voice)
            .await
    }

    async fn synthesize_chunks(&self, text: &str, voice: &str) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut audio_chunks = Vec::new();
        for chunk in split_xml_bytes(text, EDGE_ESCAPED_CHUNK_BYTES) {
            let escaped = escape_xml(&chunk);
            audio_chunks.push(self.synthesize_escaped_chunk(&escaped, voice).await?);
        }
        Ok(audio_chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_audio_is_an_error_so_the_retry_loop_can_retry() {
        assert!(ensure_nonempty_audio(Vec::new()).is_err());
        assert_eq!(ensure_nonempty_audio(vec![1, 2, 3]).unwrap(), vec![1, 2, 3]);
    }
}
