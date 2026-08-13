use crate::rank::{helpers, service};
use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Bật/Tắt tự động đổi tên theo cấp bậc cho server hiện tại (Cần quyền Admin)
#[poise::command(slash_command, guild_only)]
pub async fn autorename(
    ctx: Context<'_>,
    #[description = "Bật hoặc tắt autorename (on/off)"] state: String,
) -> Result<(), Error> {
    if !helpers::check_admin_permission(ctx).await {
        ctx.send(
            poise::CreateReply::default()
                .content("❌ Bạn cần quyền Administrator để dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    let (guild_id, _rank_config) = match helpers::guild_rank_config(ctx).await {
        Ok(value) => value,
        Err(_) => {
            ctx.send(
                poise::CreateReply::default()
                    .content("❌ Rank chưa được cấu hình cho server này.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    let enabled = match state.to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => {
            ctx.send(
                poise::CreateReply::default()
                    .content("❌ Vui lòng nhập 'on' hoặc 'off'.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    if let Err(error) =
        service::set_autorename(&ctx.data().rank_store, guild_id.get(), enabled).await
    {
        tracing::error!(guild = %guild_id, %error, "failed to persist autorename setting");
        ctx.send(
            poise::CreateReply::default()
                .content(format!("❌ Không thể lưu cấu hình autorename: {error}"))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    ctx.send(
        poise::CreateReply::default()
            .content(format!(
                "✅ Đã {} Auto-Rename cho server này.",
                if enabled { "BẬT" } else { "TẮT" }
            ))
            .ephemeral(true),
    )
    .await?;
    Ok(())
}
