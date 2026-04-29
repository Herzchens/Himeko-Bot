use crate::permissions::UserLevel;
use crate::state::VoiceSession;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[poise::command(slash_command, guild_only)]
pub async fn join(ctx: Context<'_>) -> Result<(), Error> {
    let config = &ctx.data().config;
    let state = &ctx.data().state;

    let user_id = ctx.author().id.get();
    let level = UserLevel::of(user_id, config);

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

    let channel_id = {
        let guild = ctx
            .cache()
            .guild(guild_id)
            .ok_or("guild not found in cache")?;
        guild
            .voice_states
            .get(&ctx.author().id)
            .and_then(|vs| vs.channel_id)
    };

    let channel_id = match channel_id {
        Some(id) => id,
        None => {
            ctx.send(
                CreateReply::default()
                    .content("❌ Bạn phải vào voice channel trước.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    if let Some(session) = state.get_session(guild_id) {
        if !level.can_preempt() && session.owner_level >= level {
            ctx.send(
                CreateReply::default()
                    .content("❌ Bot đang bận phục vụ người khác. Vui lòng đợi.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
        tracing::info!(
            guild = %guild_id,
            old_owner = %session.owner,
            new_owner = %ctx.author().id,
            "session preempted"
        );
    }

    let manager = songbird::get(ctx.serenity_context())
        .await
        .ok_or("songbird not registered")?;

    let _handler = manager
        .join(guild_id, channel_id)
        .await
        .map_err(|e| format!("failed to join voice channel: {}", e))?;

    state.set_session(
        guild_id,
        VoiceSession {
            owner: ctx.author().id,
            owner_level: level,
        },
    );

    let channel_name = ctx
        .cache()
        .guild(guild_id)
        .and_then(|g| g.channels.get(&channel_id).map(|c| c.name.clone()))
        .unwrap_or_else(|| "voice channel".to_string());

    tracing::info!(
        guild = %guild_id,
        channel = %channel_id,
        user = %ctx.author().id,
        "joined voice channel"
    );

    ctx.send(
        CreateReply::default()
            .content(format!("✅ Đã vào **{}**", channel_name))
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
