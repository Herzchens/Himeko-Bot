use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Đặt tốc độ đọc riêng cho tài khoản của bạn
#[poise::command(slash_command, guild_only)]
pub async fn speed(
    ctx: Context<'_>,
    #[description = "Tốc độ đọc từ 0.25x đến 2.0x"]
    #[min = 0.25]
    #[max = 2.0]
    rate: f64,
) -> Result<(), Error> {
    let config = ctx.data().config_snapshot().await;
    let level = UserLevel::of(ctx.author().id.get(), &config);
    drop(config);

    if !level.can_use_tts() {
        ctx.send(
            CreateReply::default()
                .content("❌ Bạn không có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    if !rate.is_finite() || !(0.25..=2.0).contains(&rate) {
        ctx.send(
            CreateReply::default()
                .content("❌ Tốc độ phải nằm trong khoảng 0.25x đến 2.0x.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let speed = rate as f32;
    ctx.data().state.set_speech_speed(ctx.author().id, speed);

    let message = if (speed - 1.0).abs() < f32::EPSILON {
        "🎙️ Đã đặt tốc độ đọc về mặc định 1.00x.".to_string()
    } else {
        format!("🎙️ Đã đặt tốc độ đọc của bạn thành {speed:.2}x.")
    };

    tracing::info!(
        user = %ctx.author().id,
        speed,
        "per-user speech speed changed"
    );

    ctx.send(CreateReply::default().content(message).ephemeral(true))
        .await?;
    Ok(())
}
