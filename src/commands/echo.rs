use crate::Data;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Gửi tin nhắn dưới tên của Bot vào kênh hiện tại
#[poise::command(slash_command)]
pub async fn echo(
    ctx: Context<'_>,
    #[description = "Nội dung tin nhắn muốn Bot nói"] message: String,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;

    // Log user details and the echoed message to console
    tracing::info!(
        user = %ctx.author().name,
        user_id = %ctx.author().id.get(),
        channel_id = %ctx.channel_id().get(),
        message = %message,
        "Echo command executed"
    );

    // Send message to the current channel under the Bot's account
    ctx.channel_id().say(ctx, &message).await?;

    // Send ephemeral confirmation back to the caller
    ctx.send(
        poise::CreateReply::default()
            .content("✅ Đã gửi tin nhắn dưới tên Bot thành công!")
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
