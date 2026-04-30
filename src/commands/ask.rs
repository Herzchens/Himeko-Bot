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
    if !config.ai.enabled {
        ctx.send(
            CreateReply::default()
                .content("❌ Tính năng AI chưa được cấu hình (chưa có API Key).")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let provider = config.ai.provider.clone().to_lowercase();
    let api_key = if provider == "groq" { config.ai.groq_api_key.clone() } else { config.ai.api_key.clone() };
    let model = if provider == "groq" { config.ai.groq_model.clone() } else { config.ai.model.clone() };
    let custom_answers = config.ai.custom_answers.clone();
    let use_search = config.ai.google_search;
    drop(config);

    if api_key.is_empty() {
        ctx.send(
            CreateReply::default()
                .content(format!("❌ API Key cho provider '{}' chưa được cấu hình.", provider))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    ctx.defer().await?;

    let ai_result = if provider == "groq" {
        crate::ai::ask_groq(&api_key, &model, &question, &custom_answers).await
    } else {
        crate::ai::ask_gemini(&api_key, &model, &question, &custom_answers, use_search).await
    };

    match ai_result {
        Ok(answer) => {

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
