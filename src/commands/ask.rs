use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Hỏi AI một câu hỏi
#[poise::command(slash_command)]
pub async fn ask(
    ctx: Context<'_>,
    #[description = "Câu hỏi của bạn"] question: String,
) -> Result<(), Error> {
    let config = ctx.data().config.read().await;
    if !config.ai.enabled || config.ai.api_key.is_empty() {
        ctx.send(
            CreateReply::default()
                .content("❌ Tính năng AI chưa được cấu hình (chưa có API Key).")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let api_key = config.ai.api_key.clone();
    let model = config.ai.model.clone();
    drop(config);

    ctx.defer().await?;

    match crate::ai::ask_gemini(&api_key, &model, &question).await {
        Ok(answer) => {
            // Split answer if it's too long (Discord limits to 2000 chars)
            if answer.len() > 2000 {
                let chunks = answer.chars().collect::<Vec<char>>();
                for chunk in chunks.chunks(1900) {
                    let s: String = chunk.iter().collect();
                    ctx.send(CreateReply::default().content(s)).await?;
                }
            } else {
                ctx.send(CreateReply::default().content(answer)).await?;
            }
        }
        Err(e) => {
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Lỗi khi gọi AI: {}", e))
                    .ephemeral(true),
            )
            .await?;
        }
    }
    Ok(())
}
