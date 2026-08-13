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

    let bot_mention_1 = format!("<@{current_user_id}>");
    let bot_mention_2 = format!("<@!{current_user_id}>");

    if !msg.content.contains(&bot_mention_1) && !msg.content.contains(&bot_mention_2) {
        return false;
    }

    let question = msg
        .content
        .replace(&bot_mention_1, "")
        .replace(&bot_mention_2, "")
        .trim()
        .to_string();

    if question.is_empty() {
        return false;
    }

    let config = data.config.read().await;
    let level = UserLevel::of(msg.author.id.get(), &config);
    if !level.can_use_ai() {
        drop(config);
        let _ = msg.reply(&ctx.http, "❌ Bạn không có quyền dùng AI.").await;
        return true;
    }
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
        let _ = msg
            .reply(
                &ctx.http,
                format!("❌ API Key cho provider '{provider:?}' chưa được cấu hình."),
            )
            .await;
        return true;
    }

    let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

    tracing::info!(user = %msg.author.id, question = %question, "Processing AI request");

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
            let _ = msg.reply(&ctx.http, format!("❌ Lỗi AI: {e}")).await;
            tracing::error!("AI mention error: {}", e);
        }
    }
    true
}

fn detect_language(text: &str, text_to_speak: &str, detector: &lingua::LanguageDetector) -> bool {
    let eng_exceptions = [
        "no", "ok", "yes", "hello", "hi", "bye", "wtf", "gg", "lol", "nice",
    ];
    if eng_exceptions.contains(&text) {
        return true;
    }
    let has_vn_chars = text
        .chars()
        .any(|c| "áàảãạăắằẳẵặâấầẩẫậéèẻẽẹêếềểễệíìỉĩịóòỏõọôốồổỗộơớờởỡợúùủũụưứừửữựýỳỷỹỵđ".contains(c));
    if has_vn_chars {
        return false;
    }
    let detected_lang = detector.detect_language_of(text_to_speak);
    detected_lang == Some(lingua::Language::English)
}

fn select_voice(config: &crate::config::Config, is_english: bool, is_female: bool) -> String {
    if config.tts.provider == "gtts" {
        return if is_english { "en" } else { "vi" }.to_string();
    }

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
                        || lower.contains("male") && !lower.contains("female");
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
                        || lower.contains("male") && !lower.contains("female");
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
                        || lower.contains("male") && !lower.contains("female");
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

async fn handle_owner_dm(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
    data: &Data,
) {
    let content = msg.content.trim();
    if content.is_empty() {
        return;
    }

    let via = if msg.guild_id.is_none() {
        "DM"
    } else {
        "control channel"
    };

    let active_chan_id = data
        .state
        .active_console_channel
        .load(std::sync::atomic::Ordering::SeqCst);

    if content.starts_with("/channel ") || content.starts_with(":channel ") {
        let id_str = content.split_whitespace().nth(1).unwrap_or("");
        if let Some(new_id) = id_str.parse::<u64>().ok().filter(|id| *id != 0) {
            data.state
                .active_console_channel
                .store(new_id, std::sync::atomic::Ordering::SeqCst);
            let _ = msg
                .reply(
                    ctx,
                    format!("✅ Đã chuyển kênh console chat sang: <#{new_id}>"),
                )
                .await;
            tracing::info!(
                new_channel_id = new_id,
                "Active console chat channel set via DM"
            );
        } else {
            let _ = msg
                .reply(ctx, "❌ ID kênh không hợp lệ. Cú pháp: `/channel <ID>`")
                .await;
        }
        return;
    }

    if content.starts_with("/reply ")
        || content.starts_with("/r ")
        || content.starts_with(":reply ")
        || content.starts_with(":r ")
    {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() >= 3 {
            if let Ok(idx) = parts[1].parse::<usize>() {
                if let Some(reference) = data.state.recent_message(idx) {
                    let reply_text = parts[2..].join(" ");
                    let chan = reference.channel_id;
                    let msg_ref = serenity::all::CreateMessage::new()
                        .content(&reply_text)
                        .reference_message((chan, reference.message_id));
                    match chan.send_message(&ctx.http, msg_ref).await {
                        Ok(_) => {
                            let _ = msg.react(ctx, '✅').await;
                            tracing::info!(channel = %chan, message = %reply_text, reply_to = %reference.message_id, "Sent reply via {}", via);
                        }
                        Err(e) => {
                            let _ = msg
                                .reply(ctx, format!("❌ Gửi tin nhắn trả lời thất bại: {e}"))
                                .await;
                        }
                    }
                    return;
                }
            }
        }
        let _ = msg
            .reply(
                ctx,
                "❌ Cú pháp không hợp lệ. Cú pháp: `/r <1-10> <nội dung>`",
            )
            .await;
        return;
    }

    if active_chan_id == 0 {
        let _ = msg
            .reply(
                ctx,
                "❌ Chưa chọn kênh chat. Vui lòng dùng lệnh `/channel <ID>` trước.",
            )
            .await;
        return;
    }

    let chan = serenity::all::ChannelId::new(active_chan_id);
    match chan.say(&ctx.http, content).await {
        Ok(_) => {
            let _ = msg.react(ctx, '✅').await;
            tracing::info!(channel = active_chan_id, message = %content, "Sent message via {}", via);
        }
        Err(e) => {
            let _ = msg
                .reply(ctx, format!("❌ Gửi tin nhắn thất bại: {e}"))
                .await;
        }
    }
}

pub async fn handle_message(
    ctx: &serenity::client::Context,
    msg: &serenity::model::channel::Message,
    data: &Data,
) {
    if msg.author.bot {
        return;
    }

    let config = data.config.read().await.clone();

    // Intercept DMs or control channel messages from the owner
    let is_control_channel = config.logging.control_channel_id != 0
        && msg.channel_id.get() == config.logging.control_channel_id;

    if (msg.guild_id.is_none() || is_control_channel)
        && msg.author.id.get() == config.permissions.owner_id
    {
        handle_owner_dm(ctx, msg, data).await;
        return;
    }

    if msg.content.starts_with('/') {
        return;
    }

    let active_chan = data
        .state
        .active_console_channel
        .load(std::sync::atomic::Ordering::SeqCst);
    if active_chan != 0 && msg.channel_id.get() == active_chan {
        let idx = data.state.record_recent_message(msg.channel_id, msg.id);
        println!("[{}] {}: {}", idx + 1, msg.author.name, msg.content);
        tracing::info!(target: "himeko_bot::console", "[{}] {}: {}", idx + 1, msg.author.name, msg.content);
    }

    if handle_ai_mention(ctx, msg, data).await {
        return;
    }

    let guild_id = match msg.guild_id {
        Some(id) => id,
        None => return,
    };

    let config = data.config.read().await.clone();
    let user_level = UserLevel::of(msg.author.id.get(), &config);
    if !user_level.can_use_tts() {
        return;
    }
    let Some(session) = data.state.get_session(guild_id) else {
        return;
    };

    let Some(ticket) = data.tts_scheduler.try_admit(guild_id, session.generation) else {
        tracing::warn!(
            guild = %guild_id,
            user = %msg.author.id,
            generation = session.generation,
            "TTS admission limit reached; dropping message to preserve bounded latency"
        );
        return;
    };
    let sequence = ticket.sequence();

    let manager = match songbird::get(ctx).await {
        Some(manager) => manager,
        None => {
            tracing::error!("songbird not registered — check Client::register_songbird()");
            if let Some(mut emission) = ticket.complete(None).await {
                while emission.next_ready().await.is_some() {}
            }
            return;
        }
    };

    let audio_outcome: Option<Vec<Vec<u8>>> = async {
        manager.get(guild_id)?;

        let filtered = text::prepare_for_tts(ctx, msg).await;
        if filtered.is_empty() {
            return None;
        }

        let is_english = detect_language(
            &filtered.trim().to_lowercase(),
            &filtered,
            &data.language_detector,
        );
        let normalizer = data.normalizer.read().await.clone();
        let processed = normalizer.expand_for_language(&filtered, is_english);
        if processed.is_empty() {
            return None;
        }

        let processed_graphemes =
            match crate::tts::validate_admission_limit(&processed, config.tts.max_chars) {
                Ok(graphemes) => graphemes,
                Err(error) => {
                    tracing::warn!(
                        guild = %guild_id,
                        user = %msg.author.id,
                        sequence,
                        error = %error,
                        "TTS message rejected by configured admission limit"
                    );
                    return None;
                }
            };

        let is_female = data.state.is_female(msg.author.id);
        let voice = select_voice(&config, is_english, is_female);
        let tts_engine = data.tts_engine.read().await.clone();
        let synthesis_permit = ticket.acquire_synthesis(&data.tts_scheduler).await?;

        let start_time = std::time::Instant::now();
        let audio_chunks = match tts_engine.synthesize_chunks(&processed, &voice).await {
            Ok(chunks) if !chunks.is_empty() && chunks.iter().all(|bytes| !bytes.is_empty()) => {
                chunks
            }
            Ok(_) => {
                tracing::warn!(
                    guild = %guild_id,
                    user = %msg.author.id,
                    sequence,
                    "TTS engine returned empty audio chunks"
                );
                return None;
            }
            Err(error) => {
                tracing::warn!(
                    guild = %guild_id,
                    user = %msg.author.id,
                    sequence,
                    error = %error,
                    "TTS synthesis failed"
                );
                return None;
            }
        };
        drop(synthesis_permit);

        if !data.state.is_current_session(guild_id, session.generation) {
            tracing::debug!(
                guild = %guild_id,
                generation = session.generation,
                sequence,
                "discarding TTS synthesized for a stale voice session"
            );
            return None;
        }

        let total_audio_bytes: usize = audio_chunks.iter().map(Vec::len).sum();
        let log_voice = resolve_log_voice(&config, &voice);
        tracing::info!(
            guild = %guild_id,
            user = %msg.author.id,
            provider = %config.tts.provider,
            chars = processed.chars().count(),
            graphemes = processed_graphemes,
            audio_chunks = audio_chunks.len(),
            sequence,
            voice = %log_voice,
            audio_bytes = total_audio_bytes,
            elapsed_ms = start_time.elapsed().as_millis(),
            "TTS synthesized"
        );
        Some(audio_chunks)
    }
    .await;

    let Some(mut emission) = ticket.complete(audio_outcome).await else {
        return;
    };

    while let Some(ready) = emission.next_ready().await {
        tracing::debug!(
            guild = %guild_id,
            generation = session.generation,
            sequence = ready.sequence,
            chunks = ready.audio_chunks.len(),
            "emitting synthesized TTS in admission order"
        );

        for audio_bytes in ready.audio_chunks {
            let Some(playback_permit) = emission.acquire_playback().await else {
                return;
            };

            let queue_lock = data.state.get_queue_lock(guild_id);
            let _operation = queue_lock.lock().await;
            if !data.state.is_current_session(guild_id, session.generation) {
                return;
            }

            let Some(handler_lock) = manager.get(guild_id) else {
                return;
            };
            let track =
                crate::tts::scheduler::track_with_playback_permit(audio_bytes, playback_permit);
            let mut handler = handler_lock.lock().await;
            handler.enqueue(track).await;
        }
    }
}

async fn attempt_rejoin(
    ctx: &serenity::client::Context,
    guild_id: serenity::model::id::GuildId,
    session: crate::state::VoiceSession,
    data: &Data,
) {
    let manager = match songbird::get(ctx).await {
        Some(manager) => manager,
        None => return,
    };

    for attempt in 1..=3u32 {
        let delay = std::time::Duration::from_secs(1 << (attempt - 1));
        tokio::time::sleep(delay).await;

        if !data.state.is_current_session(guild_id, session.generation) {
            tracing::debug!(
                guild = %guild_id,
                generation = session.generation,
                "aborting stale auto-rejoin task"
            );
            return;
        }

        let queue_lock = data.state.get_queue_lock(guild_id);
        let _operation = queue_lock.lock().await;
        let Some(current) = data.state.get_session(guild_id) else {
            return;
        };
        if current.generation != session.generation {
            return;
        }

        match manager.join(guild_id, current.channel_id).await {
            Ok(_) => {
                tracing::info!(
                    guild = %guild_id,
                    channel = %current.channel_id,
                    generation = current.generation,
                    attempt,
                    "auto-rejoin succeeded"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    guild = %guild_id,
                    generation = current.generation,
                    attempt,
                    error = %error,
                    "auto-rejoin failed"
                );
            }
        }
    }

    let queue_lock = data.state.get_queue_lock(guild_id);
    let _operation = queue_lock.lock().await;
    if !data.state.is_current_session(guild_id, session.generation) {
        return;
    }

    tracing::error!(
        guild = %guild_id,
        generation = session.generation,
        "auto-rejoin exhausted all attempts"
    );

    if let Some(handler_lock) = manager.get(guild_id) {
        {
            let handler = handler_lock.lock().await;
            handler.queue().stop();
        }
        if let Err(error) = manager.remove(guild_id).await {
            tracing::error!(
                guild = %guild_id,
                generation = session.generation,
                error = %error,
                "failed to release Songbird Call after rejoin exhaustion"
            );
            return;
        }
    }

    let cleared = data
        .state
        .clear_session_if_generation(guild_id, session.generation);
    drop(_operation);
    if cleared {
        data.tts_scheduler
            .cancel_generation(guild_id, session.generation)
            .await;
    }
}

async fn handle_voice_state_update(
    ctx: &serenity::client::Context,
    new: &serenity::model::voice::VoiceState,
    data: &Data,
) {
    let bot_id = ctx.cache.current_user().id;
    if new.user_id != bot_id {
        return;
    }

    let Some(guild_id) = new.guild_id else {
        return;
    };

    if let Some(channel_id) = new.channel_id {
        if data.state.update_session_channel(guild_id, channel_id) {
            tracing::debug!(
                guild = %guild_id,
                channel = %channel_id,
                "updated voice session after bot channel move"
            );
        }
        return;
    }

    if let Some(session) = data.state.get_session(guild_id) {
        tracing::warn!(
            guild = %guild_id,
            generation = session.generation,
            "bot disconnected from voice, attempting rejoin"
        );
        attempt_rejoin(ctx, guild_id, session, data).await;
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
                ctx,
                old_if_available,
                new,
                event_data,
                data,
            )
            .await;
        }
        FullEvent::GuildCreate { guild, .. } => {
            let guild_id = guild.id;
            let rank_config = {
                let config = data.config.read().await;
                config.rank.guild_config(guild_id.get())
            };
            let should_reconcile = data
                .rank_store
                .runtime_guild_needs_reconciliation(guild_id.get());
            if let Some(rank_config) = rank_config.filter(|_| should_reconcile) {
                let remote = crate::rank::service::SerenityRankRemote::new(
                    ctx.http.as_ref(),
                    ctx.cache.current_user().id,
                );
                match crate::rank::service::reconcile_guild(
                    &data.rank_store,
                    &rank_config,
                    guild_id.get(),
                    &remote,
                )
                .await
                {
                    Ok(report) => tracing::info!(
                        guild = %guild_id,
                        added = report.added,
                        updated = report.updated,
                        removed = report.removed,
                        "guild rank reconciliation complete"
                    ),
                    Err(error) => tracing::error!(
                        guild = %guild_id,
                        %error,
                        "guild rank reconciliation failed; scheduled rank work stays inactive"
                    ),
                }
            }
        }
        FullEvent::GuildMemberRemoval { guild_id, user, .. } => {
            if let Err(error) = crate::rank::service::remove_departed_user(
                &data.rank_store,
                guild_id.get(),
                user.id.get(),
            )
            .await
            {
                tracing::error!(
                    guild = %guild_id,
                    user = %user.id,
                    %error,
                    "failed to remove departed user from guild rank database"
                );
            }
        }
        FullEvent::GuildDelete { incomplete, .. } => {
            let guild_id = incomplete.id;
            let rank_generation = data
                .rank_store
                .invalidate_runtime_guild_guarded(guild_id.get())
                .await;
            if !incomplete.unavailable {
                let old_session = data.state.get_session(guild_id);
                if let Some(session) = old_session {
                    let queue_lock = data.state.get_queue_lock(guild_id);
                    let operation = queue_lock.lock().await;
                    let cleared = data
                        .state
                        .clear_session_if_generation(guild_id, session.generation);
                    drop(operation);
                    if cleared {
                        data.tts_scheduler
                            .cancel_generation(guild_id, session.generation)
                            .await;
                    }
                }

                if let Some(manager) = songbird::get(ctx).await {
                    if let Some(handler_lock) = manager.get(guild_id) {
                        let handler = handler_lock.lock().await;
                        handler.queue().stop();
                    }
                    if let Err(error) = manager.remove(guild_id).await {
                        tracing::warn!(
                            guild = %guild_id,
                            %error,
                            "failed to release Songbird call after permanent guild removal"
                        );
                    }
                }
                data.rank_store
                    .clear_runtime_guild(guild_id.get(), rank_generation);
                tracing::info!(guild = %guild_id, "cleaned runtime state after permanent guild removal");
            }
        }
        FullEvent::VoiceStateUpdate { new, .. } => {
            handle_voice_state_update(ctx, new, data).await;
        }
        _ => {}
    }
    Ok(())
}
