use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[poise::command(slash_command, guild_only)]
pub async fn leave(ctx: Context<'_>) -> Result<(), Error> {
    let config = &ctx.data().config;
    let state = &ctx.data().state;

    let level = UserLevel::of(ctx.author().id.get(), config);
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

    if state.is_idle(guild_id) {
        ctx.send(
            CreateReply::default()
                .content("❌ Bot chưa ở trong voice channel nào.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("songbird not registered")?;

    manager
        .leave(guild_id)
        .await
        .map_err(|e| format!("failed to leave voice channel: {}", e))?;

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
