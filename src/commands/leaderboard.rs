use crate::rank::{helpers, service};
use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Xem bảng xếp hạng cấp bậc trong Server
#[poise::command(slash_command, guild_only)]
pub async fn leaderboard(ctx: Context<'_>) -> Result<(), Error> {
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

    let state = ctx.data().rank_store.guild_snapshot(guild_id.get()).await;
    let entries = service::leaderboard(&state);
    if entries.is_empty() {
        ctx.send(
            poise::CreateReply::default()
                .content("Hiện tại chưa có ai trên bảng xếp hạng của server này."),
        )
        .await?;
        return Ok(());
    }

    let lines = entries
        .iter()
        .enumerate()
        .map(|(index, (user_id, level))| {
            let medal = match index {
                0 => "🥇",
                1 => "🥈",
                2 => "🥉",
                _ => "  ",
            };
            format!("{medal} #{} <@{user_id}> (Lv.{level})", index + 1)
        })
        .collect();

    helpers::send_paginated_embed(ctx, "🏆 BẢNG XẾP HẠNG", lines).await
}
