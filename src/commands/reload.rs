use crate::config::Config;
use crate::permissions::UserLevel;
use crate::text::normalizer::Normalizer;
use crate::tts::engine::MsEdgeEngine;
use crate::tts::gtts::GttsEngine;
use crate::tts::TtsEngine;
use crate::Data;
use poise::CreateReply;
use std::sync::Arc;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Tải lại file config và cập nhật bot
#[poise::command(slash_command, guild_only)]
pub async fn reload(ctx: Context<'_>) -> Result<(), Error> {
    let config = ctx.data().config.read().await.clone();
    let level = UserLevel::of(ctx.author().id.get(), &config);

    if !level.can_preempt() {
        ctx.send(
            CreateReply::default()
                .content("❌ Chỉ owner mới có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    match Config::load("config.yml") {
        Ok(new_config) => {
            let new_config = Arc::new(new_config);
            let normalizer = Arc::new(Normalizer::from_config(&new_config.abbreviations));
            
            let tts_engine: Arc<dyn TtsEngine> = if new_config.tts.provider == "gtts" {
                tracing::info!("using gTTS engine");
                Arc::new(GttsEngine::new())
            } else {
                tracing::info!("using MsEdge engine");
                Arc::new(MsEdgeEngine::new(new_config.tts.clone()))
            };

            *ctx.data().config.write().await = new_config.clone();
            *ctx.data().normalizer.write().await = normalizer;
            *ctx.data().tts_engine.write().await = tts_engine;

            let commands = &ctx.framework().options().commands;
            let _ = poise::builtins::register_globally(ctx.serenity_context(), commands).await;

            tracing::info!("Config and commands reloaded by {}", ctx.author().name);
            
            ctx.send(
                CreateReply::default()
                    .content("✅ Đã tải lại config.yml và cập nhật lệnh thành công!")
                    .ephemeral(true),
            ).await?;
        }
        Err(e) => {
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Lỗi khi đọc config: {}", e))
                    .ephemeral(true),
            ).await?;
        }
    }

    Ok(())
}
