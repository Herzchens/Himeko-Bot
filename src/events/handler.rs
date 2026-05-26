use crate::permissions::UserLevel;
use crate::text;
use crate::Data;
use serenity::all::FullEvent;

async fn handle_ai_mention(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
    data: &Data,
) -> bool {
    let current_user_id = ctx.cache.current_user().id;
    if !msg.mentions_user_id(current_user_id) {
        return false;
    }

    let bot_mention_1 = format!("<@{}>", current_user_id);
    let bot_mention_2 = format!("<@!{}>", current_user_id);
    
    if !msg.content.contains(&bot_mention_1) && !msg.content.contains(&bot_mention_2) {
        return false;
    }

    let question = msg.content
        .replace(&bot_mention_1, "")
        .replace(&bot_mention_2, "")
        .trim()
        .to_string();
        
    if question.is_empty() {
        return false;
    }

    let config = data.config.read().await;
    if !config.ai.enabled {
        return false;
    }

    let (provider, api_key, model) = config.ai.resolve();
    let api_key = api_key.to_string();
    let model = model.to_string();
    let custom_answers = config.ai.custom_answers.clone();
    let use_search = config.ai.google_search;
    drop(config);
    
    if api_key.is_empty() {
        let _ = msg.reply(&ctx.http, format!("❌ API Key cho provider '{:?}' chưa được cấu hình.", provider)).await;
        return true;
    }
    
    let _ = msg.channel_id.broadcast_typing(&ctx.http).await;
    
    tracing::info!(user = %msg.author.id, question = %question, "Processing AI request");
    
    let ai_result = crate::ai::ask_ai(provider, &api_key, &model, &question, &custom_answers, use_search).await;
    
    match ai_result {
        Ok(answer) => {
            tracing::info!(user = %msg.author.id, answer_len = answer.len(), "AI request successful");
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
    true
}

fn detect_language(text: &str, text_to_speak: &str, detector: &lingua::LanguageDetector) -> bool {
    let eng_exceptions = ["no", "ok", "yes", "hello", "hi", "bye", "wtf", "gg", "lol", "nice"];
    if eng_exceptions.contains(&text) {
        return true;
    }
    let has_vn_chars = text.chars().any(|c| "áàảãạăắằẳẵặâấầẩẫậéèẻẽẹêếềểễệíìỉĩịóòỏõọôốồổỗộơớờởỡợúùủũụưứừửữựýỳỷỹỵđ".contains(c));
    if has_vn_chars {
        return false;
    }
    let detected_lang = detector.detect_language_of(text_to_speak);
    detected_lang == Some(lingua::Language::English)
}

fn select_voice(
    config: &crate::config::Config,
    is_english: bool,
    is_female: bool,
) -> String {
    if is_english {
        if is_female {
            config.tts.get_msedge_voice("en_female")
        } else {
            config.tts.get_msedge_voice("en_male")
        }
    } else if is_female {
        config.tts.get_msedge_voice("female")
    } else {
        config.tts.get_msedge_voice("male")
    }
}

fn resolve_log_voice<'a>(config: &'a crate::config::Config, voice: &'a str) -> &'a str {
    match config.tts.provider.as_str() {
        "gtts" => {
            if voice.starts_with("en-") || voice == "en" {
                "en"
            } else {
                "vi"
            }
        }
        "supertonic" => {
            if let Some(ref list) = config.tts.supertonic {
                if voice.contains('-') {
                    let lower = voice.to_lowercase();
                    let is_male = lower.contains("nam")
                        || lower.contains("guy")
                        || lower.contains("male")
                            && !lower.contains("female");
                    let key = if is_male { "male" } else { "female" };
                    for map in list {
                        if let Some(val) = map.get(key) {
                            if let Some(s) = val.as_str() {
                                return s;
                            }
                        }
                    }
                }
                voice
            } else {
                voice
            }
        }
        "openai" => {
            if let Some(ref list) = config.tts.openai {
                if voice.contains('-') {
                    let lower = voice.to_lowercase();
                    let is_male = lower.contains("nam")
                        || lower.contains("guy")
                        || lower.contains("male")
                            && !lower.contains("female");
                    let key = if is_male { "male" } else { "female" };
                    for map in list {
                        if let Some(val) = map.get(key) {
                            if let Some(s) = val.as_str() {
                                return s;
                            }
                        }
                    }
                }
                voice
            } else {
                voice
            }
        }
        "vieneu" => {
            if let Some(ref list) = config.tts.vieneu {
                if voice.contains('-') {
                    let lower = voice.to_lowercase();
                    let is_male = lower.contains("nam")
                        || lower.contains("guy")
                        || lower.contains("male")
                            && !lower.contains("female");
                    let key = if is_male { "male" } else { "female" };
                    for map in list {
                        if let Some(val) = map.get(key) {
                            if let Some(s) = val.as_str() {
                                return s;
                            }
                        }
                    }
                }
                voice
            } else {
                voice
            }
        }
        _ => voice,
    }
}

pub async fn handle_message(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
    data: &Data,
) {
    if msg.author.bot || msg.content.starts_with('/') {
        return;
    }

    if handle_ai_mention(ctx, msg, data).await {
        return;
    }

    let guild_id = match msg.guild_id {
        Some(id) => id,
        None => return,
    };

    let config = data.config.read().await;
    let user_level = UserLevel::of(msg.author.id.get(), &config);
    if !user_level.can_use_tts() || data.state.is_idle(guild_id) {
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

    let normalizer = data.normalizer.read().await;
    let processed = text::prepare_for_tts(ctx, msg, &normalizer).await;
    if processed.is_empty() {
        return;
    }

    let text_to_speak = if config.tts.max_chars > 0 && processed.len() > config.tts.max_chars {
        processed[..config.tts.max_chars].to_string()
    } else {
        processed
    };

    let is_english = detect_language(&text_to_speak.trim().to_lowercase(), &text_to_speak, &data.language_detector);
    let is_female = data.state.is_female(msg.author.id);
    let voice = select_voice(&config, is_english, is_female);

    let tts_engine = data.tts_engine.read().await;
    
    let queue_lock = data.state.get_queue_lock(guild_id);
    let _guard = queue_lock.lock().await;

    let start_time = std::time::Instant::now();
    let audio_bytes = match tts_engine.synthesize(&text_to_speak, &voice).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(guild = %guild_id, user = %msg.author.id, error = %e, "TTS synthesis failed");
            return;
        }
    };
    let elapsed_ms = start_time.elapsed().as_millis();

    let log_voice = resolve_log_voice(&config, &voice);
    tracing::info!(
        guild = %guild_id,
        user = %msg.author.id,
        provider = %config.tts.provider,
        chars = text_to_speak.len(),
        voice = %log_voice,
        audio_bytes = audio_bytes.len(),
        elapsed_ms = elapsed_ms,
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

async fn attempt_rejoin(
    ctx: &serenity::client::Context,
    guild_id: serenity::model::id::GuildId,
    channel_id: serenity::model::id::ChannelId,
    data: &Data,
) {
    let manager = match songbird::get(ctx).await {
        Some(m) => m,
        None => return,
    };

    for attempt in 1..=3u32 {
        let delay = std::time::Duration::from_secs(1 << (attempt - 1));
        tokio::time::sleep(delay).await;

        match manager.join(guild_id, channel_id).await {
            Ok(_) => {
                tracing::info!(guild = %guild_id, channel = %channel_id, attempt, "auto-rejoin succeeded");
                return;
            }
            Err(e) => {
                tracing::warn!(guild = %guild_id, attempt, error = %e, "auto-rejoin failed");
            }
        }
    }

    tracing::error!(guild = %guild_id, "auto-rejoin exhausted all attempts, clearing session");
    data.state.clear_session(guild_id);
}

async fn handle_voice_state_update(
    ctx: &serenity::client::Context,
    new: &serenity::model::voice::VoiceState,
    data: &Data,
) {
    let bot_id = ctx.cache.current_user().id;
    if new.user_id == bot_id && new.channel_id.is_none() {
        if let Some(guild_id) = new.guild_id {
            if let Some(session) = data.state.get_session(guild_id) {
                tracing::warn!(guild = %guild_id, "bot disconnected from voice, attempting rejoin");
                attempt_rejoin(ctx, guild_id, session.channel_id, data).await;
            }
        }
    }
}

pub async fn event_handler(
    ctx: &serenity::client::Context,
    event: &FullEvent,
    _framework: poise::FrameworkContext<'_, Data, Box<dyn std::error::Error + Send + Sync>>,
    data: &Data,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match event {
        FullEvent::Message { new_message } => {
            handle_message(ctx, new_message, data).await;
        }
        FullEvent::GuildMemberUpdate {
            old_if_available,
            new,
            event: event_data,
        } => {
            crate::events::member_update::handle_member_update(
                ctx, old_if_available, new, event_data, data,
            ).await;
        }
        FullEvent::VoiceStateUpdate { new, .. } => {
            handle_voice_state_update(ctx, new, data).await;
        }
        _ => {}
    }
    Ok(())
}
