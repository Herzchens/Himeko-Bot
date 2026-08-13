use crate::config::Config;
use crate::permissions::UserLevel;
use crate::text::normalizer::Normalizer;
use crate::tts::engine::MsEdgeEngine;
use crate::tts::gtts::GttsEngine;
use crate::tts::openai::OpenAiEngine;
use crate::tts::supertonic::SupertonicEngine;
use crate::tts::vieneu::VieneuEngine;
use crate::tts::TtsEngine;
use crate::Data;
use poise::CreateReply;
use std::sync::Arc;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

async fn create_engine(config: &Config) -> Result<Arc<dyn TtsEngine>, String> {
    match config.tts.provider.as_str() {
        "gtts" => {
            tracing::info!("using gTTS engine");
            Ok(Arc::new(GttsEngine::new()))
        }
        "supertonic" => match config.tts.get_supertonic_config() {
            Some(st_cfg) => {
                tracing::info!(server = %st_cfg.server_url, "using Supertonic engine");
                Ok(Arc::new(SupertonicEngine::new(st_cfg)))
            }
            None => Err(
                "Config thiếu section [tts.supertonic] khi provider = \"supertonic\"".to_string(),
            ),
        },
        "openai" => match config.tts.get_openai_config() {
            Some(oa_cfg) => {
                tracing::info!(url = %oa_cfg.api_url, model = %oa_cfg.model, "using OpenAI-compatible engine");
                Ok(Arc::new(OpenAiEngine::new(oa_cfg)))
            }
            None => Err("Config thiếu section [tts.openai] khi provider = \"openai\"".to_string()),
        },
        "vieneu" => match config.tts.get_vieneu_config() {
            Some(vn_cfg) => {
                tracing::info!(server = %vn_cfg.server_url, "using VieNeu-TTS engine");
                VieneuEngine::new(vn_cfg)
                    .await
                    .map(|engine| Arc::new(engine) as Arc<dyn TtsEngine>)
                    .map_err(|error| format!("VieNeu-TTS initialization failed: {error}"))
            }
            None => Err("Config thiếu section [tts.vieneu] khi provider = \"vieneu\"".to_string()),
        },
        "msedge" => {
            tracing::info!("using MsEdge engine");
            Ok(Arc::new(MsEdgeEngine::new(config.tts.clone())))
        }
        other => Err(format!(
            "Unsupported TTS provider after validation: {other}"
        )),
    }
}

fn rank_reload_requires_restart(rank_enabled: bool, legacy_pending: bool) -> bool {
    rank_enabled && legacy_pending
}

/// Tải lại file config và cập nhật bot
#[poise::command(slash_command, guild_only)]
pub async fn reload(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
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
        Ok(new_config)
            if rank_reload_requires_restart(
                new_config.rank.enabled,
                ctx.data().rank_store.legacy_migration_pending(),
            ) =>
        {
            ctx.send(
                CreateReply::default()
                    .content(
                        "❌ Không thể bật Rank bằng /reload khi database.yml legacy đang chờ migrate. Hãy khởi động lại bot với cấu hình Rank đã bật để migration chạy an toàn.",
                    )
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(new_config) => match create_engine(&new_config).await {
            Ok(tts_engine) => {
                let normalizer = Arc::new(Normalizer::from_config(&new_config.abbreviations));
                let default_female = new_config.tts.default_gender == "female";
                *ctx.data().config.write().await = Arc::new(new_config);
                *ctx.data().normalizer.write().await = normalizer;
                *ctx.data().tts_engine.write().await = tts_engine;
                ctx.data().state.set_default_female(default_female);

                tracing::info!("Config reloaded by {}", ctx.author().name);
                ctx.send(
                    CreateReply::default()
                        .content("✅ Đã tải lại config.yml và cập nhật cấu hình thành công!")
                        .ephemeral(true),
                )
                .await?;
            }
            Err(err_msg) => {
                ctx.send(
                    CreateReply::default()
                        .content(format!("❌ {err_msg}"))
                        .ephemeral(true),
                )
                .await?;
            }
        },
        Err(e) => {
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Lỗi khi đọc config: {e}"))
                    .ephemeral(true),
            )
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod rank_reload_tests {
    use super::rank_reload_requires_restart;

    #[test]
    fn legacy_pending_only_requires_restart_when_reload_enables_rank() {
        assert!(rank_reload_requires_restart(true, true));
        assert!(!rank_reload_requires_restart(false, true));
        assert!(!rank_reload_requires_restart(true, false));
        assert!(!rank_reload_requires_restart(false, false));
    }
}
