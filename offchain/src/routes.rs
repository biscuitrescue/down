use crate::supply_chain_handlers;
use axum::{routing::post, Router};

pub fn configure_routes(state: crate::state::AppState) -> Router {
    Router::new()
        // ==================== SUPPLY CHAIN ROUTES ====================
        // Stage 1: Farmer Registration
        .route(
            "/api/farmer/register",
            post(supply_chain_handlers::register_farmer),
        )
        .route(
            "/api/farmer/verify",
            post(supply_chain_handlers::verify_farmer),
        )
        // Stage 2: FPO Purchase
        .route(
            "/api/fpo/purchase",
            post(supply_chain_handlers::fpo_purchase),
        )
        // Stage 3: Warehouse Storage
        .route(
            "/api/warehouse/update",
            post(supply_chain_handlers::update_warehouse_state),
        )
        .route(
            "/api/warehouse/batch-update",
            post(supply_chain_handlers::batch_update_warehouse),
        )
        // Stage 4: Logistics Tracking
        .route(
            "/api/logistics/record",
            post(supply_chain_handlers::record_logistics),
        )
        // Stage 5: Processing
        .route(
            "/api/processing/batch",
            post(supply_chain_handlers::process_batch),
        )
        // Stage 6: Packaging
        .route(
            "/api/packaging/sku",
            post(supply_chain_handlers::create_sku),
        )
        .route(
            "/api/packaging/verify",
            post(supply_chain_handlers::verify_sku),
        )
        // Stage 7: Fraud Reporting
        .route(
            "/api/fraud/report",
            post(supply_chain_handlers::report_fraud),
        )
        // Stage 8: AI Scoring
        .route(
            "/api/ai/commit",
            post(supply_chain_handlers::commit_ai_score),
        )
        .route(
            "/api/ai/reveal",
            post(supply_chain_handlers::reveal_ai_score),
        )
        // ==================== IPFS ROUTES ====================
        .route("/api/ipfs/upload", post(crate::ipfs::upload_to_ipfs))
        // Add state to all routes
        .with_state(state)
}
