use axum::{routing::get, Router};
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod chain;
mod config;
mod error;
mod ipfs;
mod routes;
mod state;
mod supply_chain_handlers;
mod workflows;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing/logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "offchain=debug,tower_http=debug,axum::rejection=trace".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Load configuration
    let config = Config::from_env()?;

    tracing::info!(
        "Starting Oilseed Value Chain Backend API in {:?} mode on {}",
        config.environment,
        config.address()
    );

    // Initialize application state (blockchain + IPFS clients)
    tracing::info!("Initializing application state...");
    let app_state = AppState::from_env().await?;
    tracing::info!("Application state initialized successfully");

    // Configure CORS
    let cors = if config.environment.is_production() {
        // In production, restrict CORS to specific origins
        // TODO: Configure allowed origins from environment variable
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        // In development, allow all origins
        CorsLayer::permissive()
    };

    // Build the application router
    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .merge(routes::configure_routes(app_state))
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = config.address();
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("🚀 Server listening on http://{}", addr);
    tracing::info!("");
    tracing::info!("📋 API Endpoints:");
    tracing::info!("");
    tracing::info!("🔄 WORKFLOW ORCHESTRATION:");
    tracing::info!("  - POST /api/workflow/execute      - Execute complete supply chain workflow");
    tracing::info!("  - POST /api/workflow/verify-sku   - Verify SKU traceability");
    tracing::info!("  - POST /api/workflow/verify-farmer - Verify farmer registration");
    tracing::info!("");
    tracing::info!("🔗 INDIVIDUAL SUPPLY CHAIN STAGES:");
    tracing::info!("  - POST /api/farmer/register       - Register a new farmer");
    tracing::info!("  - POST /api/farmer/verify         - Verify farmer registration");
    tracing::info!("  - POST /api/fpo/purchase          - Record FPO purchase");
    tracing::info!("  - POST /api/warehouse/update      - Update warehouse state");
    tracing::info!("  - POST /api/warehouse/batch-update - Batch update warehouses");
    tracing::info!("  - POST /api/logistics/record      - Record logistics milestone");
    tracing::info!("  - POST /api/processing/batch      - Process a batch");
    tracing::info!("  - POST /api/packaging/sku         - Create a new SKU");
    tracing::info!("  - POST /api/packaging/verify      - Verify SKU origin");
    tracing::info!("  - POST /api/fraud/report          - Report fraud");
    tracing::info!("  - POST /api/ai/commit             - Commit AI score");
    tracing::info!("  - POST /api/ai/reveal             - Reveal AI score");
    tracing::info!("  - POST /api/ipfs/upload           - Upload data to IPFS");
    tracing::info!("");
    tracing::info!("📚 See WORKFLOW.md for complete integration guide");
    tracing::info!("💡 Use /api/workflow/execute for end-to-end automation");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> &'static str {
    "Oilseed Value Chain Backend API v0.1.0 - All systems operational"
}

async fn health_check() -> &'static str {
    "OK"
}
