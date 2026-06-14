use artur::load_config;
use clap::Parser;
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(author, version, about = "Config-driven HTTP server")]
struct Cli {
    /// TOML configuration file path or http(s) URL.
    #[arg(long, env = "ARTUR_CONFIG")]
    config: String,

    /// Override [artur.server].port from the config.
    #[arg(long, env = "ARTUR_PORT")]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut config = load_config(&cli.config).await?;
    let default_filter = config
        .log
        .level
        .clone()
        .unwrap_or_else(|| "artur=info,tower_http=info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| default_filter.into()),
        )
        .init();
    if let Some(port) = cli.port {
        config.artur.server.port = Some(port);
    }

    let server = config.server_config();
    let bind = server.bind.clone();
    let port = server.port;
    let app = artur::build_router(config).await?;
    let addr: SocketAddr = format!("{bind}:{port}").parse()?;
    tracing::info!(%addr, "starting artur server");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
