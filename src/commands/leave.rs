use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Yêu cầu bot rời khỏi kênh thoại
#[poise::command(slash_command, guild_only)]
pub async fn leave(ctx: Context<'_>) -> Result<(), Error> {
    let config = ctx.data().config.read().await;
    let state = &ctx.data().state;

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

    let guild_id = ctx.guild_id().ok_or("command must be used in a guild")?;



    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("songbird not registered")?;

    let _ = manager.remove(guild_id).await;

    state.clear_session(guild_id);

    tracing::info!(
        guild = %guild_id,
        user = %ctx.author().id,
        "left voice channel"
    );

    ctx.send(
        CreateReply::default()
            .content("✅ Đã rời kênh")
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
