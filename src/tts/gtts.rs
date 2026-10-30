use super::chunking::{split_strict_chars, GTTS_MAX_CHARS};
use super::TtsEngine;
use futures_util::{stream, StreamExt, TryStreamExt};
use reqwest::{
    header::{HeaderMap, HeaderValue, CONTENT_TYPE, REFERER, USER_AGENT},
    Client,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, Semaphore};

const MAX_ATTEMPTS: usize = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(500);
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const TCP_KEEPALIVE: Duration = Duration::from_secs(60);
const HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(45);
const HTTP2_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PARALLEL_REQUESTS_PER_MESSAGE: usize = 3;
const MAX_PARALLEL_REQUESTS_GLOBAL: usize = 8;
const DEFAULT_ENDPOINT: &str = "https://translate.google.com/translate_tts";
const GOOGLE_REFERER: &str = "https://translate.google.com/";
const GOOGLE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0 Safari/537.36";
pub const MAX_REQUESTS_PER_MESSAGE: usize = 32;
pub const MESSAGE_SYNTHESIS_TIMEOUT: Duration = Duration::from_secs(90);

pub fn validate_request_budget<'a>(
    segments: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<usize> {
    let mut requests = 0usize;
    for segment in segments {
        requests = requests.saturating_add(split_strict_chars(segment, GTTS_MAX_CHARS).len());
        if requests > MAX_REQUESTS_PER_MESSAGE {
            anyhow::bail!(
                "gTTS message requires {requests} HTTP requests; safety limit is {MAX_REQUESTS_PER_MESSAGE}"
            );
        }
    }
    Ok(requests)
}

fn resolve_language_tag(voice: &str) -> anyhow::Result<&str> {
    match voice {
        "vi" | "en" | "ja" | "ko" | "zh" | "zh-CN" | "zh-TW" => Ok(voice),
        _ if voice.starts_with("en-") => Ok("en"),
        _ => anyhow::bail!("unsupported gTTS language tag: {voice}"),
    }
}

pub struct GttsEngine {
    client: Client,
    endpoint: String,
    retry_delay: Duration,
    request_gate: Arc<Semaphore>,
}

impl GttsEngine {
    pub fn new() -> Self {
        Self::with_options(DEFAULT_ENDPOINT.to_string(), REQUEST_TIMEOUT, RETRY_DELAY)
    }

    fn with_options(endpoint: String, request_timeout: Duration, retry_delay: Duration) -> Self {
        crate::tls::install_crypto_provider().expect("rustls crypto provider must be available");
        let mut default_headers = HeaderMap::new();
        default_headers.insert(REFERER, HeaderValue::from_static(GOOGLE_REFERER));
        default_headers.insert(USER_AGENT, HeaderValue::from_static(GOOGLE_USER_AGENT));

        let client = Client::builder()
            .tls_backend_rustls()
            .default_headers(default_headers)
            .connect_timeout(std::cmp::min(CONNECT_TIMEOUT, request_timeout))
            .timeout(request_timeout)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .tcp_keepalive(TCP_KEEPALIVE)
            .http2_keep_alive_interval(HTTP2_KEEPALIVE_INTERVAL)
            .http2_keep_alive_timeout(HTTP2_KEEPALIVE_TIMEOUT)
            .http2_keep_alive_while_idle(true)
            .build()
            .expect("valid static gTTS HTTP client configuration");
        Self {
            client,
            endpoint,
            retry_delay,
            request_gate: Arc::new(Semaphore::new(MAX_PARALLEL_REQUESTS_GLOBAL)),
        }
    }

    async fn synthesize_one(
        &self,
        text: &str,
        voice: &str,
        index: usize,
        total: usize,
    ) -> anyhow::Result<Vec<u8>> {
        let tl = resolve_language_tag(voice)?;
        let text_len = text.chars().count().to_string();
        let index = index.to_string();
        let total = total.to_string();

        for attempt in 1..=MAX_ATTEMPTS {
            let gate_started = std::time::Instant::now();
            let request_permit = Arc::clone(&self.request_gate)
                .acquire_owned()
                .await
                .map_err(|_| anyhow::anyhow!("gTTS request gate closed"))?;
            let gate_wait_ms = gate_started.elapsed().as_millis();

            let result = async {
                let request_started = std::time::Instant::now();
                let response = self
                    .client
                    .get(&self.endpoint)
                    .query(&[
                        ("ie", "UTF-8"),
                        ("tl", tl),
                        ("client", "tw-ob"),
                        ("q", text),
                        ("textlen", text_len.as_str()),
                        ("idx", index.as_str()),
                        ("total", total.as_str()),
                    ])
                    .send()
                    .await?;
                let headers_ms = request_started.elapsed().as_millis();
                let http_version = response.version();
                let status = response.status();
                let content_type = response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let response = response.error_for_status()?;
                if !content_type.starts_with("audio/") {
                    anyhow::bail!(
                        "gTTS returned unexpected content type '{}'",
                        if content_type.is_empty() {
                            "<missing>"
                        } else {
                            content_type.as_str()
                        }
                    );
                }

                let body_started = std::time::Instant::now();
                let bytes = response.bytes().await?.to_vec();
                let body_ms = body_started.elapsed().as_millis();
                if bytes.is_empty() {
                    anyhow::bail!("gTTS returned empty audio");
                }

                tracing::debug!(
                    attempt,
                    chars = text.chars().count(),
                    status = %status,
                    http_version = ?http_version,
                    content_type = %content_type,
                    gate_wait_ms,
                    headers_ms,
                    body_ms,
                    total_ms = request_started.elapsed().as_millis(),
                    audio_len = bytes.len(),
                    "gTTS HTTP request complete"
                );
                Ok::<Vec<u8>, anyhow::Error>(bytes)
            }
            .await;
            drop(request_permit);

            match result {
                Ok(bytes) => return Ok(bytes),
                Err(error) if attempt == MAX_ATTEMPTS || !is_retryable_error(&error) => {
                    return Err(anyhow::anyhow!(
                        "gTTS request failed after {attempt} attempt(s): {error}"
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

fn is_retryable_error(error: &anyhow::Error) -> bool {
    let Some(error) = error.downcast_ref::<reqwest::Error>() else {
        return true;
    };
    let Some(status) = error.status() else {
        return true;
    };
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

#[async_trait::async_trait]
impl TtsEngine for GttsEngine {
    async fn synthesize(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        self.synthesize_one(text, voice, 0, 1).await
    }

    async fn synthesize_chunks(&self, text: &str, voice: &str) -> anyhow::Result<Vec<Vec<u8>>> {
        let plan = vec![(text.to_string(), voice.to_string())];
        self.synthesize_plan(&plan).await
    }

    async fn synthesize_plan(&self, plan: &[(String, String)]) -> anyhow::Result<Vec<Vec<u8>>> {
        let requests = plan
            .iter()
            .flat_map(|(text, voice)| {
                split_strict_chars(text, GTTS_MAX_CHARS)
                    .into_iter()
                    .map(|chunk| (chunk, voice.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let total = requests.len();
        stream::iter(
            requests
                .into_iter()
                .enumerate()
                .map(|(index, (text, voice))| async move {
                    self.synthesize_one(&text, &voice, index, total)
                        .await
                        .map_err(|error| anyhow::anyhow!("voice {voice}: {error}"))
                }),
        )
        .buffered(MAX_PARALLEL_REQUESTS_PER_MESSAGE)
        .try_collect()
        .await
    }

    async fn synthesize_plan_stream(
        &self,
        plan: Vec<(String, String)>,
        sender: mpsc::Sender<anyhow::Result<Vec<u8>>>,
    ) -> anyhow::Result<()> {
        let requests = plan
            .into_iter()
            .flat_map(|(text, voice)| {
                split_strict_chars(&text, GTTS_MAX_CHARS)
                    .into_iter()
                    .map(|chunk| (chunk, voice.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let total = requests.len();
        let mut pending = stream::iter(requests.into_iter().enumerate().map(
            |(index, (text, voice))| async move {
                self.synthesize_one(&text, &voice, index, total)
                    .await
                    .map_err(|error| anyhow::anyhow!("voice {voice}: {error}"))
            },
        ))
        .buffered(MAX_PARALLEL_REQUESTS_PER_MESSAGE);

        while let Some(result) = pending.next().await {
            let failed = result.is_err();
            if sender.send(result).await.is_err() {
                return Ok(());
            }
            if failed {
                return Ok(());
            }
        }
        Ok(())
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
                            b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx",
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

    #[test]
    fn multilingual_language_tags_are_supported() {
        for tag in ["vi", "en", "ja", "ko", "zh", "zh-CN", "zh-TW"] {
            assert_eq!(resolve_language_tag(tag).unwrap(), tag);
        }
        assert_eq!(resolve_language_tag("en-US").unwrap(), "en");
        assert!(resolve_language_tag("fr").is_err());
    }

    #[test]
    fn per_message_request_budget_prevents_segment_fanout() {
        let accepted = vec!["hello"; MAX_REQUESTS_PER_MESSAGE];
        assert_eq!(
            validate_request_budget(accepted.iter().copied()).unwrap(),
            MAX_REQUESTS_PER_MESSAGE
        );

        let rejected = vec!["hello"; MAX_REQUESTS_PER_MESSAGE + 1];
        assert!(validate_request_budget(rejected.iter().copied()).is_err());
    }

    #[tokio::test]
    async fn permanent_http_error_is_not_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_task = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                requests_for_task.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut request = [0u8; 2048];
                    let _ = socket.read(&mut request).await;
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let engine = GttsEngine::with_options(
            format!("http://{address}/translate_tts"),
            Duration::from_secs(2),
            Duration::from_millis(1),
        );
        assert!(engine.synthesize("xin chào", "vi").await.is_err());
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn multilingual_plan_runs_requests_concurrently_but_preserves_order() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let active_for_server = Arc::clone(&active);
        let max_for_server = Arc::clone(&max_active);

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let active = Arc::clone(&active_for_server);
                let max_active = Arc::clone(&max_for_server);
                tokio::spawn(async move {
                    let mut request = vec![0u8; 4096];
                    let read = socket.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..read]);
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(current, Ordering::SeqCst);

                    let (delay, body): (u64, &[u8]) = if request.contains("q=first") {
                        (80, b"1")
                    } else if request.contains("q=second") {
                        (10, b"2")
                    } else {
                        (20, b"3")
                    };
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(body).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        let engine = GttsEngine::with_options(
            format!("http://{address}/translate_tts"),
            Duration::from_secs(2),
            Duration::from_millis(1),
        );
        let chunks = engine
            .synthesize_plan(&[
                ("first".to_string(), "vi".to_string()),
                ("second".to_string(), "en".to_string()),
                ("third".to_string(), "ja".to_string()),
            ])
            .await
            .unwrap();

        assert_eq!(chunks, vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()]);
        assert!(
            max_active.load(Ordering::SeqCst) >= 2,
            "at least two gTTS requests should overlap"
        );
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
