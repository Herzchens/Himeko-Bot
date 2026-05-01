use crate::rank::helpers;
use crate::Data;
use serenity::all::{RoleId, UserId};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

fn extract_mentions(text: &str) -> Vec<UserId> {
    let mut mentions = Vec::new();
    let re = regex::Regex::new(r"<@!?(\d+)>").unwrap();
    for cap in re.captures_iter(text) {
        if let Ok(id) = cap[1].parse::<u64>() {
            mentions.push(UserId::new(id));
        }
    }
    mentions
}

#[poise::command(slash_command)]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "Tag những người cần gỡ cấp (vd: @A @B)"] users: String,
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
    ctx.defer().await?;

    let config_lock = ctx.data().config.read().await;
    if !config_lock.rank.enabled {
        ctx.send(poise::CreateReply::default().content("❌ Hệ thống rank đang tắt.").ephemeral(true)).await?;
        return Ok(());
    }
    let rank_config = config_lock.rank.clone();
    drop(config_lock);

    let mentions = extract_mentions(&users);
    if mentions.is_empty() {
        ctx.send(poise::CreateReply::default().content("❌ Không tìm thấy ai được tag.").ephemeral(true)).await?;
        return Ok(());
    }

    let guild_id = ctx.guild_id().unwrap();
    let bot_user_id = ctx.cache().current_user().id;
    let http = ctx.http();

    let mut response_lines = Vec::new();

    let mut db = ctx.data().rank_db.write().await;

    for user_id in mentions {
        let member_res = guild_id.member(http, user_id).await;
        if member_res.is_err() {
            response_lines.push(format!("❌ <@{}> không nằm trong server.", user_id));
            continue;
        }
        let member = member_res.unwrap();
        if member.user.bot {
            continue;
        }

        let uid_str = user_id.get().to_string();

        let assessment = helpers::assess_member(&http, guild_id, &member, rank_config.target_role_id, bot_user_id).await?;

        if !db.users.contains_key(&uid_str) {
            response_lines.push(format!("⛔ <@{}> chưa có cấp bậc.", user_id));
            continue;
        }

        let original_name = db.users.remove(&uid_str).map(|u| u.original_name).unwrap_or_else(|| member.user.name.clone());

        if assessment.can_rename {
            let _ = helpers::apply_nickname(&http, guild_id, user_id, &original_name).await;
        }

        if member.roles.contains(&RoleId::new(rank_config.target_role_id)) {
            let _ = guild_id.member(http, user_id).await.unwrap().remove_role(http, RoleId::new(rank_config.target_role_id)).await;
        }

        let icon = if assessment.can_rename { "✅" } else { "⚠️" };

        response_lines.push(format!("{} <@{}> → Đã gỡ cấp bậc (trả tên cũ)", icon, user_id));
    }

    let _ = db.save("database.yml");

    let embed = serenity::all::CreateEmbed::new()
        .title("🗑 GỠ CẤP")
        .description(response_lines.join("\n"));

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}
