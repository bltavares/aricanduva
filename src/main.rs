use axum::{Router, routing::get};
use bytes::Bytes;
use conf::Conf;
use dashmap::DashMap;
use rand::distr::SampleString;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::signal;
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing::Level;

mod cli;
mod database;
mod info;
mod ipfs;
mod limited_slots;
mod s3;

use crate::cli::{CliOperations, RunConfig};
use crate::info::health_check;
use crate::ipfs::IpfsClient;

struct App {
    db: database::Database,
    ipfs_client: IpfsClient,
    config: RunConfig,
    multipart_slots: limited_slots::LimitedSlotsMap<String, DashMap<i8, Bytes>>,
}

type AppState = Arc<App>;

/// Signal for graceful shutdown
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http::trace=info".into()),
        )
        .compact()
        .init();

    let cli = cli::Cli::parse();
    let config = match cli.command {
        Some(CliOperations::Config(config)) => {
            println!("{config:#?}");
            std::process::exit(0);
        }
        Some(CliOperations::Credentials) => {
            let mut rng = rand::rng();
            let access_key = rand::distr::Alphanumeric
                .sample_string(&mut rng, 8)
                .to_uppercase();
            let secret_key = rand::distr::Alphanumeric
                .sample_string(&mut rng, 16)
                .to_uppercase();
            println!("AUTH_ACCESS_KEY={}", access_key.to_uppercase());
            println!("AUTH_SECRET_KEY={}", secret_key.to_uppercase());
            std::process::exit(0);
        }
        Some(CliOperations::Run(config)) => config,
        _ => cli.config,
    };

    run(config).await;
}

async fn run(config: RunConfig) {
    tracing::debug!(config = ?config, "Loaded configuration");

    if config.auth.is_none() {
        tracing::warn!(
            "Running service without credentials is not recomended if the service is exposed to the internet"
        );
    }

    // Initialize database before starting the server
    let db = match database::Database::initialize(&config.database_path, &config.sqlite).await {
        Ok(db) => {
            tracing::info!("Database initialized successfully");
            db
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to initialize database");
            std::process::exit(1);
        }
    };

    let ipfs_client = IpfsClient::new_with_config(
        config.rpc_address.clone(),
        config.rpc_credentials.clone().map(Into::into),
    );

    let app_state = Arc::new(App {
        db,
        ipfs_client,
        config: config.clone(),
        multipart_slots: limited_slots::LimitedSlotsMap::with_capacity(
            config.concurrent_multipart_upload,
        ),
    });

    let app = Router::new()
        .route("/healthz", get(health_check))
        .merge(s3::routes(&config))
        .with_state(app_state.clone())
        .layer(config.ip_extraction.clone().into_extension())
        .layer(CompressionLayer::new())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(tower_http::trace::DefaultMakeSpan::new().level(Level::INFO))
                .on_response(tower_http::trace::DefaultOnResponse::new().level(Level::INFO)),
        );

    let listener = config.listen_socket().await;
    tracing::info!(?config.mode, "Service started");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        let wait = Duration::from_secs(3);
        let _ = tokio::time::timeout(wait, async {
            tracing::info!(
                seconds = &wait.as_secs(),
                "Graceful shutdown received, closing db",
            );
            app_state.db.pool.close().await;
        })
        .await;
    })
    .await
    .expect("Failed to start axum::serve");

    tracing::info!("Server shutdown complete");
}
