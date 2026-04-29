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
    let state = &ctx.data().state;

    let level = UserLevel::of(ctx.author().id.get(), &config);
    if !level.can_use_tts() {
        ctx.send(
            CreateReply::default()
                .content("❌ Bạn không có quyền dùng lệnh này.")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let guild_id = ctx.guild_id().ok_or("command must be used in a guild")?;
    let is_female = matches!(voice, GenderChoice::Female);

    state.set_gender(guild_id, is_female);

    let (emoji, voice_label, name) = if is_female {
        ("👩", "female", &config.tts.voice_female)
    } else {
        ("👨", "male", &config.tts.voice_male)
    };

    tracing::info!(
        guild = %guild_id,
        gender = %voice_label,
        "voice gender changed"
    );

    ctx.send(
        CreateReply::default()
            .content(format!(
                "🎙️ {} Đã chuyển sang giọng {} ({})",
                emoji, voice_label, name
            ))
            .ephemeral(true),
    )
    .await?;

    Ok(())
}
