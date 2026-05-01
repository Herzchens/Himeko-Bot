use crate::rank::{db::RankUserData, helpers, logic};
use crate::Data;
use serenity::all::UserId;

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
pub async fn up(
    ctx: Context<'_>,
    #[description = "Tag những người cần tăng cấp (vd: @A @B)"] users: String,
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
        let member = match guild_id.member(http, user_id).await {
            Ok(m) => m,
            Err(_) => {
                response_lines.push(format!("❌ <@{}> không nằm trong server.", user_id));
                continue;
            }
        };

        if member.user.bot {
            continue;
        }

        let uid_str = user_id.get().to_string();
        let current_nick = member.nick.as_deref().unwrap_or(&member.user.name).to_string();

        let assessment = helpers::assess_member(&http, guild_id, &member, rank_config.target_role_id, bot_user_id).await?;

        let user_data = db.users.entry(uid_str).or_insert_with(|| RankUserData {
            level: 0,
            original_name: current_nick,
        });

        let max_lvl = rank_config.max_level();
        let mut note = assessment.note;

        if user_data.level >= max_lvl {
            note = Some("đã đạt cấp tối đa".to_string());
        } else {
            user_data.level += 1;
        }

        let new_level = user_data.level;
        let expected_nick = logic::format_nickname(&rank_config, new_level)?;

        if assessment.can_rename && new_level > 0 {
            let _ = helpers::apply_nickname(&http, guild_id, user_id, &expected_nick).await;
        }

        let role_note = if assessment.role_added { " [đã gắn role]" } else { "" };
        let note_str = note.map(|n| format!(" [{}]", n)).unwrap_or_default();

        let icon = if note_str.contains("tối đa") {
            "⛔"
        } else if assessment.can_rename {
            "✅"
        } else {
            "⚠️"
        };

        response_lines.push(format!("{} <@{}> → {} (Lv.{}){}{}", icon, user_id, expected_nick, new_level, note_str, role_note));
    }

    let _ = db.save("database.yml");

    let embed = serenity::all::CreateEmbed::new()
        .title("📈 TĂNG CẤP")
        .description(response_lines.join("\n"));

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}
