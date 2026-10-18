use axum::{
    http::{HeaderValue, StatusCode},
    routing::get,
    Router,
};
use oilschain::{config::Config, reconciler, routes, state::AppState};
use std::time::Duration;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing/logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "oilschain=info,oilschain_backend=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Load configuration
    let config = Config::from_env()?;

    tracing::info!(
        "Starting OilsChain backend in {:?} mode on {}",
        config.environment,
        config.address()
    );

    // Configure CORS. Production: only the origins listed in CORS_ALLOWED_ORIGINS (none = same-origin only).
    let cors = if config.environment.is_production() {
        let origins = config
            .cors_allowed_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect::<Vec<_>>();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        // In development, allow all origins
        CorsLayer::permissive()
    };
    let frontend_dir = config.frontend_dir.clone();
    let addr = config.address();

    // Initialize application state (blockchain + IPFS clients + database)
    let app_state = AppState::initialize(config).await?;
    reconciler::spawn(app_state.clone());

    // Build the application router
    let mut app = Router::new()
        .route("/health", get(health_check))
        .merge(routes::configure_routes(app_state));
    if let Some(dir) = frontend_dir {
        tracing::info!(dir = %dir.display(), "serving frontend");
        let index = dir.join("index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }
    let app = app
        .layer(cors)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(120),
        ))
        .layer(ConcurrencyLimitLayer::new(64))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("🚀 Server listening on http://{}", addr);
    tracing::info!("📚 API reference: docs/API.md");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> &'static str {
    "OK"
}
