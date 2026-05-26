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
        .next_back()
        .and_then(|p| p.parse::<u16>().ok())
}

fn is_server_running(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
}

fn start_vieneu_server(port: u16, mode: &str, device: &str) {
    if is_server_running(port) {
        tracing::info!(port, "VieNeu-TTS server is already running, skipping launch");
        return;
    }

    let mut python_cmd = "python".to_string();
    if std::path::Path::new("venv/Scripts/python.exe").exists() {
        python_cmd = "venv/Scripts/python.exe".to_string();
    } else if std::path::Path::new("venv/bin/python").exists() {
        python_cmd = "venv/bin/python".to_string();
    }

    tracing::info!(port, mode, device, python = %python_cmd, "Starting VieNeu-TTS server...");
    let log_file = std::fs::File::create("vieneu_server.log")
        .map(std::process::Stdio::from)
        .unwrap_or_else(|_| std::process::Stdio::null());
    let log_file_err = std::fs::File::create("vieneu_server_err.log")
        .map(std::process::Stdio::from)
        .unwrap_or_else(|_| std::process::Stdio::null());

    let result = std::process::Command::new(python_cmd)
        .args([
            "vieneu_server.py",
            "--port",
            &port.to_string(),
            "--mode",
            mode,
            "--device",
            device,
        ])
        .stdout(log_file)
        .stderr(log_file_err)
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

fn wait_for_server_ready(port: u16, timeout_secs: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let poll_interval = std::time::Duration::from_secs(3);

    tracing::info!(port, timeout_secs, "waiting for VieNeu-TTS server to become ready...");
    while std::time::Instant::now() < deadline {
        if is_server_running(port) {
            tracing::info!(port, "VieNeu-TTS server is now accepting connections");
            return;
        }
        std::thread::sleep(poll_interval);
    }
    tracing::warn!(port, timeout_secs, "VieNeu-TTS server did not become ready in time — TTS requests may fail until server finishes loading");
}

impl VieneuEngine {
    pub fn new(config: VieneuConfig) -> Self {
        if config.autostart {
            if let Some(port) = parse_port(&config.server_url) {
                let mode = config.mode.as_deref().unwrap_or("turbo");
                let device = config.device.as_deref().unwrap_or("cpu");
                start_vieneu_server(port, mode, device);
                if !is_server_running(port) {
                    wait_for_server_ready(port, 120);
                }
            }
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
            "temperature": self.config.temperature.unwrap_or(0.3),
            "pitch": self.config.pitch.unwrap_or(0),
        });

        let timeout = std::time::Duration::from_secs(5);
        let max_attempts = 2;

        for attempt in 1..=max_attempts {
            match self.client.post(&url).json(&payload).timeout(timeout).send().await {
                Ok(response) if response.status().is_success() => {
                    match response.bytes().await {
                        Ok(bytes) => {
                            tracing::debug!(
                                text_len = text.len(),
                                audio_len = bytes.len(),
                                "VieNeu-TTS synthesis complete"
                            );
                            return Ok(bytes.to_vec());
                        }
                        Err(e) if attempt < max_attempts => {
                            tracing::warn!("VieNeu-TTS read failed (attempt {attempt}/{max_attempts}): {e}");
                        }
                        Err(e) => anyhow::bail!("VieNeu-TTS failed to read bytes: {e}"),
                    }
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    if attempt >= max_attempts {
                        anyhow::bail!("VieNeu-TTS returned {status}: {body}");
                    }
                    tracing::warn!("VieNeu-TTS status error (attempt {attempt}/{max_attempts}): {status}");
                }
                Err(e) if attempt < max_attempts => {
                    tracing::warn!("VieNeu-TTS request failed (attempt {attempt}/{max_attempts}): {e}");
                }
                Err(e) => anyhow::bail!("VieNeu-TTS request failed: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        anyhow::bail!("VieNeu-TTS: unreachable")
    }
}
