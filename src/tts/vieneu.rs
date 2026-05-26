use super::TtsEngine;
use crate::config::VieneuConfig;
use reqwest::Client;

pub struct VieneuEngine {
    client: Client,
    config: VieneuConfig,
}

fn parse_port(server_url: &str) -> Option<u16> {
    server_url
        .split(':')
        .last()
        .and_then(|p| p.parse::<u16>().ok())
}

fn is_server_running(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
}

fn start_vieneu_server(port: u16) {
    if is_server_running(port) {
        tracing::info!(port, "VieNeu-TTS server is already running, skipping launch");
        return;
    }

    tracing::info!(port, "Starting VieNeu-TTS server...");
    let result = std::process::Command::new("python")
        .args(["vieneu_server.py"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(_) => {
            tracing::info!(port, "VieNeu-TTS server spawned successfully");
        }
        Err(e) => {
            tracing::error!(
                "Failed to start VieNeu-TTS server: {}. Please ensure python is in your PATH and vieneu_server.py is in the root directory.",
                e
            );
        }
    }
}

impl VieneuEngine {
    pub fn new(config: VieneuConfig) -> Self {
        if let Some(port) = parse_port(&config.server_url) {
            start_vieneu_server(port);
        }

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
impl TtsEngine for VieneuEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/v1/tts", self.config.server_url.trim_end_matches('/'));
        let voice_name = self.resolve_voice(voice);
        let payload = serde_json::json!({
            "text": text,
            "voice": voice_name,
            "speed": self.config.speed.unwrap_or(1.0),
        });

        let mut attempts = 0;
        let max_attempts = 3;

        loop {
            attempts += 1;
            match self.client.post(&url).json(&payload).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.bytes().await {
                            Ok(bytes) => {
                                tracing::debug!(
                                    text_len = text.len(),
                                    audio_len = bytes.len(),
                                    "VieNeu-TTS synthesis complete"
                                );
                                return Ok(bytes.to_vec());
                            }
                            Err(e) => {
                                if attempts >= max_attempts {
                                    anyhow::bail!("VieNeu-TTS failed to read bytes: {e}");
                                }
                                tracing::warn!("VieNeu-TTS read failed (attempt {attempts}/{max_attempts}): {e}");
                            }
                        }
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        if attempts >= max_attempts {
                            anyhow::bail!("VieNeu-TTS returned {status}: {body}");
                        }
                        tracing::warn!("VieNeu-TTS status error (attempt {attempts}/{max_attempts}): {status}");
                    }
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        anyhow::bail!("VieNeu-TTS request failed after {attempts} attempts: {e}");
                    }
                    tracing::warn!("VieNeu-TTS request failed (attempt {attempts}/{max_attempts}): {e}. Retrying in 500ms...");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
