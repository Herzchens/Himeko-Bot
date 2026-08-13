use super::local_process::{self, ManagedProcess};
use super::TtsEngine;
use crate::config::SupertonicConfig;
use reqwest::{Client, Url};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(30);
const LOCAL_PROBE_TIMEOUT: Duration = Duration::from_millis(250);

pub struct SupertonicEngine {
    client: Client,
    config: SupertonicConfig,
    _process: Option<Arc<ManagedProcess>>,
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim_start_matches('[').trim_end_matches(']');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn loopback_endpoint(server_url: &str) -> anyhow::Result<Option<(String, u16)>> {
    let url = Url::parse(server_url).map_err(|error| {
        anyhow::anyhow!("invalid Supertonic server_url '{server_url}': {error}")
    })?;
    let Some(host) = url.host_str() else {
        anyhow::bail!("Supertonic server_url must include a host");
    };
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

fn local_server_running(host: &str, port: u16) -> bool {
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    addresses
        .into_iter()
        .any(|address| TcpStream::connect_timeout(&address, LOCAL_PROBE_TIMEOUT).is_ok())
}

fn spawn_supertonic(port: u16) -> anyhow::Result<std::process::Child> {
    tracing::info!(port, "starting managed Supertonic server");
    std::process::Command::new("supertonic")
        .args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
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
    pub fn new(config: SupertonicConfig) -> Self {
        let process = if config.autostart {
            match loopback_endpoint(&config.server_url) {
                Ok(Some((host, port))) => {
                    let key = format!("supertonic:{port}");
                    let signature = "serve";
                    match local_process::existing(&key, signature) {
                        Ok(Some(existing)) => {
                            tracing::info!(
                                port,
                                pid = ?existing.pid(),
                                "reusing managed Supertonic process across engine reload"
                            );
                            Some(existing)
                        }
                        Ok(None) if local_server_running(&host, port) => {
                            tracing::info!(
                                port,
                                "using already-running external Supertonic server"
                            );
                            None
                        }
                        Ok(None) => match local_process::spawn_managed(key, signature, || {
                            spawn_supertonic(port)
                        }) {
                            Ok(process) => Some(process),
                            Err(error) => {
                                tracing::error!(%error, "failed to autostart Supertonic");
                                None
                            }
                        },
                        Err(error) => {
                            tracing::error!(%error, "cannot reuse managed Supertonic process");
                            None
                        }
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        url = %config.server_url,
                        "Supertonic autostart ignored for non-loopback server_url; using remote server as configured"
                    );
                    None
                }
                Err(error) => {
                    tracing::error!(%error, "invalid Supertonic autostart configuration");
                    None
                }
            }
        } else {
            None
        };

        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|error| {
                tracing::error!(%error, "failed to configure Supertonic HTTP client; using default client");
                Client::new()
            });

        Self {
            client,
            config,
            _process: process,
        }
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
                    Ok(_) if attempt < max_attempts => {
                        tracing::warn!(
                            "Supertonic returned empty audio (attempt {attempt}/{max_attempts})"
                        );
                    }
                    Ok(_) => anyhow::bail!("Supertonic returned empty audio"),
                    Err(error) if attempt < max_attempts => {
                        tracing::warn!(
                            "Supertonic read failed (attempt {attempt}/{max_attempts}): {error}"
                        );
                    }
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
                Err(error) if attempt < max_attempts => {
                    tracing::warn!(
                        "Supertonic request failed (attempt {attempt}/{max_attempts}): {error}"
                    );
                }
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
            loopback_endpoint("http://localhost:7788/v1").unwrap(),
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
}
