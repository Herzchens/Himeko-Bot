use super::local_process::{self, ManagedProcess};
use super::TtsEngine;
use crate::config::VieneuConfig;
use reqwest::{Client, Url};
use std::sync::Arc;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(500);
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(30);

pub struct VieneuEngine {
    client: Client,
    config: VieneuConfig,
    _process: Option<Arc<ManagedProcess>>,
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim_start_matches('[').trim_end_matches(']');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn loopback_port(server_url: &str) -> anyhow::Result<Option<u16>> {
    let url = Url::parse(server_url)
        .map_err(|error| anyhow::anyhow!("invalid VieNeu server_url '{server_url}': {error}"))?;
    let Some(host) = url.host_str() else {
        anyhow::bail!("VieNeu server_url must include a host");
    };
    if !is_loopback_host(host) {
        return Ok(None);
    }
    url.port_or_known_default()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("VieNeu loopback server_url must include a usable port"))
}

fn health_url(server_url: &str) -> String {
    format!("{}/healthz", server_url.trim_end_matches('/'))
}

async fn health_ready(client: &Client, server_url: &str) -> bool {
    client
        .get(health_url(server_url))
        .timeout(HEALTH_REQUEST_TIMEOUT)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

async fn wait_for_server_ready(
    client: &Client,
    server_url: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if health_ready(client, server_url).await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "VieNeu-TTS health endpoint did not become ready within {} seconds",
                timeout.as_secs_f32()
            );
        }
        tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
    }
}

fn python_command() -> String {
    if std::path::Path::new("venv/Scripts/python.exe").exists() {
        "venv/Scripts/python.exe".to_string()
    } else if std::path::Path::new("venv/bin/python").exists() {
        "venv/bin/python".to_string()
    } else {
        "python".to_string()
    }
}

fn spawn_vieneu(port: u16, mode: &str, device: &str) -> anyhow::Result<std::process::Child> {
    let python = python_command();
    let stdout = std::fs::File::create("vieneu_server.log")
        .map(std::process::Stdio::from)
        .unwrap_or_else(|_| std::process::Stdio::null());
    let stderr = std::fs::File::create("vieneu_server_err.log")
        .map(std::process::Stdio::from)
        .unwrap_or_else(|_| std::process::Stdio::null());

    tracing::info!(port, mode, device, %python, "starting managed VieNeu-TTS server");
    std::process::Command::new(python)
        .args([
            "vieneu_server.py",
            "--port",
            &port.to_string(),
            "--mode",
            mode,
            "--device",
            device,
        ])
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|error| anyhow::anyhow!("failed to start VieNeu-TTS server: {error}"))
}

impl VieneuEngine {
    pub async fn new(config: VieneuConfig) -> anyhow::Result<Self> {
        if config.pitch.unwrap_or(0) != 0 {
            anyhow::bail!(
                "VieNeu-TTS pitch shifting is unsupported because the previous resampling path also changed duration; set tts.vieneu pitch to 0"
            );
        }

        let client = Client::builder().connect_timeout(CONNECT_TIMEOUT).build()?;
        let mut process = None;

        if config.autostart {
            match loopback_port(&config.server_url)? {
                Some(port) => {
                    let mode = config.mode.as_deref().unwrap_or("turbo");
                    let device = config.device.as_deref().unwrap_or("cpu");
                    let key = format!("vieneu:{port}");
                    let signature = format!("mode={mode};device={device}");

                    if let Some(existing) = local_process::existing(&key, &signature)? {
                        tracing::info!(
                            port,
                            pid = ?existing.pid(),
                            "reusing managed VieNeu-TTS process across engine reload"
                        );
                        wait_for_server_ready(&client, &config.server_url, STARTUP_TIMEOUT).await?;
                        process = Some(existing);
                    } else if health_ready(&client, &config.server_url).await {
                        tracing::info!(port, "using already-ready external VieNeu-TTS server");
                    } else {
                        let mode_owned = mode.to_string();
                        let device_owned = device.to_string();
                        let managed = local_process::spawn_managed(key, signature, move || {
                            spawn_vieneu(port, &mode_owned, &device_owned)
                        })?;
                        if let Err(error) =
                            wait_for_server_ready(&client, &config.server_url, STARTUP_TIMEOUT)
                                .await
                        {
                            drop(managed);
                            return Err(error);
                        }
                        tracing::info!(
                            port,
                            pid = ?managed.pid(),
                            "managed VieNeu-TTS server is healthy"
                        );
                        process = Some(managed);
                    }
                }
                None => {
                    tracing::warn!(
                        url = %config.server_url,
                        "VieNeu autostart ignored for non-loopback server_url; using remote server as configured"
                    );
                }
            }
        }

        Ok(Self {
            client,
            config,
            _process: process,
        })
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
impl TtsEngine for VieneuEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/v1/tts", self.config.server_url.trim_end_matches('/'));
        let voice_name = self.resolve_voice(voice);
        let payload = serde_json::json!({
            "text": text,
            "voice": voice_name,
            "speed": self.config.speed.unwrap_or(1.0),
            "temperature": self.config.temperature.unwrap_or(0.3),
            "pitch": 0,
        });
        let max_attempts = 2;

        for attempt in 1..=max_attempts {
            match self
                .client
                .post(&url)
                .json(&payload)
                .timeout(SYNTHESIS_TIMEOUT)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(bytes) if !bytes.is_empty() => {
                        tracing::debug!(
                            text_chars = text.chars().count(),
                            audio_len = bytes.len(),
                            "VieNeu-TTS synthesis complete"
                        );
                        return Ok(bytes.to_vec());
                    }
                    Ok(_) if attempt < max_attempts => {
                        tracing::warn!(
                            "VieNeu-TTS returned empty audio (attempt {attempt}/{max_attempts})"
                        );
                    }
                    Ok(_) => anyhow::bail!("VieNeu-TTS returned empty audio"),
                    Err(error) if attempt < max_attempts => {
                        tracing::warn!(
                            "VieNeu-TTS read failed (attempt {attempt}/{max_attempts}): {error}"
                        );
                    }
                    Err(error) => anyhow::bail!("VieNeu-TTS failed to read bytes: {error}"),
                },
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    if attempt >= max_attempts {
                        anyhow::bail!("VieNeu-TTS returned {status}: {body}");
                    }
                    tracing::warn!(
                        "VieNeu-TTS status error (attempt {attempt}/{max_attempts}): {status}"
                    );
                }
                Err(error) if attempt < max_attempts => {
                    tracing::warn!(
                        "VieNeu-TTS request failed (attempt {attempt}/{max_attempts}): {error}"
                    );
                }
                Err(error) => anyhow::bail!("VieNeu-TTS request failed: {error}"),
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        anyhow::bail!("VieNeu-TTS exhausted synthesis attempts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn autostart_only_targets_loopback_url() {
        assert_eq!(loopback_port("http://127.0.0.1:7799").unwrap(), Some(7799));
        assert_eq!(loopback_port("http://127.42.0.9:7799").unwrap(), Some(7799));
        assert_eq!(
            loopback_port("http://localhost:7799/v1").unwrap(),
            Some(7799)
        );
        assert_eq!(loopback_port("http://[::1]:7799").unwrap(), Some(7799));
        assert_eq!(loopback_port("https://tts.example.com:7799").unwrap(), None);
        assert_eq!(loopback_port("http://192.168.1.5:7799").unwrap(), None);
    }

    async fn one_health_response(status: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let response =
                format!("HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn health_probe_requires_success_status() {
        let client = Client::builder().build().unwrap();
        let (ready_url, ready_server) = one_health_response("200 OK").await;
        assert!(health_ready(&client, &ready_url).await);
        ready_server.await.unwrap();

        let (failed_url, failed_server) = one_health_response("503 Service Unavailable").await;
        assert!(!health_ready(&client, &failed_url).await);
        failed_server.await.unwrap();
    }
}
