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


    let current_user_id = ctx.cache.current_user().id;
    if msg.mentions_user_id(current_user_id) {
        let bot_mention_1 = format!("<@{}>", current_user_id);
        let bot_mention_2 = format!("<@!{}>", current_user_id);
        
        if msg.content.contains(&bot_mention_1) || msg.content.contains(&bot_mention_2) {
            let question = msg.content
                .replace(&bot_mention_1, "")
                .replace(&bot_mention_2, "")
                .trim()
                .to_string();
                
            if !question.is_empty() {
                let config = data.config.read().await;
                if config.ai.enabled && !config.ai.api_key.is_empty() {
                    let api_key = config.ai.api_key.clone();
                    let model = config.ai.model.clone();
                    let custom_answers = config.ai.custom_answers.clone();
                    drop(config);
                    
                    let _ = msg.channel_id.broadcast_typing(&ctx.http).await;
                    
                    match crate::ai::ask_gemini(&api_key, &model, &question, &custom_answers).await {
                        Ok(answer) => {
                            if answer.len() > 2000 {
                                let chunks = answer.chars().collect::<Vec<char>>();
                                for chunk in chunks.chunks(1900) {
                                    let s: String = chunk.iter().collect();
                                    let _ = msg.reply(&ctx.http, s).await;
                                }
                            } else {
                                let _ = msg.reply(&ctx.http, answer).await;
                            }
                        }
                        Err(e) => {
                            let _ = msg.reply(&ctx.http, format!("❌ Lỗi AI: {}", e)).await;
                            tracing::error!("AI mention error: {}", e);
                        }
                    }
                    return;
                }
            }
        }
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

    let processed = text::prepare_for_tts(msg, &normalizer);
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

    let text_lower = text_to_speak.trim().to_lowercase();
    let eng_exceptions = ["no", "ok", "yes", "hello", "hi", "bye", "wtf", "gg", "lol", "nice"];
    
    let is_english = if eng_exceptions.contains(&text_lower.as_str()) {
        true
    } else {
        let has_vn_chars = text_lower.chars().any(|c| "áàảãạăắằẳẵặâấầẩẫậéèẻẽẹêếềểễệíìỉĩịóòỏõọôốồổỗộơớờởỡợúùủũụưứừửữựýỳỷỹỵđ".contains(c));
        if has_vn_chars {
            false
        } else {
            let detected_lang = data.language_detector.detect_language_of(&text_to_speak);
            detected_lang == Some(lingua::Language::English)
        }
    };

    let voice = if is_english {
        if data.state.is_female(guild_id) {
            &config.tts.voice_en_female
        } else {
            &config.tts.voice_en_male
        }
    } else {
        if data.state.is_female(guild_id) {
            &config.tts.voice_female
        } else {
            &config.tts.voice_male
        }
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
