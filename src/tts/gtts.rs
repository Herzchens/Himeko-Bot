use super::chunking::{split_strict_chars, GTTS_MAX_CHARS};
use super::TtsEngine;
use reqwest::Client;
use std::time::Duration;

const MAX_ATTEMPTS: usize = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(500);
const DEFAULT_ENDPOINT: &str = "https://translate.google.com/translate_tts";

pub struct GttsEngine {
    client: Client,
    endpoint: String,
    retry_delay: Duration,
}

impl GttsEngine {
    pub fn new() -> Self {
        Self::with_options(DEFAULT_ENDPOINT.to_string(), REQUEST_TIMEOUT, RETRY_DELAY)
    }

    fn with_options(endpoint: String, request_timeout: Duration, retry_delay: Duration) -> Self {
        crate::tls::install_crypto_provider().expect("rustls crypto provider must be available");
        let client = Client::builder()
            .connect_timeout(std::cmp::min(CONNECT_TIMEOUT, request_timeout))
            .timeout(request_timeout)
            .build()
            .expect("valid static gTTS HTTP client configuration");
        Self {
            client,
            endpoint,
            retry_delay,
        }
    }

    async fn synthesize_one(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let tl = if voice.starts_with("en-") || voice == "en" {
            "en"
        } else {
            "vi"
        };

        for attempt in 1..=MAX_ATTEMPTS {
            let result = async {
                let response = self
                    .client
                    .get(&self.endpoint)
                    .query(&[
                        ("ie", "UTF-8"),
                        ("tl", tl),
                        ("client", "tw-ob"),
                        ("q", text),
                    ])
                    .send()
                    .await?;
                let response = response.error_for_status()?;
                let bytes = response.bytes().await?.to_vec();
                if bytes.is_empty() {
                    anyhow::bail!("gTTS returned empty audio");
                }
                Ok::<Vec<u8>, anyhow::Error>(bytes)
            }
            .await;

            match result {
                Ok(bytes) => {
                    tracing::debug!(
                        chars = text.chars().count(),
                        audio_len = bytes.len(),
                        attempt,
                        "gTTS synthesis complete"
                    );
                    return Ok(bytes);
                }
                Err(error) if attempt == MAX_ATTEMPTS => {
                    return Err(anyhow::anyhow!(
                        "gTTS request failed after {attempt} attempts: {error}"
                    ));
                }
                Err(error) => {
                    tracing::warn!(
                        attempt,
                        max_attempts = MAX_ATTEMPTS,
                        error = %error,
                        "gTTS request failed; retrying"
                    );
                    tokio::time::sleep(self.retry_delay).await;
                }
            }
        }

        Err(anyhow::anyhow!("gTTS synthesis exhausted retries"))
    }
}

#[async_trait::async_trait]
impl TtsEngine for GttsEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        self.synthesize_one(text, voice).await
    }

    async fn synthesize_chunks(&self, text: &str, voice: &str) -> anyhow::Result<Vec<Vec<u8>>> {
        let mut audio_chunks = Vec::new();
        for chunk in split_strict_chars(text, GTTS_MAX_CHARS) {
            audio_chunks.push(self.synthesize_one(&chunk, voice).await?);
        }
        Ok(audio_chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn start_ok_server() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let count_for_task = Arc::clone(&count);

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                count_for_task.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut request = vec![0u8; 8192];
                    let _ = socket.read(&mut request).await;
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx",
                        )
                        .await;
                });
            }
        });

        (format!("http://{address}/translate_tts"), count)
    }

    async fn start_hanging_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut request = [0u8; 1024];
                    let _ = socket.read(&mut request).await;
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    drop(socket);
                });
            }
        });
        format!("http://{address}/translate_tts")
    }

    #[tokio::test]
    async fn long_text_is_split_into_multiple_real_http_requests() {
        let (endpoint, requests) = start_ok_server().await;
        let engine =
            GttsEngine::with_options(endpoint, Duration::from_secs(2), Duration::from_millis(1));
        let text = "xin chào mọi người ".repeat(20);
        let expected = split_strict_chars(&text, GTTS_MAX_CHARS).len();
        assert!(expected > 1);

        let chunks = engine.synthesize_chunks(&text, "vi").await.unwrap();
        assert_eq!(chunks.len(), expected);
        assert_eq!(requests.load(Ordering::SeqCst), expected);
    }

    #[tokio::test]
    async fn request_timeout_is_enforced_without_hanging_forever() {
        let endpoint = start_hanging_server().await;
        let engine = GttsEngine::with_options(
            endpoint,
            Duration::from_millis(50),
            Duration::from_millis(1),
        );
        let started = std::time::Instant::now();
        let result = engine.synthesize("xin chào", "vi").await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
