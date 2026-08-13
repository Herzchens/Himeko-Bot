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
use tokio::sync::{Mutex, RwLock};
use tts::engine::MsEdgeEngine;
use tts::gtts::GttsEngine;
use tts::openai::OpenAiEngine;
use tts::supertonic::SupertonicEngine;
use tts::vieneu::VieneuEngine;
use tts::TtsEngine;

use serenity::prelude::GatewayIntents;
use songbird::SerenityInit;

pub struct RuntimeSnapshot {
    pub config: Arc<Config>,
    pub normalizer: Arc<Normalizer>,
    pub tts_engine: Arc<dyn TtsEngine>,
    pub default_female: bool,
}

pub struct RuntimeState {
    current: RwLock<Arc<RuntimeSnapshot>>,
    updates: tokio::sync::watch::Sender<u64>,
    reload_gate: Mutex<()>,
}

impl RuntimeState {
    pub fn new(initial: Arc<RuntimeSnapshot>) -> Self {
        let (updates, _initial_receiver) = tokio::sync::watch::channel(0);
        Self {
            current: RwLock::new(initial),
            updates,
            reload_gate: Mutex::new(()),
        }
    }

    pub async fn begin_reload(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.reload_gate.lock().await
    }

    pub async fn snapshot(&self) -> Arc<RuntimeSnapshot> {
        let current = self.current.read().await;
        Arc::clone(&*current)
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.updates.subscribe()
    }

    pub async fn publish(&self, next: Arc<RuntimeSnapshot>) {
        {
            let mut current = self.current.write().await;
            *current = next;
        }
        self.updates
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}

pub struct Data {
    pub runtime: Arc<RuntimeState>,
    pub state: BotState,
    pub tts_scheduler: Arc<tts::scheduler::TtsScheduler>,
    pub language_detector: lingua::LanguageDetector,
    pub rank_store: Arc<rank::db::RankStore>,
}

impl Data {
    pub async fn runtime_snapshot(&self) -> Arc<RuntimeSnapshot> {
        self.runtime.snapshot().await
    }

    pub async fn config_snapshot(&self) -> Arc<Config> {
        Arc::clone(&self.runtime.snapshot().await.config)
    }

    pub async fn publish_runtime(&self, next: Arc<RuntimeSnapshot>) {
        self.runtime.publish(next).await;
    }
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

    let rank_store = Arc::new(rank::db::RankStore::open_runtime(
        "database.yml",
        config.rank.legacy_guild_id(),
        config.rank.enabled,
    )?);
    if rank_store.legacy_migration_pending() {
        tracing::warn!(
            "rank is disabled and a valid legacy database was found; migration is deferred until restart with rank enabled"
        );
    }

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
            Arc::new(SupertonicEngine::new(st_cfg).await?)
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

    let initial_runtime = Arc::new(RuntimeSnapshot {
        config: Arc::clone(&config),
        normalizer,
        tts_engine,
        default_female,
    });
    let runtime_clone = Arc::clone(&initial_runtime);
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

                let runtime_state = Arc::new(RuntimeState::new(Arc::clone(&runtime_clone)));

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
                                        if let Some(reference) = state_clone.recent_message(idx) {
                                            let reply_text = parts[2..].join(" ");
                                            let chan = reference.channel_id;
                                            let msg_ref = serenity::all::CreateMessage::new()
                                                .content(&reply_text)
                                                .reference_message((chan, reference.message_id));
                                            match chan.send_message(&http_console, msg_ref).await {
                                                Ok(_) => {
                                                    tracing::info!(channel = %chan, message = %reply_text, reply_to = %reference.message_id, "Sent reply from console");
                                                }
                                                Err(e) => {
                                                    tracing::error!(error = %e, channel = %chan, "Failed to send reply from console");
                                                }
                                            }
                                            continue;
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

                let runtime_for_task = Arc::clone(&runtime_state);
                let http_for_task = Arc::clone(&ctx.http);
                let cache_for_task = Arc::clone(&ctx.cache);
                tokio::spawn(run_voice_status_task(
                    runtime_for_task,
                    http_for_task,
                    cache_for_task,
                ));

                let data = Data {
                    runtime: Arc::clone(&runtime_state),
                    state,
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
                        match rank::service::reconcile_guild(
                            &rank_store_setup,
                            &rank_config,
                            guild_id,
                            &remote,
                        )
                        .await
                        {
                            Ok(report) => tracing::info!(
                                guild_id,
                                added = report.added,
                                updated = report.updated,
                                removed = report.removed,
                                "startup rank reconciliation complete"
                            ),
                            Err(error) => tracing::error!(
                                guild_id,
                                %error,
                                "startup rank reconciliation failed; scheduled rank work stays inactive"
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

#[derive(Default)]
struct VoiceStatusLoopState {
    applied: Option<config::VoiceStatusConfig>,
    pending_clear: Option<u64>,
    current_index: usize,
    was_empty: Option<bool>,
}

impl VoiceStatusLoopState {
    fn transition(&mut self, desired: &config::VoiceStatusConfig) -> Option<u64> {
        if let Some(channel_id) = self.pending_clear {
            return Some(channel_id);
        }
        if self.applied.as_ref() == Some(desired) {
            return None;
        }

        if let Some(previous) = self.applied.as_ref() {
            let needs_clear = previous.enabled
                && previous.channel_id != 0
                && (!desired.enabled || previous.channel_id != desired.channel_id);
            if needs_clear {
                self.pending_clear = Some(previous.channel_id);
                return self.pending_clear;
            }
        }

        self.applied = Some(desired.clone());
        self.current_index = 0;
        self.was_empty = None;
        None
    }

    fn clear_succeeded(&mut self, channel_id: u64) {
        if self.pending_clear == Some(channel_id) {
            self.pending_clear = None;
            self.applied = None;
            self.current_index = 0;
            self.was_empty = None;
        }
    }
}

async fn wait_for_runtime_update_or_timeout(
    updates: &mut tokio::sync::watch::Receiver<u64>,
    duration: std::time::Duration,
) {
    tokio::select! {
        _ = tokio::time::sleep(duration) => {}
        changed = updates.changed() => {
            if changed.is_err() {
                tokio::time::sleep(duration).await;
            }
        }
    }
}

async fn run_voice_status_task(
    runtime: Arc<RuntimeState>,
    http: Arc<serenity::all::Http>,
    cache: Arc<serenity::all::Cache>,
) {
    let mut loop_state = VoiceStatusLoopState::default();
    let mut runtime_updates = runtime.subscribe();

    loop {
        let desired = runtime.snapshot().await.config.voice_status.clone();

        if let Some(stale_channel_id) = loop_state.transition(&desired) {
            let channel_id = serenity::all::ChannelId::new(stale_channel_id);
            let map = serde_json::json!({ "status": "" });
            match http.edit_voice_status(channel_id, &map, None).await {
                Ok(_) => {
                    tracing::info!(
                        channel_id = stale_channel_id,
                        "cleared stale voice status before applying reloaded voice-status config"
                    );
                    loop_state.clear_succeeded(stale_channel_id);
                    continue;
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        channel_id = stale_channel_id,
                        "failed to clear stale voice status; retaining one pending clear for retry"
                    );
                    wait_for_runtime_update_or_timeout(
                        &mut runtime_updates,
                        std::time::Duration::from_secs(15),
                    )
                    .await;
                    continue;
                }
            }
        }

        if !desired.enabled {
            wait_for_runtime_update_or_timeout(
                &mut runtime_updates,
                std::time::Duration::from_secs(15),
            )
            .await;
            continue;
        }

        let steps = &desired.steps;
        let interval = std::time::Duration::from_secs(desired.interval_secs.max(10));
        if desired.channel_id == 0 || steps.is_empty() {
            wait_for_runtime_update_or_timeout(
                &mut runtime_updates,
                std::time::Duration::from_secs(15),
            )
            .await;
            continue;
        }
        let channel_id = serenity::all::ChannelId::new(desired.channel_id);

        let mut member_count = 0;
        let mut target_guild = None;
        for guild_id in cache.guilds() {
            if let Some(guild) = cache.guild(guild_id) {
                if guild.channels.contains_key(&channel_id) {
                    target_guild = Some(guild.clone());
                    break;
                }
            }
        }

        if let Some(guild) = target_guild {
            member_count = guild
                .voice_states
                .iter()
                .filter(|(user_id, voice_state)| {
                    if voice_state.channel_id != Some(channel_id) {
                        return false;
                    }
                    cache.user(*user_id).is_none_or(|user| !user.bot)
                })
                .count();
        }

        if member_count == 0 {
            if loop_state.was_empty != Some(true) {
                let map = serde_json::json!({ "status": "" });
                match http.edit_voice_status(channel_id, &map, None).await {
                    Ok(_) => loop_state.was_empty = Some(true),
                    Err(error) => tracing::warn!(
                        %error,
                        channel_id = channel_id.get(),
                        "failed to clear voice channel status; will retry"
                    ),
                }
            }
            wait_for_runtime_update_or_timeout(
                &mut runtime_updates,
                std::time::Duration::from_secs(15),
            )
            .await;
            continue;
        }

        loop_state.was_empty = Some(false);
        let status_text = if desired.random {
            use rand::seq::IndexedRandom;
            let Some(status) = steps.choose(&mut rand::rng()) else {
                wait_for_runtime_update_or_timeout(
                    &mut runtime_updates,
                    std::time::Duration::from_secs(15),
                )
                .await;
                continue;
            };
            status
        } else {
            if loop_state.current_index >= steps.len() {
                loop_state.current_index = 0;
            }
            let text = &steps[loop_state.current_index];
            loop_state.current_index = (loop_state.current_index + 1) % steps.len();
            text
        };

        let map = serde_json::json!({ "status": status_text });
        match http.edit_voice_status(channel_id, &map, None).await {
            Ok(_) => tracing::info!(
                channel_id = channel_id.get(),
                status = %status_text,
                "successfully updated voice channel status"
            ),
            Err(error) => tracing::error!(
                channel_id = channel_id.get(),
                %error,
                "failed to update voice channel status"
            ),
        }

        wait_for_runtime_update_or_timeout(&mut runtime_updates, interval).await;
    }
}

fn monthly_channel_belongs_to_guild(
    channel: &serenity::all::Channel,
    expected_guild_id: u64,
) -> bool {
    matches!(
        channel,
        serenity::all::Channel::Guild(guild_channel)
            if guild_channel.guild_id.get() == expected_guild_id
    )
}

async fn run_monthly_ping(
    store: &rank::db::RankStore,
    rank_config: &config::RankConfig,
    http: &Arc<serenity::all::Http>,
) -> anyhow::Result<()> {
    for (guild_id, guild_config) in rank_config.configured_guilds()? {
        if !store.is_runtime_guild_active(guild_id) {
            tracing::debug!(guild_id, "skipping monthly rank work for inactive guild");
            continue;
        }
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
        let channel = match channel_id.to_channel(http).await {
            Ok(channel) => channel,
            Err(error) => {
                tracing::error!(
                    guild_id,
                    channel_id = channel_id.get(),
                    %error,
                    "failed to resolve monthly leaderboard channel"
                );
                continue;
            }
        };
        if !monthly_channel_belongs_to_guild(&channel, guild_id) {
            tracing::error!(
                guild_id,
                channel_id = channel_id.get(),
                "monthly leaderboard channel belongs to a different guild; refusing to send"
            );
            continue;
        }
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

#[cfg(test)]
mod monthly_channel_boundary_tests {
    use super::*;

    fn guild_channel(guild_id: u64) -> serenity::all::Channel {
        let mut channel = serenity::all::GuildChannel::default();
        channel.guild_id = serenity::all::GuildId::new(guild_id);
        serenity::all::Channel::Guild(channel)
    }

    #[test]
    fn monthly_channel_accepts_only_the_configured_guild() {
        let channel = guild_channel(10);
        assert!(monthly_channel_belongs_to_guild(&channel, 10));
        assert!(!monthly_channel_belongs_to_guild(&channel, 20));
    }
}

#[cfg(test)]
mod runtime_snapshot_tests {
    use super::*;
    use std::collections::HashMap;

    struct MarkerEngine(u8);

    #[async_trait::async_trait]
    impl TtsEngine for MarkerEngine {
        async fn synthesize(&self, _text: &str, _voice: &str) -> anyhow::Result<Vec<u8>> {
            Ok(vec![self.0])
        }
    }

    fn snapshot(
        max_chars: usize,
        word: &str,
        engine: u8,
        default_female: bool,
    ) -> Arc<RuntimeSnapshot> {
        let mut config: Config = serde_yaml::from_str(
            r#"
bot:
  token: test-token
  application_id: 1
permissions:
  owner_id: 1
tts:
  provider: msedge
  msedge: []
"#,
        )
        .expect("test config must parse");
        config.tts.max_chars = max_chars;
        let normalizer = Arc::new(Normalizer::from_config(&HashMap::from([(
            "x".to_string(),
            word.to_string(),
        )])));
        Arc::new(RuntimeSnapshot {
            config: Arc::new(config),
            normalizer,
            tts_engine: Arc::new(MarkerEngine(engine)),
            default_female,
        })
    }

    async fn assert_generation_is_coherent(snapshot: Arc<RuntimeSnapshot>) {
        let observed = (
            snapshot.config.tts.max_chars,
            snapshot.normalizer.expand("x"),
            snapshot
                .tts_engine
                .synthesize("marker", "voice")
                .await
                .expect("marker engine must synthesize"),
            snapshot.default_female,
        );
        assert!(
            observed == (11, "old".to_string(), vec![1], true)
                || observed == (22, "new".to_string(), vec![2], false),
            "runtime reader observed a mixed generation: {observed:?}"
        );
    }

    #[tokio::test]
    async fn reload_transactions_are_serialized() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = Arc::new(RuntimeState::new(snapshot(11, "initial", 1, true)));
        let first = state.begin_reload().await;
        let second_state = Arc::clone(&state);
        let second_entered = Arc::new(AtomicBool::new(false));
        let second_entered_task = Arc::clone(&second_entered);

        let second = tokio::spawn(async move {
            let _guard = second_state.begin_reload().await;
            second_entered_task.store(true, Ordering::SeqCst);
        });

        tokio::task::yield_now().await;
        assert!(
            !second_entered.load(Ordering::SeqCst),
            "a second reload must wait while the first transaction is still active"
        );

        drop(first);
        tokio::time::timeout(std::time::Duration::from_secs(1), second)
            .await
            .expect("second reload must proceed after the first transaction releases")
            .expect("second reload task must complete");
        assert!(second_entered.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn runtime_publication_wakes_subscribers_even_before_they_poll() {
        let old = snapshot(11, "old", 1, true);
        let new = snapshot(22, "new", 2, false);
        let state = Arc::new(RuntimeState::new(old));
        let mut updates = state.subscribe();

        state.publish(new).await;

        tokio::time::timeout(std::time::Duration::from_secs(1), updates.changed())
            .await
            .expect("runtime update notification must not wait for the old timer")
            .expect("RuntimeState sender must remain alive");
        assert_eq!(state.snapshot().await.config.tts.max_chars, 22);
    }

    #[tokio::test]
    async fn concurrent_runtime_readers_never_observe_mixed_generations() {
        let old = snapshot(11, "old", 1, true);
        let new = snapshot(22, "new", 2, false);
        let state = Arc::new(RuntimeState::new(Arc::clone(&old)));

        let writer_state = Arc::clone(&state);
        let writer_old = Arc::clone(&old);
        let writer_new = Arc::clone(&new);
        let writer = tokio::spawn(async move {
            for index in 0..500 {
                let next = if index % 2 == 0 {
                    Arc::clone(&writer_new)
                } else {
                    Arc::clone(&writer_old)
                };
                writer_state.publish(next).await;
                tokio::task::yield_now().await;
            }
        });

        let mut readers = Vec::new();
        for _ in 0..8 {
            let reader_state = Arc::clone(&state);
            readers.push(tokio::spawn(async move {
                for _ in 0..250 {
                    assert_generation_is_coherent(reader_state.snapshot().await).await;
                    tokio::task::yield_now().await;
                }
            }));
        }

        writer.await.expect("writer task must complete");
        for reader in readers {
            reader.await.expect("reader task must complete");
        }
    }

    #[test]
    fn voice_status_transition_keeps_only_one_stale_clear_pending() {
        let mut state = VoiceStatusLoopState::default();
        let first = config::VoiceStatusConfig {
            enabled: true,
            channel_id: 10,
            interval_secs: 30,
            steps: vec!["one".into()],
            random: false,
        };
        let second = config::VoiceStatusConfig {
            channel_id: 20,
            steps: vec!["two".into()],
            ..first.clone()
        };
        let third = config::VoiceStatusConfig {
            channel_id: 30,
            steps: vec!["three".into()],
            ..first.clone()
        };

        assert_eq!(state.transition(&first), None);
        state.current_index = 7;
        state.was_empty = Some(false);
        assert_eq!(state.transition(&second), Some(10));
        assert_eq!(state.transition(&third), Some(10));
        state.clear_succeeded(10);
        assert_eq!(state.transition(&third), None);
        assert_eq!(
            state.applied.as_ref().map(|value| value.channel_id),
            Some(30)
        );
        assert_eq!(state.current_index, 0);
        assert_eq!(state.was_empty, None);
    }

    #[test]
    fn voice_status_same_channel_reload_resets_cursor_without_stale_clear() {
        let mut state = VoiceStatusLoopState::default();
        let first = config::VoiceStatusConfig {
            enabled: true,
            channel_id: 10,
            interval_secs: 30,
            steps: vec!["one".into(), "two".into()],
            random: false,
        };
        let mut changed = first.clone();
        changed.steps = vec!["new".into()];

        assert_eq!(state.transition(&first), None);
        state.current_index = 1;
        state.was_empty = Some(false);
        assert_eq!(state.transition(&changed), None);
        assert_eq!(state.current_index, 0);
        assert_eq!(state.was_empty, None);
        assert_eq!(state.pending_clear, None);
    }
}
