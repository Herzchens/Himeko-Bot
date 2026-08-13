use crate::rank::{helpers, service};
use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Quét lại biệt danh thành viên để đồng bộ dữ liệu Rank của server hiện tại
#[poise::command(slash_command, guild_only)]
pub async fn rescan(ctx: Context<'_>) -> Result<(), Error> {
    if !helpers::check_admin_permission(ctx).await {
        ctx.send(
            poise::CreateReply::default()
                .content("❌ Bạn cần quyền Administrator để dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }
    let (guild_id, rank_config) = match helpers::guild_rank_config(ctx).await {
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

    ctx.defer().await?;
    let remote = service::SerenityRankRemote::new(ctx.http(), ctx.cache().current_user().id);
    match service::rescan(
        &ctx.data().rank_store,
        &rank_config,
        guild_id.get(),
        &remote,
    )
    .await
    {
        Ok(report) => {
            ctx.send(poise::CreateReply::default().content(format!(
                "✅ Quét xong: +{} mới, {} cập nhật cấp, {} mục cũ/đã rời server được loại bỏ.",
                report.added, report.updated, report.removed
            )))
            .await?;
        }
        Err(error) => {
            tracing::error!(guild = %guild_id, %error, "rank rescan failed; previous database state preserved");
            ctx.send(
                poise::CreateReply::default()
                    .content(format!(
                        "❌ Quét thất bại, dữ liệu cũ được giữ nguyên: {error}"
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
    }
    Ok(())
}
