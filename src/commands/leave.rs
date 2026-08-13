use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Cho bot rời voice channel hiện tại
#[poise::command(slash_command, guild_only)]
pub async fn leave(ctx: Context<'_>) -> Result<(), Error> {
    let config = ctx.data().config.read().await.clone();
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

    let Some(guild_id) = ctx.guild_id() else {
        return Ok(());
    };

    let state = &ctx.data().state;
    let queue_lock = state.get_queue_lock(guild_id);
    let _operation = queue_lock.lock().await;

    let Some(session) = state.get_session(guild_id) else {
        ctx.send(
            CreateReply::default()
                .content("ℹ️ Bot không có voice session đang hoạt động.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    };

    if !level.can_control_session(ctx.author().id.get(), session.owner.get()) {
        ctx.send(
            CreateReply::default()
                .content("❌ Chỉ session owner hoặc bot owner mới có thể cho bot rời phòng.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let Some(manager) = songbird::get(ctx.serenity_context()).await else {
        ctx.send(
            CreateReply::default()
                .content("❌ Songbird chưa được khởi tạo.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    };

    if let Some(handler_lock) = manager.get(guild_id) {
        {
            let handler = handler_lock.lock().await;
            handler.queue().stop();
        }

        if let Err(error) = manager.remove(guild_id).await {
            tracing::error!(guild = %guild_id, error = %error, "failed to leave voice channel");
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Không thể rời voice channel: {error}"))
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    } else {
        tracing::warn!(
            guild = %guild_id,
            generation = session.generation,
            "voice session existed without a Songbird Call; cleaning stale state"
        );
    }

    let cleared = state.clear_session_if_generation(guild_id, session.generation);
    drop(_operation);
    if cleared {
        ctx.data()
            .tts_scheduler
            .cancel_generation(guild_id, session.generation)
            .await;
    }
    tracing::info!(
        guild = %guild_id,
        generation = session.generation,
        "voice session ended"
    );

    ctx.send(
        CreateReply::default()
            .content("✅ Đã rời voice channel.")
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
