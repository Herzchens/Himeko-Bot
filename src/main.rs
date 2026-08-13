pub mod ai;
mod commands;
mod config;
mod events;
pub mod logging;
mod permissions;
pub mod rank;
mod state;
mod text;
mod tls;
mod tts;

use config::Config;
use state::BotState;
use std::sync::Arc;
use text::normalizer::Normalizer;
use tokio::sync::RwLock;
use tts::engine::MsEdgeEngine;
use tts::gtts::GttsEngine;
use tts::openai::OpenAiEngine;
use tts::supertonic::SupertonicEngine;
use tts::vieneu::VieneuEngine;
use tts::TtsEngine;

use serenity::prelude::GatewayIntents;
use songbird::SerenityInit;

pub struct Data {
    pub config: Arc<RwLock<Arc<Config>>>,
    pub state: BotState,
    pub normalizer: RwLock<Arc<Normalizer>>,
    pub tts_engine: RwLock<Arc<dyn TtsEngine>>,
    pub tts_scheduler: Arc<tts::scheduler::TtsScheduler>,
    pub language_detector: lingua::LanguageDetector,
    pub rank_store: Arc<rank::db::RankStore>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tls::install_crypto_provider()?;

    let config = Config::load("config.yml")?;
    let config = Arc::new(config);

    // Setup tracing registry with fmt layer and optional Discord webhook layer
    use tracing_subscriber::prelude::*;
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("himeko_bot=debug".parse()?);
    let fmt_layer = tracing_subscriber::fmt::layer();
    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);

    let _log_task_handle = if !config.logging.webhook_url.is_empty() {
        let (discord_layer, log_task) =
            logging::start_discord_logging(config.logging.webhook_url.clone());
        let registry = registry.with(discord_layer);
        registry.init();
        Some(log_task)
    } else {
        registry.init();
        None
    };

    tracing::info!("config loaded — owner_id={}", config.permissions.owner_id);

    let rank_store = Arc::new(rank::db::RankStore::open(
        "database.yml",
        config.rank.legacy_guild_id(),
    )?);

    let default_female = config.tts.default_gender == "female";
    let state = BotState::new(default_female);
    state.active_console_channel.store(
        config.console_chat.default_channel_id,
        std::sync::atomic::Ordering::SeqCst,
    );

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
            let st_cfg = config.tts.get_supertonic_config().ok_or_else(|| {
                anyhow::anyhow!("tts.supertonic section required when provider = \"supertonic\"")
            })?;
            tracing::info!(server = %st_cfg.server_url, "using Supertonic engine");
            Arc::new(SupertonicEngine::new(st_cfg))
        }
        "openai" => {
            let oa_cfg = config.tts.get_openai_config().ok_or_else(|| {
                anyhow::anyhow!("tts.openai section required when provider = \"openai\"")
            })?;
            tracing::info!(url = %oa_cfg.api_url, model = %oa_cfg.model, "using OpenAI-compatible engine");
            Arc::new(OpenAiEngine::new(oa_cfg))
        }
        "vieneu" => {
            let vn_cfg = config.tts.get_vieneu_config().ok_or_else(|| {
                anyhow::anyhow!("tts.vieneu section required when provider = \"vieneu\"")
            })?;
            tracing::info!(server = %vn_cfg.server_url, "using VieNeu-TTS engine");
            Arc::new(VieneuEngine::new(vn_cfg).await?)
        }
        "msedge" => {
            tracing::info!("using MsEdge engine");
            Arc::new(MsEdgeEngine::new(config.tts.clone()))
        }
        other => anyhow::bail!("unsupported tts provider after validation: {other}"),
    };

    let config_clone = Arc::clone(&config);
    let rank_store_setup = Arc::clone(&rank_store);
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
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                tracing::info!(
                    commands = framework.options().commands.len(),
                    "slash commands registered globally"
                );

                // PR #9 switched the application to one guild-scoped command set. Global and
                // guild application command scopes coexist, so clear old guild-local definitions
                // after the global set is installed. Failure in one guild is observable but does
                // not make the bot unavailable everywhere else.
                for guild_id in ctx.cache.guilds() {
                    if let Err(error) = guild_id.set_commands(&ctx.http, Vec::new()).await {
                        tracing::warn!(
                            guild = %guild_id,
                            %error,
                            "failed to clear stale guild-scoped application commands"
                        );
                    }
                }

                let config_rwlock = Arc::new(RwLock::new(config_clone.clone()));

                if config_clone.console_chat.enabled {
                    let http_console = Arc::clone(&ctx.http);
                    let initial_channel_id = config_clone.console_chat.default_channel_id;
                    let state_clone = state.clone();

                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, BufReader};
                        let mut reader = BufReader::new(tokio::io::stdin()).lines();
                        tracing::info!(
                            default_channel = initial_channel_id,
                            "Console chat listener task started. Type messages to send to Discord. Type /channel <ID> to swap."
                        );

                        while let Ok(Some(line)) = reader.next_line().await {
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }

                            if line.starts_with("/channel ") || line.starts_with(":channel ") {
                                let id_str = line.split_whitespace().nth(1).unwrap_or("");
                                if let Some(new_id) = id_str.parse::<u64>().ok().filter(|id| *id != 0) {
                                    state_clone.active_console_channel.store(new_id, std::sync::atomic::Ordering::SeqCst);
                                    tracing::info!(new_channel_id = new_id, "Active console chat channel set");
                                } else {
                                    tracing::warn!("Invalid channel ID format. Usage: /channel <ID>");
                                }
                                continue;
                            }

                            if line.starts_with("/reply ") || line.starts_with("/r ") || line.starts_with(":reply ") || line.starts_with(":r ") {
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                if parts.len() >= 3 {
                                    if let Ok(idx) = parts[1].parse::<usize>() {
                                        if (1..=10).contains(&idx) {
                                            let msg_id_opt = {
                                                if let Ok(guard) = state_clone.recent_messages.lock() {
                                                    guard[idx - 1]
                                                } else {
                                                    None
                                                }
                                            };
                                            if let Some(msg_id) = msg_id_opt {
                                                let reply_text = parts[2..].join(" ");
                                                let active_channel_id = state_clone.active_console_channel.load(std::sync::atomic::Ordering::SeqCst);
                                                if active_channel_id == 0 {
                                                    tracing::warn!("No active console channel selected");
                                                    continue;
                                                }
                                                let chan = serenity::all::ChannelId::new(active_channel_id);
                                                let msg_ref = serenity::all::CreateMessage::new()
                                                    .content(&reply_text)
                                                    .reference_message((chan, msg_id));
                                                match chan.send_message(&http_console, msg_ref).await {
                                                    Ok(_) => {
                                                        tracing::info!(channel = active_channel_id, message = %reply_text, reply_to = %msg_id, "Sent reply from console");
                                                    }
                                                    Err(e) => {
                                                        tracing::error!(error = %e, "Failed to send reply from console");
                                                    }
                                                }
                                                continue;
                                            }
                                        }
                                    }
                                }
                                tracing::warn!("Invalid reply format. Usage: /r <1-10> <message>");
                                continue;
                            }

                            let active_channel_id = state_clone.active_console_channel.load(std::sync::atomic::Ordering::SeqCst);
                            if active_channel_id == 0 {
                                tracing::warn!("No active console channel selected");
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

                        let steps = &config.voice_status.steps;
                        let interval = std::time::Duration::from_secs(config.voice_status.interval_secs.max(10));

                        if config.voice_status.channel_id == 0 || steps.is_empty() {
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            continue;
                        }
                        let channel_id = serenity::all::ChannelId::new(config.voice_status.channel_id);

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
                                match http_for_task.edit_voice_status(channel_id, &map, None).await {
                                    Ok(_) => was_empty = Some(true),
                                    Err(error) => tracing::warn!(
                                        %error,
                                        channel_id = channel_id.get(),
                                        "Failed to clear voice channel status; will retry"
                                    ),
                                }
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
                    tts_scheduler: Arc::new(tts::scheduler::TtsScheduler::default()),
                    language_detector: lingua::LanguageDetectorBuilder::from_languages(&[lingua::Language::Vietnamese, lingua::Language::English]).build(),
                    rank_store: Arc::clone(&rank_store_setup),
                };

                if config_clone.rank.enabled {
                    let remote = rank::service::SerenityRankRemote::new(
                        ctx.http.as_ref(),
                        ctx.cache.current_user().id,
                    );
                    for (guild_id, rank_config) in config_clone.rank.configured_guilds()? {
                        match rank::service::initialize_if_needed(
                            &rank_store_setup,
                            &rank_config,
                            guild_id,
                            &remote,
                        )
                        .await
                        {
                            Ok(Some(report)) => tracing::info!(
                                guild_id,
                                added = report.added,
                                updated = report.updated,
                                removed = report.removed,
                                "first-run rank scan complete"
                            ),
                            Ok(None) => tracing::debug!(
                                guild_id,
                                "rank guild already initialized"
                            ),
                            Err(error) => tracing::error!(
                                guild_id,
                                %error,
                                "first-run rank scan failed; guild remains uninitialized"
                            ),
                        }
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
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::DIRECT_MESSAGES;

    let mut client = serenity::Client::builder(&config.bot.token, intents)
        .framework(framework)
        .register_songbird()
        .await
        .unwrap();

    let config_scheduler = config.clone();
    let rank_store_scheduler = Arc::clone(&rank_store);
    let http_scheduler = Arc::clone(&client.http);

    tokio::spawn(async move {
        use tokio_cron_scheduler::{Job, JobScheduler};
        if config_scheduler.rank.enabled {
            let sched = match JobScheduler::new().await {
                Ok(scheduler) => scheduler,
                Err(error) => {
                    tracing::error!(%error, "failed to create rank cron scheduler");
                    return;
                }
            };
            let cron_expr = "0 0 9 15 * *";
            let store = Arc::clone(&rank_store_scheduler);
            let cfg = config_scheduler.clone();
            let http = Arc::clone(&http_scheduler);

            let job = match Job::new_async_tz(
                cron_expr,
                chrono_tz::Asia::Ho_Chi_Minh,
                move |_uuid, _lock| {
                    let store = Arc::clone(&store);
                    let cfg = cfg.clone();
                    let http = Arc::clone(&http);
                    Box::pin(async move {
                        if let Err(error) = run_monthly_ping(&store, &cfg.rank, &http).await {
                            tracing::error!(%error, "monthly rank ping failed");
                        }
                    })
                },
            ) {
                Ok(job) => job,
                Err(error) => {
                    tracing::error!(%error, "failed to construct rank cron job");
                    return;
                }
            };

            if let Err(error) = sched.add(job).await {
                tracing::error!(%error, "failed to add rank cron job");
                return;
            }
            if let Err(error) = sched.start().await {
                tracing::error!(%error, "failed to start rank cron scheduler");
                return;
            }
            tracing::info!("cron scheduler started for multi-guild rank system");
        }
    });

    tracing::info!("starting bot...");
    client.start().await?;

    Ok(())
}

async fn run_monthly_ping(
    store: &rank::db::RankStore,
    rank_config: &config::RankConfig,
    http: &Arc<serenity::all::Http>,
) -> anyhow::Result<()> {
    for (guild_id, guild_config) in rank_config.configured_guilds()? {
        let state = store.guild_snapshot(guild_id).await;
        let top3 = rank::service::leaderboard(&state)
            .into_iter()
            .take(3)
            .collect::<Vec<_>>();
        if top3.is_empty() {
            continue;
        }

        let lines = top3
            .iter()
            .enumerate()
            .map(|(index, (user_id, level))| {
                let medal = match index {
                    0 => "🥇",
                    1 => "🥈",
                    2 => "🥉",
                    _ => "  ",
                };
                format!("{medal} #{} <@{user_id}> (Lv.{level})", index + 1)
            })
            .collect::<Vec<_>>();

        let embed = serenity::all::CreateEmbed::new()
            .title("🎉 BẢNG VÀNG THÁNG NÀY 🎉")
            .description(lines.join("\n"));
        let channel_id = serenity::all::ChannelId::new(guild_config.leaderboard_channel_id);
        if let Err(error) = channel_id
            .send_message(http, serenity::all::CreateMessage::new().embed(embed))
            .await
        {
            tracing::error!(
                guild_id,
                channel_id = guild_config.leaderboard_channel_id,
                %error,
                "failed to send monthly guild leaderboard"
            );
        }
    }
    Ok(())
}
