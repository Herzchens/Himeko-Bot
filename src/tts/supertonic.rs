use super::TtsEngine;
use crate::config::SupertonicConfig;
use reqwest::Client;

pub struct SupertonicEngine {
    client: Client,
    config: SupertonicConfig,
}

fn parse_port(server_url: &str) -> Option<u16> {
    server_url
        .split(':')
        .next_back()
        .and_then(|p| p.parse::<u16>().ok())
}

fn is_server_running(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
}

fn start_supertonic_server(port: u16) {
    if is_server_running(port) {
        tracing::info!(port, "Supertonic server is already running, skipping launch");
        return;
    }

    tracing::info!(port, "Starting Supertonic server...");
    let result = std::process::Command::new("supertonic")
        .args(["serve", "--port", &port.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match result {
        Ok(_) => {
            tracing::info!(port, "Supertonic server spawned successfully");
        }
        Err(e) => {
            tracing::error!(
                "Failed to start Supertonic server: {}. Please ensure supertonic CLI is installed and in your PATH.",
                e
            );
        }
    }
}

impl SupertonicEngine {
    pub fn new(config: SupertonicConfig) -> Self {
        if let Some(port) = parse_port(&config.server_url) {
            start_supertonic_server(port);
        } else {
            tracing::warn!(url = %config.server_url, "Could not parse port from server_url, skipping automatic Supertonic launch");
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

    fn resolve_lang(&self, voice: &str) -> &str {
        if voice.starts_with("en-") || voice == "en" {
            "en"
        } else {
            &self.config.lang
        }
    }

    fn build_payload(&self, text: &str, voice: &str) -> serde_json::Value {
        let voice_name = self.resolve_voice(voice);
        let lang = self.resolve_lang(voice);
        let mut payload = serde_json::json!({
            "text": text,
            "voice": voice_name,
            "lang": lang,
            "response_format": "wav",
        });
        if let Some(steps) = self.config.steps {
            payload["steps"] = serde_json::json!(steps);
        }
        if let Some(speed) = self.config.speed {
            payload["speed"] = serde_json::json!(speed);
        }
        payload
    }
}

#[async_trait::async_trait]
impl TtsEngine for SupertonicEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/v1/tts", self.config.server_url.trim_end_matches('/'));
        let payload = self.build_payload(text, voice);
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
                                    "Supertonic synthesis complete"
                                );
                                return Ok(bytes.to_vec());
                            }
                            Err(e) => {
                                if attempts >= max_attempts {
                                    anyhow::bail!("Supertonic failed to read bytes: {e}");
                                }
                                tracing::warn!("Supertonic read failed (attempt {attempts}/{max_attempts}): {e}");
                            }
                        }
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        if attempts >= max_attempts {
                            anyhow::bail!("Supertonic returned {status}: {body}");
                        }
                        tracing::warn!("Supertonic status error (attempt {attempts}/{max_attempts}): {status}");
                    }
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        anyhow::bail!("Supertonic request failed after {attempts} attempts: {e}");
                    }
                    tracing::warn!("Supertonic request failed (attempt {attempts}/{max_attempts}): {e}. Retrying in 500ms...");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
