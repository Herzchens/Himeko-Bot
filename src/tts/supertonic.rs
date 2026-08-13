use super::local_process::{self, ManagedProcess};
use super::TtsEngine;
use crate::config::SupertonicConfig;
use reqwest::{Client, StatusCode, Url};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);
const STARTUP_POLL_DELAY: Duration = Duration::from_millis(250);
const LOCAL_PROBE_TIMEOUT: Duration = Duration::from_millis(250);

pub struct SupertonicEngine {
    client: Client,
    config: SupertonicConfig,
    _process: Option<Arc<ManagedProcess>>,
}

#[derive(Debug)]
enum HealthProbe {
    Ready,
    Loading(String),
    Unreachable(String),
    Invalid(String),
}

fn parse_server_url(server_url: &str) -> anyhow::Result<Url> {
    let url = Url::parse(server_url).map_err(|error| {
        anyhow::anyhow!("invalid Supertonic server_url '{server_url}': {error}")
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!(
            "Supertonic server_url must use http or https, got '{}'",
            url.scheme()
        );
    }
    if url.host_str().is_none() {
        anyhow::bail!("Supertonic server_url must include a host");
    }
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("Supertonic server_url must not contain credentials");
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!(
            "Supertonic server_url must be a base URL without a path, query, or fragment"
        );
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

fn loopback_endpoint(server_url: &str) -> anyhow::Result<Option<(String, u16)>> {
    let url = parse_server_url(server_url)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Supertonic server_url must include a host"))?;
    if !is_loopback_host(host) {
        return Ok(None);
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        anyhow::anyhow!("Supertonic loopback server_url must include a usable port")
    })?;
    let connect_host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    Ok(Some((connect_host, port)))
}

fn health_url(server_url: &str) -> anyhow::Result<Url> {
    let base = parse_server_url(server_url)?;
    base.join("v1/health")
        .map_err(|error| anyhow::anyhow!("failed to construct Supertonic health URL: {error}"))
}

async fn port_accepting(host: &str, port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            LOCAL_PROBE_TIMEOUT,
            tokio::net::TcpStream::connect((host, port)),
        )
        .await,
        Ok(Ok(_))
    )
}

async fn probe_health(client: &Client, url: &Url, timeout: Duration) -> HealthProbe {
    let request = async {
        let response = client.get(url.clone()).send().await?;
        let status = response.status();
        let body = response.text().await?;
        Ok::<_, reqwest::Error>((status, body))
    };

    let (status, body) = match tokio::time::timeout(timeout, request).await {
        Err(_) => {
            return HealthProbe::Unreachable(format!(
                "health request exceeded {}ms",
                timeout.as_millis()
            ));
        }
        Ok(Err(error)) => return HealthProbe::Unreachable(error.to_string()),
        Ok(Ok(result)) => result,
    };

    if status == StatusCode::SERVICE_UNAVAILABLE {
        return HealthProbe::Loading(format!("HTTP {status}: {body}"));
    }
    if !status.is_success() {
        return HealthProbe::Invalid(format!("HTTP {status}: {body}"));
    }

    let value: serde_json::Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(error) => {
            return HealthProbe::Invalid(format!(
                "health endpoint returned invalid JSON: {error}; body={body}"
            ));
        }
    };
    match value.get("status").and_then(serde_json::Value::as_str) {
        Some("ok") => HealthProbe::Ready,
        Some("loading") => HealthProbe::Loading(body),
        Some(other) => HealthProbe::Invalid(format!(
            "health endpoint returned unexpected status '{other}': {body}"
        )),
        None => HealthProbe::Invalid(format!(
            "health endpoint response is missing string field 'status': {body}"
        )),
    }
}

async fn wait_until_ready(
    client: &Client,
    url: &Url,
    startup_timeout: Duration,
    poll_delay: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + startup_timeout;
    let mut last_state = "not probed".to_string();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!(
                "Supertonic did not become ready within {}ms: {last_state}",
                startup_timeout.as_millis()
            );
        }
        let request_timeout = remaining.min(HEALTH_REQUEST_TIMEOUT);
        match probe_health(client, url, request_timeout).await {
            HealthProbe::Ready => return Ok(()),
            HealthProbe::Loading(reason) => last_state = format!("loading: {reason}"),
            HealthProbe::Unreachable(reason) => last_state = format!("unreachable: {reason}"),
            HealthProbe::Invalid(reason) => {
                anyhow::bail!("Supertonic health endpoint is incompatible: {reason}");
            }
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            continue;
        }
        tokio::time::sleep(poll_delay.min(remaining)).await;
    }
}

fn spawn_supertonic(host: &str, port: u16) -> anyhow::Result<std::process::Child> {
    tracing::info!(host, port, "starting managed Supertonic server");
    let port_string = port.to_string();
    std::process::Command::new("supertonic")
        .args(["serve", "--host", host, "--port", port_string.as_str()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to start Supertonic server; ensure the supertonic CLI is installed: {error}"
            )
        })
}

impl SupertonicEngine {
    pub async fn new(config: SupertonicConfig) -> anyhow::Result<Self> {
        Self::with_readiness_options(config, STARTUP_TIMEOUT, STARTUP_POLL_DELAY).await
    }

    async fn with_readiness_options(
        config: SupertonicConfig,
        startup_timeout: Duration,
        poll_delay: Duration,
    ) -> anyhow::Result<Self> {
        parse_server_url(&config.server_url)?;
        let ready_url = health_url(&config.server_url)?;
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(SYNTHESIS_TIMEOUT)
            .build()
            .map_err(|error| {
                anyhow::anyhow!("failed to configure Supertonic HTTP client: {error}")
            })?;

        let mut process = None;
        if config.autostart {
            match loopback_endpoint(&config.server_url)? {
                Some((host, port)) => {
                    let key = format!("supertonic:{host}:{port}");
                    let signature = format!("serve --host {host} --port {port}");
                    if let Some(existing) = local_process::existing(&key, &signature)? {
                        tracing::info!(
                            host,
                            port,
                            pid = ?existing.pid(),
                            "reusing managed Supertonic process across engine reload"
                        );
                        process = Some(existing);
                    } else {
                        let initial_probe = probe_health(
                            &client,
                            &ready_url,
                            startup_timeout.min(HEALTH_REQUEST_TIMEOUT),
                        )
                        .await;
                        match initial_probe {
                            HealthProbe::Ready | HealthProbe::Loading(_) => {
                                tracing::info!(
                                    host,
                                    port,
                                    "using already-running external Supertonic server"
                                );
                            }
                            HealthProbe::Invalid(reason) => {
                                anyhow::bail!(
                                    "Supertonic loopback endpoint is occupied by an incompatible service: {reason}"
                                );
                            }
                            HealthProbe::Unreachable(reason) => {
                                if port_accepting(&host, port).await {
                                    tracing::warn!(
                                        host,
                                        port,
                                        %reason,
                                        "Supertonic port accepts TCP but health is not reachable; refusing to start a competing process"
                                    );
                                } else {
                                    let spawn_host = host.clone();
                                    let managed =
                                        local_process::spawn_managed(key, signature, || {
                                            spawn_supertonic(&spawn_host, port)
                                        })?;
                                    tracing::info!(
                                        host,
                                        port,
                                        pid = ?managed.pid(),
                                        "managed Supertonic process started; waiting for health readiness"
                                    );
                                    process = Some(managed);
                                }
                            }
                        }
                    }
                }
                None => tracing::warn!(
                    url = %config.server_url,
                    "Supertonic autostart ignored for non-loopback server_url; waiting for remote server readiness"
                ),
            }
        }

        wait_until_ready(&client, &ready_url, startup_timeout, poll_delay).await?;

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
                            "Supertonic synthesis complete"
                        );
                        return Ok(bytes.to_vec());
                    }
                    Ok(_) if attempt < max_attempts => tracing::warn!(
                        "Supertonic returned empty audio (attempt {attempt}/{max_attempts})"
                    ),
                    Ok(_) => anyhow::bail!("Supertonic returned empty audio"),
                    Err(error) if attempt < max_attempts => tracing::warn!(
                        "Supertonic read failed (attempt {attempt}/{max_attempts}): {error}"
                    ),
                    Err(error) => anyhow::bail!("Supertonic failed to read bytes: {error}"),
                },
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    if attempt >= max_attempts {
                        anyhow::bail!("Supertonic returned {status}: {body}");
                    }
                    tracing::warn!(
                        "Supertonic status error (attempt {attempt}/{max_attempts}): {status}"
                    );
                }
                Err(error) if attempt < max_attempts => tracing::warn!(
                    "Supertonic request failed (attempt {attempt}/{max_attempts}): {error}"
                ),
                Err(error) => anyhow::bail!("Supertonic request failed: {error}"),
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        anyhow::bail!("Supertonic exhausted synthesis attempts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn config(server_url: String, autostart: bool) -> SupertonicConfig {
        SupertonicConfig {
            server_url,
            voice_female: "F2".to_string(),
            voice_male: "M1".to_string(),
            lang: "vi".to_string(),
            steps: Some(5),
            speed: Some(1.0),
            autostart,
        }
    }

    async fn start_health_server(
        loading_before_ready: usize,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_task = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = [0u8; 4096];
                let Ok(read) = socket.read(&mut buffer).await else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..read]);
                if !request.starts_with("GET /v1/health ") {
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    continue;
                }
                let index = requests_for_task.fetch_add(1, Ordering::SeqCst);
                if index < loading_before_ready {
                    let body = br#"{"status":"loading"}"#;
                    let response = format!(
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        String::from_utf8_lossy(body)
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                } else {
                    let body = br#"{"status":"ok"}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        String::from_utf8_lossy(body)
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                }
            }
        });
        (format!("http://{address}"), requests, task)
    }

    #[test]
    fn autostart_only_targets_loopback_url() {
        assert_eq!(
            loopback_endpoint("http://127.0.0.1:7788").unwrap(),
            Some(("127.0.0.1".to_string(), 7788))
        );
        assert_eq!(
            loopback_endpoint("http://127.42.0.9:7788").unwrap(),
            Some(("127.42.0.9".to_string(), 7788))
        );
        assert_eq!(
            loopback_endpoint("http://localhost:7788").unwrap(),
            Some(("localhost".to_string(), 7788))
        );
        assert_eq!(
            loopback_endpoint("http://[::1]:7788").unwrap(),
            Some(("::1".to_string(), 7788))
        );
        assert_eq!(
            loopback_endpoint("https://tts.example.com:7788").unwrap(),
            None
        );
        assert_eq!(loopback_endpoint("http://192.168.1.5:7788").unwrap(), None);
    }

    #[test]
    fn server_url_requires_http_root_base_url() {
        assert!(parse_server_url("not a url").is_err());
        assert!(parse_server_url("ftp://127.0.0.1:7788").is_err());
        assert!(parse_server_url("http://127.0.0.1:7788/v1").is_err());
        assert!(parse_server_url("http://127.0.0.1:7788?x=1").is_err());
        assert!(parse_server_url("http://user:pass@127.0.0.1:7788").is_err());
        assert!(parse_server_url("http://127.0.0.1:7788").is_ok());
    }

    #[tokio::test]
    async fn readiness_waits_for_official_health_endpoint() {
        let (server_url, requests, task) = start_health_server(1).await;
        let engine = SupertonicEngine::with_readiness_options(
            config(server_url, false),
            Duration::from_secs(2),
            Duration::from_millis(10),
        )
        .await;
        assert!(engine.is_ok());
        assert!(requests.load(Ordering::SeqCst) >= 2);
        task.abort();
    }

    #[tokio::test]
    async fn unreachable_server_fails_with_bounded_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let started = Instant::now();
        let error = SupertonicEngine::with_readiness_options(
            config(format!("http://{address}"), false),
            Duration::from_millis(100),
            Duration::from_millis(10),
        )
        .await
        .err()
        .expect("unreachable server must fail readiness");
        assert!(error.to_string().contains("did not become ready"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
