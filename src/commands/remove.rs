use crate::rank::{helpers, service};
use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Gỡ bỏ hoàn toàn cấp bậc của thành viên (Cần quyền Admin)
#[poise::command(slash_command, guild_only)]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "Tag những người cần gỡ cấp"] users: String,
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
    let mentions = helpers::extract_mentions(&users);
    if mentions.is_empty() {
        ctx.send(
            poise::CreateReply::default()
                .content("❌ Không tìm thấy ai được tag.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    ctx.defer().await?;
    let remote = service::SerenityRankRemote::new(ctx.http(), ctx.cache().current_user().id);
    let mut lines = Vec::new();
    for user_id in mentions {
        match service::remove_rank(
            &ctx.data().rank_store,
            &rank_config,
            guild_id.get(),
            user_id.get(),
            &remote,
        )
        .await
        {
            Ok(service::RankChange::Changed {
                nickname_managed, ..
            }) => {
                let icon = if nickname_managed { "✅" } else { "⚠️" };
                let warning = if nickname_managed {
                    ""
                } else {
                    " — không thể đổi nickname do hierarchy"
                };
                lines.push(format!("{icon} <@{user_id}> → Đã gỡ cấp bậc{warning}"));
            }
            Ok(service::RankChange::NotRanked) => {
                lines.push(format!("⛔ <@{user_id}> chưa có cấp bậc."));
            }
            Ok(service::RankChange::SkippedBot) => {
                lines.push(format!("⏭️ Bỏ qua bot <@{user_id}>."));
            }
            Ok(service::RankChange::AlreadyMaximum { .. }) => {
                lines.push(format!("❌ <@{user_id}>: trạng thái rank không hợp lệ."));
            }
            Err(error) => lines.push(format!("❌ <@{user_id}>: {error}")),
        }
    }

    helpers::send_paginated_embed(ctx, "🗑 GỠ CẤP", lines).await
}
