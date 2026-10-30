use super::owner_config;
use crate::Data;
use poise::CreateReply;
use serenity::all::GuildId;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Cho bot rời một server theo guild ID
#[poise::command(slash_command)]
pub async fn leaveguild(
    ctx: Context<'_>,
    #[description = "Guild ID của server cần rời"] guild_id: String,
) -> Result<(), Error> {
    if !owner_config::require_owner(ctx).await? {
        return Ok(());
    }

    let Some(raw_guild_id) = parse_guild_id(&guild_id) else {
        ctx.send(
            CreateReply::default()
                .content("❌ Guild ID không hợp lệ.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    };
    let guild_id = GuildId::new(raw_guild_id);

    let cache = &ctx.serenity_context().cache;
    if !cache.guilds().contains(&guild_id) {
        ctx.send(
            CreateReply::default()
                .content(format!(
                    "❌ Bot không thấy guild `{raw_guild_id}` trong runtime cache. Dùng `/info` để lấy ID hiện tại."
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let guild_name = cache
        .guild(guild_id)
        .map(|guild| guild.name.clone())
        .unwrap_or_else(|| "(uncached)".to_string());

    ctx.defer_ephemeral().await?;

    if let Some(session) = ctx.data().state.get_session(guild_id) {
        if let Some(manager) = songbird::get(ctx.serenity_context()).await {
            if let Err(error) = manager.remove(guild_id).await {
                tracing::warn!(
                    guild = %guild_id,
                    %error,
                    "failed to remove voice call before leaving guild"
                );
            }
        }
        if ctx
            .data()
            .state
            .clear_session_if_generation(guild_id, session.generation)
        {
            ctx.data()
                .tts_scheduler
                .cancel_generation(guild_id, session.generation)
                .await;
        }
    }

    match guild_id.leave(&ctx.serenity_context().http).await {
        Ok(()) => {
            tracing::info!(
                actor = %ctx.author().id,
                guild = %guild_id,
                guild_name = %guild_name,
                "owner made bot leave guild"
            );
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "✅ Đã rời server **{}** (`{}`).",
                        sanitize_guild_name(&guild_name),
                        raw_guild_id
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Err(error) => {
            tracing::error!(
                actor = %ctx.author().id,
                guild = %guild_id,
                guild_name = %guild_name,
                %error,
                "failed to leave guild"
            );
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "❌ Không thể rời server **{}** (`{}`): {error}",
                        sanitize_guild_name(&guild_name),
                        raw_guild_id
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
    }

    Ok(())
}

fn parse_guild_id(input: &str) -> Option<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    trimmed.parse::<u64>().ok().filter(|id| *id != 0)
}

fn sanitize_guild_name(name: &str) -> String {
    name.replace(['\r', '\n'], " ").replace('`', "ˋ")
}

#[cfg(test)]
mod tests {
    use super::parse_guild_id;

    #[test]
    fn parses_nonzero_numeric_guild_ids() {
        assert_eq!(parse_guild_id("123456789"), Some(123456789));
        assert_eq!(parse_guild_id(" 123456789 "), Some(123456789));
    }

    #[test]
    fn rejects_invalid_guild_ids() {
        assert_eq!(parse_guild_id(""), None);
        assert_eq!(parse_guild_id("0"), None);
        assert_eq!(parse_guild_id("guild-123"), None);
    }
}
