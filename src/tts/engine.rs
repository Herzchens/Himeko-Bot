use super::TtsEngine;
use crate::config::TtsConfig;
use msedge_tts::tts::client::connect_async;
use msedge_tts::tts::SpeechConfig;

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
}

#[async_trait::async_trait]
impl TtsEngine for MsEdgeEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let speech_config = self.build_speech_config(voice);
        let mut attempts = 0;
        let max_attempts = 3;

        loop {
            attempts += 1;
            
            match connect_async().await {
                Ok(mut client) => {
                    match client.synthesize(text, &speech_config).await {
                        Ok(audio) => {
                            tracing::debug!(
                                voice = %voice,
                                text_len = text.len(),
                                audio_len = audio.audio_bytes.len(),
                                "TTS synthesis complete"
                            );
                            return Ok(audio.audio_bytes);
                        }
                        Err(e) => {
                            if attempts >= max_attempts {
                                return Err(anyhow::anyhow!("TTS synthesis failed after {} attempts: {}", attempts, e));
                            }
                            tracing::warn!("TTS synthesis failed (attempt {}/{}): {}. Retrying in 500ms...", attempts, max_attempts, e);
                        }
                    }
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        return Err(anyhow::anyhow!("failed to connect to Edge TTS after {} attempts: {}", attempts, e));
                    }
                    tracing::warn!("Failed to connect to Edge TTS (attempt {}/{}): {}. Retrying in 500ms...", attempts, max_attempts, e);
                }
            }
            
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
