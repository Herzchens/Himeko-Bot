use super::owner_config::{self, AllowedUserMutation};
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Thêm một người vào danh sách được phép sử dụng bot
#[poise::command(slash_command)]
pub async fn add(
    ctx: Context<'_>,
    #[description = "Discord user ID hoặc @mention"]
    #[rename = "user"]
    target: String,
) -> Result<(), Error> {
    if !owner_config::require_owner(ctx).await? {
        return Ok(());
    }

    let Some(user_id) = parse_user_id(&target) else {
        ctx.send(
            CreateReply::default()
                .content("❌ User không hợp lệ. Hãy nhập Discord user ID hoặc @mention.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    };

    let config = ctx.data().config_snapshot().await;
    if user_id == config.permissions.owner_id {
        ctx.send(
            CreateReply::default()
                .content(
                    "ℹ️ Bot owner luôn có quyền sử dụng bot, không cần thêm vào allowed_users.",
                )
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    drop(config);

    match owner_config::add_allowed_user(ctx.data(), ctx.author().id.get(), user_id).await {
        Ok(AllowedUserMutation::Added) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "✅ Đã thêm <@{user_id}> (`{user_id}`) vào allowed_users và áp dụng ngay."
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(AllowedUserMutation::AlreadyPresent) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "ℹ️ <@{user_id}> (`{user_id}`) đã có trong allowed_users."
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(other) => {
            tracing::warn!(?other, user_id, "unexpected add-user mutation result");
            ctx.send(
                CreateReply::default()
                    .content("❌ Không thể thêm user do trạng thái config không mong đợi.")
                    .ephemeral(true),
            )
            .await?;
        }
        Err(error) => {
            tracing::error!(user_id, %error, "failed to add allowed user");
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Không thể cập nhật allowed_users: {error}"))
                    .ephemeral(true),
            )
            .await?;
        }
    }

    Ok(())
}

/// Xóa một người khỏi danh sách được phép sử dụng bot
#[poise::command(slash_command)]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "Discord user ID hoặc @mention"]
    #[rename = "user"]
    target: String,
) -> Result<(), Error> {
    if !owner_config::require_owner(ctx).await? {
        return Ok(());
    }

    let Some(user_id) = parse_user_id(&target) else {
        ctx.send(
            CreateReply::default()
                .content("❌ User không hợp lệ. Hãy nhập Discord user ID hoặc @mention.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    };

    let config = ctx.data().config_snapshot().await;
    if user_id == config.permissions.owner_id {
        ctx.send(
            CreateReply::default()
                .content("❌ Không thể xóa quyền của bot owner.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    drop(config);

    match owner_config::remove_allowed_user(ctx.data(), ctx.author().id.get(), user_id).await {
        Ok(AllowedUserMutation::Removed) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "✅ Đã xóa <@{user_id}> (`{user_id}`) khỏi allowed_users và áp dụng ngay."
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(AllowedUserMutation::NotFound) => {
            ctx.send(
                CreateReply::default()
                    .content(format!(
                        "ℹ️ <@{user_id}> (`{user_id}`) không có trong allowed_users."
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
        Ok(other) => {
            tracing::warn!(?other, user_id, "unexpected remove-user mutation result");
            ctx.send(
                CreateReply::default()
                    .content("❌ Không thể xóa user do trạng thái config không mong đợi.")
                    .ephemeral(true),
            )
            .await?;
        }
        Err(error) => {
            tracing::error!(user_id, %error, "failed to remove allowed user");
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Không thể cập nhật allowed_users: {error}"))
                    .ephemeral(true),
            )
            .await?;
        }
    }

    Ok(())
}

fn parse_user_id(input: &str) -> Option<u64> {
    let input = input.trim();
    let raw = if let Some(mention) = input
        .strip_prefix("<@")
        .and_then(|value| value.strip_suffix('>'))
    {
        mention.strip_prefix('!').unwrap_or(mention)
    } else {
        input
    };

    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse::<u64>().ok().filter(|user_id| *user_id != 0)
}

#[cfg(test)]
mod tests {
    use super::parse_user_id;

    #[test]
    fn parses_plain_id_and_mentions() {
        assert_eq!(parse_user_id("123456789"), Some(123456789));
        assert_eq!(parse_user_id("<@123456789>"), Some(123456789));
        assert_eq!(parse_user_id("<@!123456789>"), Some(123456789));
    }

    #[test]
    fn rejects_invalid_user_targets() {
        assert_eq!(parse_user_id(""), None);
        assert_eq!(parse_user_id("0"), None);
        assert_eq!(parse_user_id("<@&123>"), None);
        assert_eq!(parse_user_id("user123"), None);
    }
}
