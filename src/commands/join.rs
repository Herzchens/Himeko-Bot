use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Cho bot vào voice channel hiện tại của bạn
#[poise::command(slash_command, guild_only)]
pub async fn join(ctx: Context<'_>) -> Result<(), Error> {
    let config = ctx.data().config_snapshot().await;
    let level = UserLevel::of(ctx.author().id.get(), &config);

    if !level.can_join() {
        ctx.send(
            CreateReply::default()
                .content("❌ Bạn không có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let guild_id = match ctx.guild_id() {
        Some(id) => id,
        None => {
            ctx.send(
                CreateReply::default()
                    .content("❌ Lệnh này chỉ dùng được trong server.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    let channel_id = ctx.guild().and_then(|guild| {
        guild
            .voice_states
            .get(&ctx.author().id)
            .and_then(|state| state.channel_id)
    });

    let channel_id = match channel_id {
        Some(id) => id,
        None => {
            ctx.send(
                CreateReply::default()
                    .content("❌ Bạn cần vào voice channel trước.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    let state = &ctx.data().state;
    let queue_lock = state.get_queue_lock(guild_id);
    let _operation = queue_lock.lock().await;

    let existing_session = state.get_session(guild_id);
    let old_generation = existing_session.as_ref().map(|session| session.generation);
    let mut joined_new_session = false;
    if let Some(session) = &existing_session {
        if !level.can_control_session(ctx.author().id.get(), session.owner.get()) {
            ctx.send(
                CreateReply::default()
                    .content("❌ Bot đang phục vụ người khác. Chỉ session owner hoặc bot owner mới có thể chuyển phòng.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    }

    let manager = match songbird::get(ctx.serenity_context()).await {
        Some(manager) => manager,
        None => {
            ctx.send(
                CreateReply::default()
                    .content("❌ Songbird chưa được khởi tạo.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    let had_call_before = manager.get(guild_id).is_some();
    if let Some(handler_lock) = manager.get(guild_id) {
        let handler = handler_lock.lock().await;
        handler.queue().stop();
    }

    match manager.join(guild_id, channel_id).await {
        Ok(_) => {
            let session = state.begin_session(guild_id, ctx.author().id, channel_id);
            joined_new_session = true;
            tracing::info!(
                guild = %guild_id,
                channel = %channel_id,
                owner = %ctx.author().id,
                generation = session.generation,
                "voice session started"
            );
            ctx.send(
                CreateReply::default()
                    .content(format!("✅ Đã vào voice channel <#{channel_id}>."))
                    .ephemeral(true),
            )
            .await?;
        }
        Err(error) => {
            let mut cleanup_error = None;
            if !had_call_before {
                if let Err(cleanup) = manager.remove(guild_id).await {
                    tracing::error!(
                        guild = %guild_id,
                        error = %cleanup,
                        "failed to clean Songbird Call after join failure"
                    );
                    cleanup_error = Some(cleanup.to_string());
                }
                if let Some(session) = &existing_session {
                    state.clear_session_if_generation(guild_id, session.generation);
                }
            }

            tracing::error!(guild = %guild_id, channel = %channel_id, error = %error, "failed to join voice channel");
            let detail = cleanup_error
                .map(|cleanup| format!("{error}; cleanup failed: {cleanup}"))
                .unwrap_or_else(|| error.to_string());
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Không thể vào voice channel: {detail}"))
                    .ephemeral(true),
            )
            .await?;
        }
    }

    drop(_operation);
    if joined_new_session {
        if let Some(generation) = old_generation {
            ctx.data()
                .tts_scheduler
                .cancel_generation(guild_id, generation)
                .await;
        }
    }

    Ok(())
}
