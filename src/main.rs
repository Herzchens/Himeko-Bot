mod commands;
mod config;
mod events;
mod permissions;
mod state;
mod text;
mod tts;

use config::Config;
use state::BotState;
use std::sync::Arc;
use text::normalizer::Normalizer;
use tts::engine::MsEdgeEngine;
use tts::gtts::GttsEngine;
use tts::TtsEngine;
use tokio::sync::RwLock;

use serenity::prelude::GatewayIntents;
use songbird::SerenityInit;

pub struct Data {
    pub config: RwLock<Arc<Config>>,
    pub state: BotState,
    pub normalizer: RwLock<Arc<Normalizer>>,
    pub tts_engine: RwLock<Arc<dyn TtsEngine>>,
    pub language_detector: lingua::LanguageDetector,
}

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tts_bot=debug".parse()?),
        )
        .init();

    let config = Config::load("config.yml")?;
    let config = Arc::new(config);

    tracing::info!("config loaded — owner_id={}", config.permissions.owner_id);

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

    let tts_engine: Arc<dyn TtsEngine> = if config.tts.provider == "gtts" {
        tracing::info!("using gTTS engine");
        Arc::new(GttsEngine::new())
    } else {
        tracing::info!("using MsEdge engine");
        Arc::new(MsEdgeEngine::new(config.tts.clone()))
    };

    let config_clone = Arc::clone(&config);
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                commands::join::join(),
                commands::leave::leave(),
                commands::ping::ping(),
                commands::gender::gender(),
                commands::reload::reload(),
            ],
            event_handler: |ctx, event, framework, data| {
                Box::pin(events::handler::event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                
                for guild in ctx.cache.guilds() {
                    let _ = poise::builtins::register_in_guild(ctx, &Vec::<poise::Command<Data, Error>>::new(), guild).await;
                }

                tracing::info!(
                    commands = framework.options().commands.len(),
                    "slash commands registered globally"
                );
                Ok(Data {
                    config: RwLock::new(config_clone),
                    state,
                    normalizer: RwLock::new(normalizer),
                    tts_engine: RwLock::new(tts_engine),
                    language_detector: lingua::LanguageDetectorBuilder::from_languages(&[lingua::Language::Vietnamese, lingua::Language::English]).build(),
                })
            })
        })
        .build();

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = serenity::Client::builder(&config.bot.token, intents)
        .framework(framework)
        .register_songbird()
        .await?;

    tracing::info!("starting bot...");
    client.start().await?;

    Ok(())
}
