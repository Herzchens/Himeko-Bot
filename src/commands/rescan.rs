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
    match service::reconcile_guild(
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
            tracing::error!(
                guild = %guild_id,
                %error,
                "rank reconciliation failed; scheduled rank work remains inactive"
            );
            ctx.send(
                poise::CreateReply::default()
                    .content(format!(
                        "❌ Đồng bộ Rank chưa hoàn tất; tác vụ Rank định kỳ sẽ tạm dừng cho tới khi đồng bộ thành công: {error}"
                    ))
                    .ephemeral(true),
            )
            .await?;
        }
    }
    Ok(())
}
