use crate::permissions::UserLevel;
use crate::text;
use crate::Data;
use serenity::all::FullEvent;

pub async fn handle_message(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
    data: &Data,
) {
    if msg.author.bot {
        return;
    }
    if msg.content.starts_with('/') {
        return;
    }

    let guild_id = match msg.guild_id {
        Some(id) => id,
        None => return,
    };

    let config = data.config.read().await;
    let normalizer = data.normalizer.read().await;

    let user_level = UserLevel::of(msg.author.id.get(), &config);
    if !user_level.can_use_tts() {
        return;
    }

    if data.state.is_idle(guild_id) {
        return;
    }

    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => {
            tracing::error!("songbird not registered — check Client::register_songbird()");
            return;
        }
    };

    if manager.get(guild_id).is_none() {
        return;
    }

    let processed = text::prepare_for_tts(&msg.content, &normalizer);
    if processed.is_empty() {
        return;
    }

    let text_to_speak = if config.tts.max_chars > 0 {
        let limit = config.tts.max_chars;
        if processed.len() > limit {
            processed[..limit].to_string()
        } else {
            processed
        }
    } else {
        processed
    };

    let voice = if data.state.is_female(guild_id) {
        &config.tts.voice_female
    } else {
        &config.tts.voice_male
    };

    let tts_engine = data.tts_engine.read().await;
    let audio_bytes = match tts_engine.synthesize(&text_to_speak, voice).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(
                guild = %guild_id,
                user = %msg.author.id,
                error = %e,
                "TTS synthesis failed"
            );
            return;
        }
    };

    tracing::info!(
        guild = %guild_id,
        user = %msg.author.id,
        chars = text_to_speak.len(),
        voice = %voice,
        audio_bytes = audio_bytes.len(),
        "TTS synthesized"
    );

    if audio_bytes.is_empty() {
        tracing::error!("TTS engine returned 0 bytes! (Check if the voice name is correct or if text is empty)");
        return;
    }

    if let Some(handler_lock) = manager.get(guild_id) {
        let input = songbird::input::Input::from(audio_bytes);
        let mut handler = handler_lock.lock().await;
        handler.enqueue_input(input).await;
    }
}

pub async fn event_handler(
    ctx: &serenity::client::Context,
    event: &FullEvent,
    _framework: poise::FrameworkContext<'_, Data, Box<dyn std::error::Error + Send + Sync>>,
    data: &Data,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let FullEvent::Message { new_message } = event {
        handle_message(ctx, new_message, data).await;
    }
    Ok(())
}
