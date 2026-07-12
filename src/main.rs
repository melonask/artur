use artur::load_config;
use clap::{Parser, Subcommand};
use std::future::IntoFuture;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::Notify;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(author, version, about = "Config-driven HTTP server")]
struct Cli {
    /// TOML configuration file path or http(s) URL.
    #[arg(long, env = "ARTUR_CONFIG")]
    config: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and validate configuration without opening listeners or connecting to stores.
    Check,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = load_config(&cli.config).await?;
    if matches!(cli.command, Some(Command::Check)) {
        println!("configuration valid");
        return Ok(());
    }
    let default_filter = config
        .log
        .level
        .clone()
        .unwrap_or_else(|| "artur=info,tower_http=info".to_string());
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| default_filter.into());
    match config.log.format {
        Some(artur::config::LogFormat::Json) => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filter)
                .init();
        }
        Some(artur::config::LogFormat::Pretty) => {
            tracing_subscriber::fmt()
                .pretty()
                .with_env_filter(filter)
                .init();
        }
        None => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
    let server = config.server_config();
    let bind = server.bind.clone();
    let port = server.port;
    let shutdown_timeout = config
        .runtime
        .shutdown_timeout_secs
        .map(Duration::from_secs);
    let app = artur::build_router(config).await?;
    let addr: SocketAddr = format!("{bind}:{port}").parse()?;
    tracing::info!(%addr, "starting artur server");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let shutdown = Arc::new(Notify::new());
    let graceful_shutdown = {
        let shutdown = shutdown.clone();
        async move { shutdown.notified().await }
    };
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(graceful_shutdown)
    .into_future();
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => result?,
        () = shutdown_signal() => {
            tracing::info!("shutdown signal received; draining requests");
            shutdown.notify_one();
            if let Some(timeout) = shutdown_timeout {
                tokio::time::timeout(timeout, &mut server)
                    .await
                    .map_err(|_| anyhow::anyhow!("graceful shutdown exceeded configured timeout of {} seconds", timeout.as_secs()))??;
            } else {
                (&mut server).await?;
            }
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(%err, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => tracing::warn!(%err, "failed to install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
