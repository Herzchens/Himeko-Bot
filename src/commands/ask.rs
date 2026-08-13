use crate::permissions::UserLevel;
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
    let level = UserLevel::of(ctx.author().id.get(), &config);
    if !level.can_use_ai() {
        drop(config);
        ctx.send(
            CreateReply::default()
                .content("❌ Bạn không có quyền dùng AI.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    if !config.ai.enabled {
        drop(config);
        ctx.send(
            CreateReply::default()
                .content("❌ Tính năng AI chưa được cấu hình (chưa có API Key).")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let (provider, api_key, model) = config.ai.resolve();
    let api_key = api_key.to_string();
    let model = model.to_string();
    let custom_answers = config.ai.custom_answers.clone();
    let use_search = config.ai.google_search;
    drop(config);

    if api_key.is_empty() {
        ctx.send(
            CreateReply::default()
                .content(format!(
                    "❌ API Key cho provider '{provider:?}' chưa được cấu hình."
                ))
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    ctx.defer().await?;

    let ai_result = crate::ai::ask_ai(
        provider,
        &api_key,
        &model,
        &question,
        &custom_answers,
        use_search,
    )
    .await;

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
        Err(error) => {
            ctx.send(
                CreateReply::default()
                    .content(format!("❌ Lỗi khi gọi AI: {error}"))
                    .ephemeral(true),
            )
            .await?;
        }
    }
    Ok(())
}
