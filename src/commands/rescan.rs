use crate::rank::helpers;
use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Quét lại biệt danh tất cả thành viên trong Server để cập nhật/bổ sung cơ sở dữ liệu Rank
#[poise::command(slash_command, guild_only)]
pub async fn rescan(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer().await?;

    if !helpers::check_admin_permission(ctx).await {
        ctx.send(
            poise::CreateReply::default()
                .content("❌ Bạn cần quyền Administrator để dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let config = ctx.data().config.read().await.clone();
    if !config.rank.enabled {
        ctx.send(
            poise::CreateReply::default()
                .content("❌ Hệ thống rank đang tắt.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let guild_id = match ctx.guild_id() {
        Some(id) => id,
        None => {
            ctx.send(
                poise::CreateReply::default()
                    .content("❌ Lệnh này chỉ sử dụng được trong server Discord.")
                    .ephemeral(true),
            )
            .await?;
            return Ok(());
        }
    };

    let mut db = ctx.data().rank_db.write().await;
    let mut total_added = 0;
    let mut total_updated = 0;
    let mut after: Option<serenity::all::UserId> = None;

    loop {
        // Fetch up to 1000 members per batch
        let members = match guild_id.members(ctx, Some(1000), after).await {
            Ok(m) => m,
            Err(e) => {
                ctx.send(
                    poise::CreateReply::default()
                        .content(format!("❌ Gặp lỗi khi tải danh sách thành viên: {}", e)),
                )
                .await?;
                return Ok(());
            }
        };

        if members.is_empty() {
            break;
        }

        for member in &members {
            if member.user.bot {
                continue;
            }

            let uid = member.user.id.get().to_string();
            let nick = member.nick.as_deref().unwrap_or(&member.user.name);

            if let Some(level) = crate::rank::logic::parse_nickname(&config.rank, nick) {
                if let Some(user_data) = db.users.get_mut(&uid) {
                    if user_data.level != level {
                        user_data.level = level;
                        user_data.original_name = nick.to_string();
                        total_updated += 1;
                    }
                } else {
                    db.users.insert(uid, crate::rank::db::RankUserData {
                        level,
                        original_name: nick.to_string(),
                    });
                    total_added += 1;
                }
            }
        }

        after = members.last().map(|m| m.user.id);
        if members.len() < 1000 {
            break;
        }
    }

    let _ = db.save("database.yml");

    ctx.send(
        poise::CreateReply::default()
            .content(format!(
                "✅ Đã quét xong thành viên! Bổ sung: {} người mới. Cập nhật: {} người đổi cấp bậc.",
                total_added, total_updated
            )),
    )
    .await?;

    Ok(())
}
