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

    let user_level = UserLevel::of(msg.author.id.get(), &data.config);
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

    let processed = text::prepare_for_tts(&msg.content, &data.normalizer);
    if processed.is_empty() {
        return;
    }

    let text_to_speak = if data.config.tts.max_chars > 0 {
        let limit = data.config.tts.max_chars;
        if processed.len() > limit {
            processed[..limit].to_string()
        } else {
            processed
        }
    } else {
        processed
    };

    let voice = if data.state.is_female(guild_id) {
        &data.config.tts.voice_female
    } else {
        &data.config.tts.voice_male
    };

    let audio_bytes = match data.tts_engine.synthesize(&text_to_speak, voice).await {
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
