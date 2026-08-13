use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Gửi tin nhắn dưới tên của Bot vào kênh hiện tại
#[poise::command(slash_command)]
pub async fn echo(
    ctx: Context<'_>,
    #[description = "Nội dung tin nhắn muốn Bot nói"] message: String,
) -> Result<(), Error> {
    let config = ctx.data().config_snapshot().await;
    let level = UserLevel::of(ctx.author().id.get(), &config);
    drop(config);

    if !level.can_echo() {
        ctx.send(
            CreateReply::default()
                .content("❌ Bạn không có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    ctx.defer_ephemeral().await?;

    tracing::info!(
        user = %ctx.author().name,
        user_id = %ctx.author().id.get(),
        channel_id = %ctx.channel_id().get(),
        message = %message,
        "Echo command executed"
    );

    ctx.channel_id().say(ctx, &message).await?;

    ctx.send(
        CreateReply::default()
            .content("✅ Đã gửi tin nhắn dưới tên Bot thành công!")
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
