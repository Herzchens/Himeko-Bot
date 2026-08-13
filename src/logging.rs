use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const LOG_QUEUE_CAPACITY: usize = 512;
const WEBHOOK_CONTENT_LIMIT: usize = 1900;
const WEBHOOK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WEBHOOK_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const RECEIVE_TICK: Duration = Duration::from_millis(500);

pub struct DiscordLogLayer {
    sender: mpsc::Sender<String>,
    dropped: Arc<AtomicU64>,
}

impl DiscordLogLayer {
    fn new(sender: mpsc::Sender<String>, dropped: Arc<AtomicU64>) -> Self {
        Self { sender, dropped }
    }

    fn queue(&self, message: String) {
        queue_nonblocking(&self.sender, &self.dropped, message);
    }
}

fn queue_nonblocking(sender: &mpsc::Sender<String>, dropped: &AtomicU64, message: String) {
    match sender.try_send(message) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

struct MessageVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let name = field.name();
        let val_str = format!("{value:?}");
        if name == "message" {
            self.message = val_str;
        } else {
            self.fields.push((name.to_string(), val_str));
        }
    }
}

impl<S> Layer<S> for DiscordLogLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        if !metadata.target().starts_with("himeko_bot") {
            return;
        }

        let mut visitor = MessageVisitor {
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut visitor);
        if visitor.message.is_empty() {
            return;
        }

        let level = metadata.level().to_string();
        let target = metadata
            .target()
            .strip_prefix("himeko_bot::")
            .unwrap_or(metadata.target());
        let mut clean_msg = visitor.message;
        strip_debug_quotes(&mut clean_msg);

        let mut fields = String::new();
        for (name, mut value) in visitor.fields {
            strip_debug_quotes(&mut value);
            fields.push_str(&format!(" {name}={value}"));
        }
        self.queue(format!("**[{level}] {target}:** {clean_msg}{fields}"));
    }
}

fn strip_debug_quotes(value: &mut String) {
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        *value = value[1..value.len() - 1].to_string();
    }
}

pub fn start_discord_logging(
    webhook_url: String,
) -> (DiscordLogLayer, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(LOG_QUEUE_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    let layer = DiscordLogLayer::new(tx, Arc::clone(&dropped));
    let handle = tokio::spawn(run_worker(webhook_url, rx, dropped));
    (layer, handle)
}

async fn run_worker(webhook_url: String, mut rx: mpsc::Receiver<String>, dropped: Arc<AtomicU64>) {
    let client = match reqwest::Client::builder()
        .connect_timeout(WEBHOOK_CONNECT_TIMEOUT)
        .timeout(WEBHOOK_REQUEST_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[DiscordLogger] Failed to build HTTP client: {error}");
            return;
        }
    };

    let mut buffer = String::new();
    let mut last_send = Instant::now();
    loop {
        let received = tokio::time::timeout(RECEIVE_TICK, rx.recv()).await;
        match received {
            Ok(Some(message)) => {
                emit_dropped_notice(&mut buffer, &client, &webhook_url, &dropped).await;
                append_bounded(&mut buffer, &client, &webhook_url, &message).await;
                if last_send.elapsed() >= FLUSH_INTERVAL {
                    flush(&mut buffer, &client, &webhook_url).await;
                    last_send = Instant::now();
                }
            }
            Ok(None) => {
                emit_dropped_notice(&mut buffer, &client, &webhook_url, &dropped).await;
                flush(&mut buffer, &client, &webhook_url).await;
                break;
            }
            Err(_) => {
                emit_dropped_notice(&mut buffer, &client, &webhook_url, &dropped).await;
                if last_send.elapsed() >= FLUSH_INTERVAL {
                    flush(&mut buffer, &client, &webhook_url).await;
                    last_send = Instant::now();
                }
            }
        }
    }
}

async fn emit_dropped_notice(
    buffer: &mut String,
    client: &reqwest::Client,
    webhook_url: &str,
    dropped: &AtomicU64,
) {
    let count = dropped.swap(0, Ordering::AcqRel);
    if count > 0 {
        append_bounded(
            buffer,
            client,
            webhook_url,
            &format!("⚠️ Discord logger dropped {count} events due to bounded backpressure."),
        )
        .await;
    }
}

async fn append_bounded(
    buffer: &mut String,
    client: &reqwest::Client,
    webhook_url: &str,
    message: &str,
) {
    for chunk in split_unicode(message, WEBHOOK_CONTENT_LIMIT) {
        let separator = usize::from(!buffer.is_empty());
        if buffer.chars().count() + separator + chunk.chars().count() > WEBHOOK_CONTENT_LIMIT {
            flush(buffer, client, webhook_url).await;
        }
        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(&chunk);
        if buffer.chars().count() >= WEBHOOK_CONTENT_LIMIT {
            flush(buffer, client, webhook_url).await;
        }
    }
}

fn split_unicode(input: &str, max_chars: usize) -> Vec<String> {
    if input.is_empty() || max_chars == 0 {
        return Vec::new();
    }
    let chars = input.chars().collect::<Vec<_>>();
    chars
        .chunks(max_chars)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

async fn flush(buffer: &mut String, client: &reqwest::Client, webhook_url: &str) {
    if buffer.is_empty() {
        return;
    }
    let content = std::mem::take(buffer);
    if let Err(error) = send_to_webhook(client, webhook_url, &content).await {
        eprintln!("[DiscordLogger] Failed to send logs to webhook: {error}");
    }
}

async fn send_to_webhook(client: &reqwest::Client, url: &str, content: &str) -> anyhow::Result<()> {
    if content.is_empty() {
        return Ok(());
    }
    let response = client
        .post(url)
        .json(&serde_json::json!({ "content": content }))
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("webhook returned status {}", response.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn unicode_split_preserves_content_and_character_limit() {
        let input = "🔥xin chào🙂".repeat(500);
        let chunks = split_unicode(&input, 37);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 37));
        assert_eq!(chunks.concat(), input);
    }

    #[tokio::test]
    async fn bounded_queue_drops_without_blocking_or_growing() {
        let (tx, _rx) = mpsc::channel(1);
        let dropped = AtomicU64::new(0);
        queue_nonblocking(&tx, &dropped, "first".into());
        queue_nonblocking(&tx, &dropped, "second".into());
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert_eq!(tx.capacity(), 0);
    }

    #[tokio::test]
    async fn oversized_single_event_is_sent_as_nonempty_bounded_payloads() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut bodies = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0u8; 8192];
                let read = socket.read(&mut request).await.unwrap();
                let text = String::from_utf8_lossy(&request[..read]);
                let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                bodies.push(body);
                socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
            bodies
        });

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let url = format!("http://{address}/webhook");
        let mut buffer = String::new();
        let input = "🙂".repeat(WEBHOOK_CONTENT_LIMIT * 2 + 1);
        append_bounded(&mut buffer, &client, &url, &input).await;
        flush(&mut buffer, &client, &url).await;
        let bodies = server.await.unwrap();
        assert_eq!(bodies.len(), 3);
        assert!(bodies.iter().all(|body| !body.contains("\"content\":\"\"")));
    }

    #[tokio::test]
    async fn webhook_request_timeout_is_enforced() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                let _socket = socket;
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let started = Instant::now();
        let result = send_to_webhook(&client, &format!("http://{address}/webhook"), "x").await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
