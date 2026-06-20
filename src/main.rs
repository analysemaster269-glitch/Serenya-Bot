mod audio;
mod commands;
mod config;
mod core;
mod database;
mod discord;
mod logging;
mod utils;

use std::sync::Arc;

use dashmap::DashMap;
use poise::serenity_prelude as serenity;
use songbird::SerenityInit;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::BotConfig;
use crate::database::DatabaseManager;

/// Shared application state accessible from all command handlers.
pub struct Data {
    pub config: std::sync::RwLock<Arc<BotConfig>>,
    pub database: Arc<DatabaseManager>,
    pub guild_players: Arc<
        DashMap<serenity::GuildId, std::sync::Arc<tokio::sync::RwLock<crate::core::GuildPlayer>>>,
    >,
    pub http_client: reqwest::Client,
    pub start_time: std::time::Instant,
}

impl Data {
    pub fn config(&self) -> Arc<BotConfig> {
        self.config.read().unwrap().clone()
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Fatal error: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), utils::Error> {
    configure_path();

    let config = Arc::new(config::load_config("config.yml")?);

    // Register secrets for redaction
    crate::logging::register_secret_to_redact(&config.bot.token);
    if let Some(ref secret) = config.spotify.client_secret {
        crate::logging::register_secret_to_redact(secret);
    }
    if let Some(ref url) = config.logging.webhook_url {
        crate::logging::register_secret_to_redact(url);
    }
    if let Some(ref url) = config.bot.log_webhook_url {
        crate::logging::register_secret_to_redact(url);
    }

    audio::runtime::configure(&config.resolver, &config.spotify);
    init_tracing(&config.logging);
    info!("Starting Serenya...");
    info!(instance_id = %config.bot.instance_id, "Configuration loaded");

    let database = Arc::new(DatabaseManager::load("database.yml").await?);
    info!("Database loaded");

    let cancel_token = CancellationToken::new();
    let auto_save_handle =
        database.start_auto_save(std::time::Duration::from_secs(30), cancel_token.clone());

    let http_client = reqwest::Client::new();
    let start_time = std::time::Instant::now();

    let config_clone = Arc::clone(&config);
    let database_clone = Arc::clone(&database);

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: commands::all_commands(),
            prefix_options: poise::PrefixFrameworkOptions {
                dynamic_prefix: Some(|ctx| {
                    Box::pin(async move {
                        let prefix = ctx.data.config().bot.prefix.clone();
                        Ok(Some(prefix))
                    })
                }),
                ..Default::default()
            },
            on_error: |error| Box::pin(on_error(error)),
            pre_command: |ctx| {
                Box::pin(async move {
                    info!(
                        command = ctx.command().name,
                        user = %ctx.author().name,
                        user_id = %ctx.author().id,
                        guild_id = ?ctx.guild_id(),
                        "Command invoked"
                    );
                })
            },
            post_command: |ctx| {
                Box::pin(async move {
                    info!(
                        command = ctx.command().name,
                        user = %ctx.author().name,
                        user_id = %ctx.author().id,
                        guild_id = ?ctx.guild_id(),
                        "Command executed successfully"
                    );
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                info!("Slash commands registered globally");

                Ok(Data {
                    config: std::sync::RwLock::new(config_clone),
                    database: database_clone,
                    guild_players: std::sync::Arc::new(DashMap::new()),
                    http_client,
                    start_time,
                })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_VOICE_STATES;

    let songbird_config = songbird::Config::default().use_softclip(false);
    let mut client = serenity::ClientBuilder::new(&config.bot.token, intents)
        .framework(framework)
        .register_songbird_from_config(songbird_config)
        .await?;

    info!(
        display_name = %config.bot.display_name,
        "Serenya is ready"
    );

    tokio::select! {
        result = client.start() => {
            if let Err(err) = result {
                error!(%err, "Client exited with error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown signal received (ctrl+c)");
        }
    }

    shutdown(cancel_token, auto_save_handle, &database).await;
    Ok(())
}

async fn shutdown(
    cancel_token: CancellationToken,
    auto_save_handle: tokio::task::JoinHandle<()>,
    database: &DatabaseManager,
) {
    info!("Initiating graceful shutdown...");

    cancel_token.cancel();

    // Wait for auto-save task to finish
    if let Err(err) = auto_save_handle.await {
        error!(%err, "Auto-save task panicked during shutdown");
    }

    if let Err(err) = database.shutdown().await {
        error!(%err, "Failed to save database during shutdown");
    }

    info!("Serenya shut down gracefully");
}

/// Appends the `bin/` subdirectory (relative to CWD) to the process PATH.
fn configure_path() {
    if let Ok(cwd) = std::env::current_dir() {
        let bin_dir = cwd.join("bin");
        if bin_dir.exists() {
            let current = std::env::var("PATH").unwrap_or_default();
            unsafe {
                std::env::set_var("PATH", format!("{};{}", bin_dir.display(), current));
            }
            eprintln!("Added {} to PATH", bin_dir.display());
        }
    }
}

fn init_tracing(logging: &crate::config::LoggingSection) {
    use crate::logging::MakeRedactingWriter;
    use tracing::Level;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let env_filter_str = match logging.level.to_lowercase().as_str() {
        "error" => "error,songbird=error,serenity=error,hyper=error,reqwest=error",
        "warn" => "warn,serenya=warn,songbird=warn,serenity=warn,hyper=warn,reqwest=warn",
        "info" => "info,serenya=info,songbird=info,serenity=warn,hyper=info,reqwest=info",
        "debug" => "info,serenya=debug,songbird=info,serenity=warn,hyper=info,reqwest=info",
        "trace" => "info,serenya=trace,songbird=info,serenity=warn,hyper=info,reqwest=info",
        _ => "info,serenya=debug,songbird=info,serenity=warn,hyper=info,reqwest=info",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(env_filter_str));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_writer(MakeRedactingWriter);

    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);

    if logging.webhook_enabled {
        if let Some(ref url) = logging.webhook_url {
            let http_client = reqwest::Client::new();
            let min_level = match logging.webhook_min_level.to_lowercase().as_str() {
                "error" => Level::ERROR,
                "warn" => Level::WARN,
                "info" => Level::INFO,
                "debug" => Level::DEBUG,
                "trace" => Level::TRACE,
                _ => Level::INFO,
            };
            let webhook_layer = logging::webhook::WebhookLayer::new(
                url.clone(),
                http_client,
                min_level,
                logging.webhook_plain_text,
            );
            let _ = registry.with(webhook_layer).try_init();
            return;
        }
    }
    let _ = registry.try_init();
}

async fn on_error(error: poise::FrameworkError<'_, Data, utils::Error>) {
    match error {
        poise::FrameworkError::Command { error, ctx, .. } => {
            error!(%error, command = ctx.command().name, "Command error");
            let message = if let Some(serenya_err) =
                error.downcast_ref::<crate::utils::error::SerenyaError>()
            {
                match serenya_err {
                    crate::utils::error::SerenyaError::Permission(msg) => {
                        format!("**Permission Denied:** {msg}")
                    }
                    crate::utils::error::SerenyaError::NotFound(msg) => {
                        format!("**Not Found:** {msg}")
                    }
                    crate::utils::error::SerenyaError::Voice(msg) => {
                        format!("**Voice Connection Error:** {msg}")
                    }
                    crate::utils::error::SerenyaError::Queue(msg) => {
                        format!("**Queue Error:** {msg}")
                    }
                    crate::utils::error::SerenyaError::Database(msg) => {
                        format!("**Database Error:** {msg}")
                    }
                    crate::utils::error::SerenyaError::Config(msg) => {
                        format!("**Configuration Error:** {msg}")
                    }
                    other => format!("{other}"),
                }
            } else {
                error.to_string()
            };

            let embed = crate::discord::embeds::error_embed(&message);
            let reply = poise::CreateReply::default().embed(embed).ephemeral(true);
            let _ = ctx.send(reply).await;
        }
        poise::FrameworkError::Setup { error, .. } => {
            error!(%error, "Failed to start bot");
        }
        other => {
            if let Err(err) = poise::builtins::on_error(other).await {
                error!(%err, "Unhandled framework error");
            }
        }
    }
}
