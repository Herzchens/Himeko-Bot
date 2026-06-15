use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

pub struct DiscordLogLayer {
    sender: mpsc::UnboundedSender<String>,
}

impl DiscordLogLayer {
    pub fn new(sender: mpsc::UnboundedSender<String>) -> Self {
        Self { sender }
    }
}

struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        }
    }
}

impl<S> Layer<S> for DiscordLogLayer
where
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        if metadata.target().starts_with("himeko_bot") {
            let mut visitor = MessageVisitor { message: String::new() };
            event.record(&mut visitor);
            
            if !visitor.message.is_empty() {
                let level = metadata.level().to_string();
                let target = metadata.target().strip_prefix("himeko_bot::").unwrap_or(metadata.target());
                
                // Clean up string representation if it's quoted
                let mut clean_msg = visitor.message;
                if clean_msg.starts_with('"') && clean_msg.ends_with('"') && clean_msg.len() >= 2 {
                    clean_msg = clean_msg[1..clean_msg.len()-1].to_string();
                }
                
                let formatted = format!("[{}] {}: {}", level, target, clean_msg);
                let _ = self.sender.send(formatted);
            }
        }
    }
}

pub fn start_discord_logging(webhook_url: String) -> (DiscordLogLayer, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let client = reqwest::Client::new();
    
    let handle = tokio::spawn(async move {
        let mut buffer = String::new();
        let mut last_send = std::time::Instant::now();
        
        loop {
            let msg_opt = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
            
            match msg_opt {
                Ok(Some(msg)) => {
                    if buffer.len() + msg.len() + 10 > 1900 {
                        send_to_webhook(&client, &webhook_url, &buffer).await;
                        buffer.clear();
                        last_send = std::time::Instant::now();
                    }
                    if !buffer.is_empty() {
                        buffer.push('\n');
                    }
                    buffer.push_str(&msg);
                }
                Ok(None) => {
                    if !buffer.is_empty() {
                        send_to_webhook(&client, &webhook_url, &buffer).await;
                    }
                    break;
                }
                Err(_) => {
                    if !buffer.is_empty() && last_send.elapsed() >= std::time::Duration::from_secs(1) {
                        send_to_webhook(&client, &webhook_url, &buffer).await;
                        buffer.clear();
                        last_send = std::time::Instant::now();
                    }
                }
            }
        }
    });
    
    (DiscordLogLayer::new(tx), handle)
}

async fn send_to_webhook(client: &reqwest::Client, url: &str, content: &str) {
    let payload = serde_json::json!({
        "content": format!("```ini\n{}\n```", content)
    });
    
    match client.post(url).json(&payload).send().await {
        Ok(res) => {
            if !res.status().is_success() {
                eprintln!("[DiscordLogger] Webhook returned status {}", res.status());
            }
        }
        Err(e) => {
            eprintln!("[DiscordLogger] Failed to send logs to Webhook: {}", e);
        }
    }
}
