use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Kiểm tra độ trễ của bot
#[poise::command(slash_command, guild_only)]
pub async fn ping(ctx: Context<'_>) -> Result<(), Error> {
    let config = ctx.data().config_snapshot().await;
    let level = UserLevel::of(ctx.author().id.get(), &config);

    if !level.can_use_tts() {
        ctx.send(
            CreateReply::default()
                .content("❌ Bạn không có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let before = std::time::Instant::now();
    let reply = ctx
        .send(
            CreateReply::default()
                .content("🏓 Đang đo...")
                .ephemeral(true),
        )
        .await?;
    let http_latency = before.elapsed();

    let shard_latency = {
        let shard_manager = ctx.framework().shard_manager();
        let runners = shard_manager.runners.lock().await;
        runners
            .values()
            .next()
            .and_then(|runner| runner.latency)
            .map(|d| format!("{}ms", d.as_millis()))
            .unwrap_or_else(|| "N/A".to_string())
    };

    let content = format!(
        "🏓 **Pong!**\n\
         WebSocket: {}\n\
         HTTP: {}ms",
        shard_latency,
        http_latency.as_millis()
    );

    reply
        .edit(ctx, CreateReply::default().content(content))
        .await?;

    Ok(())
}
