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

        let mut client = connect_async()
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to Edge TTS: {}", e))?;

        let audio = client
            .synthesize(text, &speech_config)
            .await
            .map_err(|e| anyhow::anyhow!("TTS synthesis failed: {}", e))?;

        tracing::debug!(
            voice = %voice,
            text_len = text.len(),
            audio_len = audio.audio_bytes.len(),
            "TTS synthesis complete"
        );

        Ok(audio.audio_bytes)
    }
}
