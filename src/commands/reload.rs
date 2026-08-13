use crate::config::Config;
use crate::permissions::UserLevel;
use crate::text::normalizer::Normalizer;
use crate::tts::engine::MsEdgeEngine;
use crate::tts::gtts::GttsEngine;
use crate::tts::openai::OpenAiEngine;
use crate::tts::supertonic::SupertonicEngine;
use crate::tts::vieneu::VieneuEngine;
use crate::tts::TtsEngine;
use crate::{Data, RuntimeSnapshot};
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
                SupertonicEngine::new(st_cfg)
                    .await
                    .map(|engine| Arc::new(engine) as Arc<dyn TtsEngine>)
                    .map_err(|error| format!("Supertonic initialization failed: {error}"))
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

fn legacy_rank_enable_requires_restart(
    current_rank_enabled: bool,
    new_rank_enabled: bool,
    legacy_pending: bool,
) -> bool {
    !current_rank_enabled && new_rank_enabled && legacy_pending
}

/// Tải lại các cấu hình runtime-safe bằng một immutable snapshot duy nhất.
#[poise::command(slash_command, guild_only)]
pub async fn reload(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    // Serialize the full reload transaction, not just pointer publication. Otherwise an
    // older slow engine build can complete after a newer reload and restore stale state.
    let _reload_transaction = ctx.data().runtime.begin_reload().await;
    let current_runtime = ctx.data().runtime_snapshot().await;
    let current_config = Arc::clone(&current_runtime.config);
    let level = UserLevel::of(ctx.author().id.get(), &current_config);

    if !level.can_preempt() {
        ctx.send(
            CreateReply::default()
                .content("❌ Chỉ owner mới có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let new_config = match Config::load("config.yml") {
        Ok(config) => config,
        Err(error) => {
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Lỗi khi đọc config: {error}"))
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    if legacy_rank_enable_requires_restart(
        current_config.rank.enabled,
        new_config.rank.enabled,
        ctx.data().rank_store.legacy_migration_pending(),
    ) {
        ctx.send(
            CreateReply::default()
                .content(
                    "❌ Không thể bật Rank bằng /reload khi database.yml legacy đang chờ migrate. Hãy khởi động lại bot với cấu hình Rank đã bật để migration chạy an toàn.",
                )
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    if let Err(error) = new_config.validate_hot_reload_from(&current_config) {
        ctx.send(
            CreateReply::default()
                .content(format!(
                    "❌ Không thể hot-reload thay đổi này: {error}. Hãy restart bot để áp dụng."
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    // Build every fallible replacement before publishing any new runtime state.
    let tts_engine = match create_engine(&new_config).await {
        Ok(engine) => engine,
        Err(error) => {
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ {error}"))
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };
    let normalizer = Arc::new(Normalizer::from_config(&new_config.abbreviations));
    let default_female = new_config.tts.default_gender == "female";
    let new_runtime = Arc::new(RuntimeSnapshot {
        config: Arc::new(new_config),
        normalizer,
        tts_engine,
        default_female,
    });

    // A single pointer publication makes config, normalizer, engine and default gender one generation.
    ctx.data().publish_runtime(Arc::clone(&new_runtime)).await;
    // Keep the legacy BotState default synchronized for non-TTS callers; the TTS path uses the snapshot.
    ctx.data().state.set_default_female(default_female);

    tracing::info!("Config hot-reloaded by {}", ctx.author().name);
    ctx.send(
        CreateReply::default()
            .content("✅ Đã hot-reload các cấu hình runtime an toàn.")
            .ephemeral(true),
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod reload_tests {
    use super::legacy_rank_enable_requires_restart;

    #[test]
    fn legacy_pending_only_special_cases_disabled_to_enabled_rank_transition() {
        assert!(legacy_rank_enable_requires_restart(false, true, true));
        assert!(!legacy_rank_enable_requires_restart(true, true, true));
        assert!(!legacy_rank_enable_requires_restart(false, false, true));
        assert!(!legacy_rank_enable_requires_restart(false, true, false));
    }
}
