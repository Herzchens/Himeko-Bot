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

fn parse_server_url(server_url: &str) -> anyhow::Result<Url> {
    let url = Url::parse(server_url)
        .map_err(|error| anyhow::anyhow!("invalid VieNeu server_url '{server_url}': {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!(
            "VieNeu server_url must use http or https, got '{}'",
            url.scheme()
        );
    }
    if url.host_str().is_none() {
        anyhow::bail!("VieNeu server_url must include a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("VieNeu server_url must not contain credentials");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("VieNeu server_url must be a base URL without a path, query, or fragment");
    }
    Ok(url)
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim_start_matches('[').trim_end_matches(']');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn loopback_port(server_url: &str) -> anyhow::Result<Option<u16>> {
    let url = parse_server_url(server_url)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("VieNeu server_url must include a host"))?;
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
    let Ok(response) = client
        .get(health_url(server_url))
        .timeout(HEALTH_REQUEST_TIMEOUT)
        .send()
        .await
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let Ok(body) = response.json::<serde_json::Value>().await else {
        return false;
    };
    body.get("status").and_then(serde_json::Value::as_str) == Some("ok")
}

async fn wait_for_server_ready(
    client: &Client,
    server_url: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> anyhow::Result<()> {
    let poll_interval = if poll_interval.is_zero() {
        Duration::from_millis(1)
    } else {
        poll_interval
    };
    let ready = tokio::time::timeout(timeout, async {
        loop {
            if health_ready(client, server_url).await {
                break;
            }
            tokio::time::sleep(poll_interval).await;
        }
    })
    .await;
    if ready.is_err() {
        anyhow::bail!(
            "VieNeu-TTS health endpoint did not become ready within {} seconds",
            timeout.as_secs_f32()
        );
    }
    Ok(())
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
        Self::with_readiness_options(config, STARTUP_TIMEOUT, STARTUP_POLL_INTERVAL).await
    }

    async fn with_readiness_options(
        config: VieneuConfig,
        startup_timeout: Duration,
        poll_interval: Duration,
    ) -> anyhow::Result<Self> {
        if config.pitch.unwrap_or(0) != 0 {
            anyhow::bail!(
                "VieNeu-TTS pitch shifting is unsupported because the previous resampling path also changed duration; set tts.vieneu pitch to 0"
            );
        }

        parse_server_url(&config.server_url)?;
        let client = Client::builder().connect_timeout(CONNECT_TIMEOUT).build()?;
        let mut process = None;
        let mut readiness_proven = false;

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
                        process = Some(existing);
                    } else if health_ready(&client, &config.server_url).await {
                        readiness_proven = true;
                        tracing::info!(port, "using already-ready external VieNeu-TTS server");
                    } else {
                        let mode_owned = mode.to_string();
                        let device_owned = device.to_string();
                        let managed = local_process::spawn_managed(key, signature, move || {
                            spawn_vieneu(port, &mode_owned, &device_owned)
                        })?;
                        tracing::info!(
                            port,
                            pid = ?managed.pid(),
                            "managed VieNeu-TTS server started; waiting for readiness"
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

        if !readiness_proven {
            wait_for_server_ready(&client, &config.server_url, startup_timeout, poll_interval)
                .await?;
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
    fn server_url_must_be_root_http_base() {
        for valid in [
            "http://127.0.0.1:7799",
            "http://localhost:7799/",
            "https://tts.example.com",
        ] {
            parse_server_url(valid).unwrap_or_else(|error| {
                panic!("valid VieNeu base URL {valid:?} was rejected: {error}")
            });
        }

        for invalid in [
            "ftp://127.0.0.1:7799",
            "http://user:pass@127.0.0.1:7799",
            "http://127.0.0.1:7799/v1",
            "http://127.0.0.1:7799?mode=test",
            "http://127.0.0.1:7799#fragment",
        ] {
            assert!(
                parse_server_url(invalid).is_err(),
                "invalid VieNeu server_url was accepted: {invalid}"
            );
        }
    }

    #[test]
    fn autostart_only_targets_loopback_url() {
        assert_eq!(loopback_port("http://127.0.0.1:7799").unwrap(), Some(7799));
        assert_eq!(loopback_port("http://127.42.0.9:7799").unwrap(), Some(7799));
        assert!(loopback_port("http://localhost:7799/v1").is_err());
        assert_eq!(loopback_port("http://[::1]:7799").unwrap(), Some(7799));
        assert_eq!(loopback_port("https://tts.example.com:7799").unwrap(), None);
        assert_eq!(loopback_port("http://192.168.1.5:7799").unwrap(), None);
    }

    fn test_config(server_url: String, autostart: bool) -> VieneuConfig {
        VieneuConfig {
            server_url,
            voice_female: "Ly".to_string(),
            voice_male: "Binh".to_string(),
            speed: Some(1.0),
            autostart,
            mode: Some("turbo".to_string()),
            temperature: Some(0.3),
            device: Some("cpu".to_string()),
            pitch: Some(0),
        }
    }

    async fn one_health_response(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn health_probe_requires_expected_status_payload() {
        let client = Client::builder().build().unwrap();
        let (ready_url, ready_server) = one_health_response("200 OK", r#"{"status":"ok"}"#).await;
        assert!(health_ready(&client, &ready_url).await);
        ready_server.await.unwrap();

        let (invalid_url, invalid_server) = one_health_response("200 OK", "{}").await;
        assert!(!health_ready(&client, &invalid_url).await);
        invalid_server.await.unwrap();

        let (loading_url, loading_server) =
            one_health_response("503 Service Unavailable", r#"{"status":"loading"}"#).await;
        assert!(!health_ready(&client, &loading_url).await);
        loading_server.await.unwrap();
    }

    #[tokio::test]
    async fn readiness_wait_accepts_loading_then_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (status, body) in [
                ("503 Service Unavailable", r#"{"status":"loading"}"#),
                ("200 OK", r#"{"status":"ok"}"#),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                let _ = socket.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = Client::builder().build().unwrap();
        wait_for_server_ready(
            &client,
            &format!("http://{address}"),
            Duration::from_millis(500),
            Duration::from_millis(10),
        )
        .await
        .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn constructor_requires_readiness_when_autostart_is_disabled() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let started = std::time::Instant::now();
        let result = VieneuEngine::with_readiness_options(
            test_config(format!("http://{address}"), false),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        assert!(result.is_err(), "unready VieNeu provider must be rejected");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "test readiness deadline was not bounded"
        );
    }

    #[tokio::test]
    async fn constructor_accepts_ready_provider_when_autostart_is_disabled() {
        let (url, server) = one_health_response("200 OK", r#"{"status":"ok"}"#).await;
        let engine = VieneuEngine::with_readiness_options(
            test_config(url, false),
            Duration::from_millis(500),
            Duration::from_millis(10),
        )
        .await;
        server.await.unwrap();
        assert!(engine.is_ok(), "ready VieNeu provider should be accepted");
    }
}
