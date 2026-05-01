use crate::rank::helpers;
use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Bật/Tắt tính năng tự động đổi tên theo cấp bậc (Cần quyền Admin)
#[poise::command(slash_command)]
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

    let is_on = match state.to_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => {
            ctx.send(poise::CreateReply::default().content("❌ Vui lòng nhập 'on' hoặc 'off'.").ephemeral(true)).await?;
            return Ok(());
        }
    };

    let mut db = ctx.data().rank_db.write().await;
    db.settings.autorename = is_on;
    let _ = db.save("database.yml");

    ctx.send(
        poise::CreateReply::default()
            .content(format!("✅ Đã {} tính năng tự động đổi tên (Auto-Rename).", if is_on { "BẬT" } else { "TẮT" }))
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
