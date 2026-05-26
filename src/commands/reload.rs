use crate::config::Config;
use crate::permissions::UserLevel;
use crate::text::normalizer::Normalizer;
use crate::tts::engine::MsEdgeEngine;
use crate::tts::gtts::GttsEngine;
use crate::tts::supertonic::SupertonicEngine;
use crate::tts::openai::OpenAiEngine;
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
            
            let tts_engine: Arc<dyn TtsEngine> = match new_config.tts.provider.as_str() {
                "gtts" => {
                    tracing::info!("using gTTS engine");
                    Arc::new(GttsEngine::new())
                }
                "supertonic" => {
                    match new_config.tts.get_supertonic_config() {
                        Some(st_cfg) => {
                            tracing::info!(server = %st_cfg.server_url, "using Supertonic engine");
                            Arc::new(SupertonicEngine::new(st_cfg))
                        }
                        None => {
                            ctx.send(
                                CreateReply::default()
                                    .content("❌ Config thiếu section [tts.supertonic] khi provider = \"supertonic\"")
                                    .ephemeral(true),
                            ).await?;
                            return Ok(());
                        }
                    }
                }
                "openai" => {
                    match new_config.tts.get_openai_config() {
                        Some(oa_cfg) => {
                            tracing::info!(url = %oa_cfg.api_url, model = %oa_cfg.model, "using OpenAI-compatible engine");
                            Arc::new(OpenAiEngine::new(oa_cfg))
                        }
                        None => {
                            ctx.send(
                                CreateReply::default()
                                    .content("❌ Config thiếu section [tts.openai] khi provider = \"openai\"")
                                    .ephemeral(true),
                            ).await?;
                            return Ok(());
                        }
                    }
                }
                _ => {
                    tracing::info!("using MsEdge engine");
                    Arc::new(MsEdgeEngine::new(new_config.tts.clone()))
                }
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
