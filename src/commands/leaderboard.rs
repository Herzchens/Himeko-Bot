
use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[poise::command(slash_command)]
pub async fn leaderboard(ctx: Context<'_>) -> Result<(), Error> {
    let config_lock = ctx.data().config.read().await;
    if !config_lock.rank.enabled {
        ctx.send(poise::CreateReply::default().content("❌ Hệ thống rank đang tắt.").ephemeral(true)).await?;
        return Ok(());
    }
    drop(config_lock);

    let db = ctx.data().rank_db.read().await;

    let mut ranked_users: Vec<_> = db
        .users
        .iter()
        .filter(|(_, u)| u.level > 0)
        .collect();

    if ranked_users.is_empty() {
        ctx.send(poise::CreateReply::default().content("Hiện tại chưa có ai trên bảng xếp hạng.")).await?;
        return Ok(());
    }

    // Sort by level DESC, tie-break UID ASC
    ranked_users.sort_by(|(id_a, u_a), (id_b, u_b)| {
        u_b.level.cmp(&u_a.level).then_with(|| id_a.cmp(id_b))
    });

    let mut lines = Vec::new();
    for (i, (uid, user_data)) in ranked_users.iter().enumerate() {
        let medal = match i {
            0 => "🥇",
            1 => "🥈",
            2 => "🥉",
            _ => "  ",
        };

        lines.push(format!("{} #{} <@{}> (Lv.{})", medal, i + 1, uid, user_data.level));
    }

    let embed = serenity::all::CreateEmbed::new()
        .title("🏆 BẢNG XẾP HẠNG")
        .description(lines.join("\n"));

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}
