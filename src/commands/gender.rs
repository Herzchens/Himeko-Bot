use crate::permissions::UserLevel;
use crate::Data;
use poise::CreateReply;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[derive(Debug, poise::ChoiceParameter)]
pub enum GenderChoice {
    #[name = "male"]
    Male,
    #[name = "female"]
    Female,
}

/// Đổi giọng đọc (Nam/Nữ)
#[poise::command(slash_command, guild_only)]
pub async fn gender(
    ctx: Context<'_>,
    #[description = "Giọng đọc: male hoặc female"] voice: GenderChoice,
) -> Result<(), Error> {
    let config = ctx.data().config.read().await;
    let level = UserLevel::of(ctx.author().id.get(), &config);
    if !level.can_use_tts() {
        drop(config);
        ctx.send(
            CreateReply::default()
                .content("❌ Bạn không có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    if config.tts.provider == "gtts" {
        drop(config);
        ctx.send(
            CreateReply::default()
                .content("ℹ️ gTTS không hỗ trợ lựa chọn giọng nam/nữ. Bot sẽ chỉ chọn ngôn ngữ vi/en cho provider này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let guild_id = ctx
        .guild_id()
        .unwrap_or(serenity::model::id::GuildId::new(1));
    let user_id = ctx.author().id;
    let is_female = matches!(voice, GenderChoice::Female);
    let name = config.tts.get_active_voice(is_female);
    drop(config);

    ctx.data().state.set_gender(user_id, is_female);

    let (emoji, voice_label) = if is_female {
        ("👩", "female")
    } else {
        ("👨", "male")
    };

    tracing::info!(
        guild = %guild_id,
        gender = %voice_label,
        "voice gender changed"
    );

    ctx.send(
        CreateReply::default()
            .content(format!(
                "🎙️ {emoji} Đã chuyển sang giọng {voice_label} ({name})"
            ))
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
