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

    /// Override [server].port from the config.
    #[arg(long, env = "ARTUR_PORT")]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "artur=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let mut config = load_config(&cli.config).await?;
    if let Some(port) = cli.port {
        config.server.port = port;
    }

    let bind = config.server.bind.clone();
    let port = config.server.port;
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
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
