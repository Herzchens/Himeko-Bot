mod commands;
mod config;
mod events;
mod permissions;
mod state;
mod text;
mod tts;
pub mod ai;
pub mod rank;

use config::Config;
use state::BotState;
use std::sync::Arc;
use text::normalizer::Normalizer;
use tts::engine::MsEdgeEngine;
use tts::gtts::GttsEngine;
use tts::supertonic::SupertonicEngine;
use tts::openai::OpenAiEngine;
use tts::vieneu::VieneuEngine;
use tts::TtsEngine;
use tokio::sync::RwLock;

use serenity::prelude::GatewayIntents;
use songbird::SerenityInit;

pub struct Data {
    pub config: Arc<RwLock<Arc<Config>>>,
    pub state: BotState,
    pub normalizer: RwLock<Arc<Normalizer>>,
    pub tts_engine: RwLock<Arc<dyn TtsEngine>>,
    pub language_detector: lingua::LanguageDetector,
    pub rank_db: Arc<tokio::sync::RwLock<rank::db::RankDatabase>>,
}



#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("himeko_bot=debug".parse()?),
        )
        .init();

    let config = Config::load("config.yml")?;
    let config = Arc::new(config);

    tracing::info!("config loaded — owner_id={}", config.permissions.owner_id);

    let rank_db = Arc::new(tokio::sync::RwLock::new(
        rank::db::RankDatabase::load("database.yml").unwrap_or_default()
    ));

    let state = BotState::default();

    let default_female = config.tts.default_gender == "female";
    tracing::info!(
        default_voice = if default_female { "female" } else { "male" },
        "TTS engine configured"
    );

    let normalizer = Arc::new(Normalizer::from_config(&config.abbreviations));
    tracing::info!(
        abbreviations = config.abbreviations.len(),
        "normalizer loaded"
    );

    let tts_engine: Arc<dyn TtsEngine> = match config.tts.provider.as_str() {
        "gtts" => {
            tracing::info!("using gTTS engine");
            Arc::new(GttsEngine::new())
        }
        "supertonic" => {
            let st_cfg = config.tts.get_supertonic_config()
                .ok_or_else(|| anyhow::anyhow!("tts.supertonic section required when provider = \"supertonic\""))?;
            tracing::info!(server = %st_cfg.server_url, "using Supertonic engine");
            Arc::new(SupertonicEngine::new(st_cfg))
        }
        "openai" => {
            let oa_cfg = config.tts.get_openai_config()
                .ok_or_else(|| anyhow::anyhow!("tts.openai section required when provider = \"openai\""))?;
            tracing::info!(url = %oa_cfg.api_url, model = %oa_cfg.model, "using OpenAI-compatible engine");
            Arc::new(OpenAiEngine::new(oa_cfg))
        }
        "vieneu" => {
            let vn_cfg = config.tts.get_vieneu_config()
                .ok_or_else(|| anyhow::anyhow!("tts.vieneu section required when provider = \"vieneu\""))?;
            tracing::info!(server = %vn_cfg.server_url, "using VieNeu-TTS engine");
            Arc::new(VieneuEngine::new(vn_cfg))
        }
        _ => {
            tracing::info!("using MsEdge engine");
            Arc::new(MsEdgeEngine::new(config.tts.clone()))
        }
    };

    let config_clone = Arc::clone(&config);
    let rank_db_setup = Arc::clone(&rank_db);
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands::ask::ask(),
                commands::gender::gender(),
                commands::join::join(),
                commands::leave::leave(),
                commands::ping::ping(),
                commands::reload::reload(),
                commands::makecustom::makecustom(),
                commands::up::up(),
                commands::down::down(),
                commands::remove::remove(),
                commands::leaderboard::leaderboard(),
                commands::autorename::autorename(),
                commands::rescan::rescan(),
                commands::echo::echo(),
            ],
            event_handler: |ctx, event, framework, data| {
                Box::pin(events::handler::event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                // Register commands globally. 
                // Global commands can take up to an hour to propagate, but this prevents duplicates 
                // and is the standard for production bots.
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;

                tracing::info!(
                    commands = framework.options().commands.len(),
                    "slash commands registered in guilds (instant update)"
                );
                let config_rwlock = Arc::new(RwLock::new(config_clone.clone()));

                if config_clone.console_chat.enabled {
                    let http_console = Arc::clone(&ctx.http);
                    let initial_channel_id = config_clone.console_chat.default_channel_id;

                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, BufReader};
                        let mut reader = BufReader::new(tokio::io::stdin()).lines();
                        let mut active_channel_id = initial_channel_id;

                        tracing::info!(
                            default_channel = active_channel_id,
                            "Console chat listener task started. Type messages to send to Discord. Type /channel <ID> to swap."
                        );

                        while let Ok(Some(line)) = reader.next_line().await {
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }

                            if line.starts_with("/channel ") || line.starts_with(":channel ") {
                                let id_str = line.split_whitespace().nth(1).unwrap_or("");
                                if let Ok(new_id) = id_str.parse::<u64>() {
                                    active_channel_id = new_id;
                                    tracing::info!(new_channel_id = active_channel_id, "Active console chat channel set");
                                } else {
                                    tracing::warn!("Invalid channel ID format. Usage: /channel <ID>");
                                }
                                continue;
                            }

                            let chan = serenity::all::ChannelId::new(active_channel_id);
                            match chan.say(&http_console, line).await {
                                Ok(_) => {
                                    tracing::info!(channel = active_channel_id, message = %line, "Sent message from console");
                                }
                                Err(e) => {
                                    tracing::error!(error = %e, channel = active_channel_id, "Failed to send message from console");
                                }
                            }
                        }
                    });
                }

                let config_for_task = config_rwlock.clone();
                let http_for_task = Arc::clone(&ctx.http);
                let cache_for_task = Arc::clone(&ctx.cache);

                tokio::spawn(async move {
                    let mut current_index = 0;
                    let mut was_empty = None;
                    loop {
                        let config = config_for_task.read().await.clone();
                        
                        if !config.voice_status.enabled {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            continue;
                        }

                        let channel_id = serenity::all::ChannelId::new(config.voice_status.channel_id);
                        let steps = &config.voice_status.steps;
                        let interval = std::time::Duration::from_secs(config.voice_status.interval_secs.max(10));

                        if channel_id.get() == 0 || steps.is_empty() {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            continue;
                        }

                        // Check member count in target voice channel (excluding bots)
                        let mut member_count = 0;
                        let mut target_guild = None;
                        for guild_id in cache_for_task.guilds() {
                            if let Some(guild) = cache_for_task.guild(guild_id) {
                                if guild.channels.contains_key(&channel_id) {
                                    target_guild = Some(guild.clone());
                                    break;
                                }
                            }
                        }

                        if let Some(guild) = target_guild {
                            member_count = guild.voice_states.iter()
                                .filter(|(user_id, vs)| {
                                    if vs.channel_id != Some(channel_id) {
                                        return false;
                                    }
                                    if let Some(user) = cache_for_task.user(*user_id) {
                                        !user.bot
                                    } else {
                                        true
                                    }
                                })
                                .count();
                        }

                        if member_count == 0 {
                            if was_empty != Some(true) {
                                tracing::info!(channel_id = channel_id.get(), "Voice channel is empty. Clearing status.");
                                let map = serde_json::json!({
                                    "status": ""
                                });
                                let _ = http_for_task.edit_voice_status(channel_id, &map, None).await;
                                was_empty = Some(true);
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            continue;
                        }

                        was_empty = Some(false);

                        let status_text = if config.voice_status.random {
                            use rand::seq::IndexedRandom;
                            use rand::rng;
                            steps.choose(&mut rng()).unwrap_or(&steps[0])
                        } else {
                            if current_index >= steps.len() {
                                current_index = 0;
                            }
                            let text = &steps[current_index];
                            current_index = (current_index + 1) % steps.len();
                            text
                        };

                        tracing::debug!(
                            channel_id = channel_id.get(),
                            status = %status_text,
                            "Updating voice channel status"
                        );

                        let map = serde_json::json!({
                            "status": status_text
                        });

                        match http_for_task.edit_voice_status(channel_id, &map, None).await {
                            Ok(_) => {
                                tracing::info!(
                                    channel_id = channel_id.get(),
                                    status = %status_text,
                                    "Successfully updated voice channel status"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    channel_id = channel_id.get(),
                                    error = %e,
                                    "Failed to update voice channel status"
                                );
                            }
                        }

                        tokio::time::sleep(interval).await;
                    }
                });

                let data = Data {
                    config: config_rwlock,
                    state,
                    normalizer: RwLock::new(normalizer),
                    tts_engine: RwLock::new(tts_engine),
                    language_detector: lingua::LanguageDetectorBuilder::from_languages(&[lingua::Language::Vietnamese, lingua::Language::English]).build(),
                    rank_db: Arc::clone(&rank_db_setup),
                };

                if config_clone.rank.enabled {
                    let mut db = rank_db_setup.write().await;
                    if !db.initialized {
                        tracing::info!("first-run: scanning guild members...");
                        let guild_id = serenity::all::GuildId::new(config_clone.rank.guild_id);
                        let mut after: Option<serenity::all::UserId> = None;
                        let mut total_added = 0u32;

                        loop {
                            let members_res: Result<Vec<serenity::all::Member>, serenity::all::Error> = guild_id.members(&ctx.http, Some(1000), after).await;
                            match members_res {
                                Ok(members) => {
                                    if members.is_empty() { break; }
                                    for member in &members {
                                        if member.user.bot { continue; }
                                        let uid = member.user.id.get().to_string();
                                        if db.users.contains_key(&uid) { continue; }

                                        let nick = member.nick.as_deref().unwrap_or(&member.user.name);
                                        if let Some(level) = rank::logic::parse_nickname(&config_clone.rank, nick) {
                                            db.users.insert(uid, rank::db::RankUserData {
                                                level,
                                                original_name: nick.to_string(),
                                            });
                                            total_added += 1;
                                        }
                                    }
                                    after = members.last().map(|m| m.user.id);
                                    if members.len() < 1000 { break; }
                                }
                                Err(e) => {
                                    tracing::error!(error = %e, "Failed to fetch members for scan");
                                    break;
                                }
                            }
                        }

                        db.initialized = true;
                        let _ = db.save("database.yml");
                        tracing::info!(added = total_added, "first-run scan complete");
                    }
                }

                Ok(data)
            })
        })
        .build();

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = serenity::Client::builder(&config.bot.token, intents)
        .framework(framework)
        .register_songbird()
        .await
        .unwrap();

    let config_scheduler = config.clone();
    let rank_db_scheduler = Arc::clone(&rank_db);
    let http_scheduler = Arc::clone(&client.http);

    tokio::spawn(async move {
        use tokio_cron_scheduler::{Job, JobScheduler};
        if config_scheduler.rank.enabled {
            let sched = JobScheduler::new().await.unwrap();
            let cron_expr = "0 0 9 15 * *";
            let db_clone = Arc::clone(&rank_db_scheduler);
            let cfg_clone = config_scheduler.clone();
            let http_clone = Arc::clone(&http_scheduler);

            let job = Job::new_async_tz(
                cron_expr,
                chrono_tz::Asia::Ho_Chi_Minh,
                move |_uuid, _lock| {
                    let db = Arc::clone(&db_clone);
                    let cfg = cfg_clone.clone();
                    let http = Arc::clone(&http_clone);
                    Box::pin(async move {
                        if let Err(e) = run_monthly_ping(&db, &cfg.rank, &http).await {
                            tracing::error!(error = %e, "monthly top3 ping failed");
                        }
                    })
                },
            ).unwrap();

            sched.add(job).await.unwrap();
            sched.start().await.unwrap();
            tracing::info!("cron scheduler started for rank system");
        }
    });

    tracing::info!("starting bot...");
    client.start().await?;

    Ok(())
}

async fn run_monthly_ping(
    db_lock: &tokio::sync::RwLock<rank::db::RankDatabase>,
    rank_config: &config::RankConfig,
    http: &Arc<serenity::all::Http>,
) -> anyhow::Result<()> {
    if rank_config.leaderboard_channel_id == 0 {
        return Ok(());
    }

    let db = db_lock.read().await;
    let mut ranked_users: Vec<_> = db.users.iter().filter(|(_, u)| u.level > 0).collect();
    if ranked_users.is_empty() { return Ok(()); }

    ranked_users.sort_by(|(id_a, u_a), (id_b, u_b)| u_b.level.cmp(&u_a.level).then_with(|| id_a.cmp(id_b)));
    let top3 = ranked_users.into_iter().take(3).collect::<Vec<_>>();

    let mut lines = Vec::new();
    for (i, (uid, user_data)) in top3.iter().enumerate() {
        let medal = match i {
            0 => "🥇",
            1 => "🥈",
            2 => "🥉",
            _ => "  ",
        };
        lines.push(format!("{} #{} <@{}> (Lv.{})", medal, i + 1, uid, user_data.level));
    }

    let embed = serenity::all::CreateEmbed::new()
        .title("🎉 BẢNG VÀNG THÁNG NÀY 🎉")
        .description(lines.join("\n"));

    let channel_id = serenity::all::ChannelId::new(rank_config.leaderboard_channel_id);
    channel_id.send_message(http, serenity::all::CreateMessage::new().embed(embed)).await?;

    Ok(())
}
