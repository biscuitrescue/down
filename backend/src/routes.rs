use crate::chain::{hex32, hexaddr};
use crate::error::ApiError;
use crate::state::AppState;
use crate::{
    advisory, ipfs, nft, ops, participants, ranking, supply_chain_handlers as sc, verify, workflows,
};
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// JSON request bodies are small; attachments go through /api/ipfs/upload with a larger limit.
pub const JSON_BODY_LIMIT: usize = 256 * 1024;

pub fn configure_routes(state: AppState) -> Router {
    Router::new()
        // ==================== META ====================
        .route("/api/health", get(health))
        .route("/api/config", get(public_config))
        // ==================== PARTICIPANTS ====================
        .route(
            "/api/admin/participants",
            post(participants::create_participant),
        )
        .route("/api/participants", get(participants::list_participants))
        .route("/api/me", get(participants::me))
        // ==================== WORKFLOW ROUTES ====================
        // Complete end-to-end workflow (admin orchestration of custodial participants; resumable)
        .route("/api/workflow/execute", post(workflows::execute_workflow))
        .route(
            "/api/workflow/verify-sku",
            post(workflows::verify_sku_handler),
        )
        .route("/api/workflow/verify-farmer", post(verify_farmer_body))
        // ==================== SUPPLY CHAIN ROUTES ====================
        // Stage 1: Farmer Registration
        .route("/api/farmer/register", post(sc::register_farmer))
        .route("/api/farmer/verify", post(verify_farmer_body))
        .route("/api/farmers", get(sc::list_farmers))
        .route("/api/farmers/{id}", get(sc::get_farmer))
        .route("/api/farmers/{id}/verify", get(verify_farmer))
        // Stage 2: FPO Purchase (creates the batch)
        .route("/api/fpo/purchase", post(sc::fpo_purchase))
        // Custody
        .route("/api/custody/transfer", post(sc::transfer_custody))
        // Stage 3: Warehouse Storage / inspection (`update` kept as an alias)
        .route("/api/warehouse/inspection", post(sc::warehouse_inspection))
        .route("/api/warehouse/update", post(sc::warehouse_inspection))
        // Stage 4: Logistics Tracking
        .route("/api/logistics/record", post(sc::record_logistics))
        // Stage 5: Processing
        .route("/api/processing/batch", post(sc::process_batch))
        // Stage 6: Packaging
        .route("/api/packaging/sku", post(sc::create_sku))
        .route("/api/packaging/verify", post(workflows::verify_sku_handler))
        // Stage 7: Fraud Reporting and review
        .route("/api/fraud/report", post(sc::report_fraud))
        .route("/api/fraud/resolve", post(sc::resolve_fraud))
        // Stage 8: AI assessment commit–reveal
        .route("/api/ai/commit", post(sc::commit_ai_score))
        .route("/api/ai/reveal", post(sc::reveal_ai_score))
        .route("/api/ai/{batch_id}", get(sc::ai_status))
        // ==================== RECORDS / VERIFICATION ====================
        .route("/api/operations/{id}", get(ops::get_operation))
        .route("/api/batches", get(sc::list_batches))
        .route("/api/batches/{id}", get(sc::get_batch))
        .route(
            "/api/batches/{id}/provenance",
            get(verify::batch_provenance),
        )
        .route("/api/skus/{id}/verify", get(verify::verify_sku))
        .route("/api/records/{digest}/verify", get(verify::verify_record))
        // ==================== NFTs ====================
        .route("/api/nft/mint-from-tx", post(nft::mint_from_tx))
        .route("/api/nft/credential", post(nft::issue_credential))
        .route(
            "/api/nft/provenance/{token_id}",
            get(nft::get_provenance_token),
        )
        .route(
            "/api/nft/credential/{token_id}",
            get(nft::get_credential_token),
        )
        .route("/api/nfts", get(nft::list_nfts))
        // ==================== AI ====================
        .route("/api/rankings/farmers", get(ranking::rank_farmers))
        .route("/api/rankings/farmers/{id}", get(ranking::farmer_ranking))
        .route("/api/advisory", post(advisory::advise))
        .route("/api/advisory/status", get(advisory::status))
        .layer(DefaultBodyLimit::max(JSON_BODY_LIMIT))
        // ==================== IPFS ROUTES ====================
        .route(
            "/api/ipfs/upload",
            post(ipfs::upload_to_ipfs).layer(DefaultBodyLimit::max(ipfs::MAX_BLOCK_SIZE)),
        )
        .route("/api/ipfs/pins/{cid}", get(crate::pinning::pins_for_cid))
        .route("/api/admin/pins", get(crate::pinning::list_pins))
        // Add state to all routes
        .with_state(state)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    chain: Value,
    ipfs: Value,
    database: Value,
}

/// GET /api/health — checks each dependency and reports it individually.
async fn health(State(state): State<AppState>) -> (axum::http::StatusCode, Json<Health>) {
    let chain =
        match alloy::providers::Provider::get_block_number(state.chain_client.provider()).await {
            Ok(n) => json!({"ok": true, "chain_id": state.chain_client.chain_id, "block": n}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        };
    let ipfs = match state.ipfs_client.version().await {
        Ok(v) => json!({"ok": true, "version": v}),
        Err(e) => json!({"ok": false, "error": e.to_string()}),
    };
    let database = match state
        .db
        .call(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)))
        .await
    {
        Ok(_) => json!({"ok": true}),
        Err(e) => json!({"ok": false, "error": e.to_string()}),
    };
    let ok = chain["ok"] == true && ipfs["ok"] == true && database["ok"] == true;
    let code = if ok {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(Health {
            status: if ok { "ok" } else { "degraded" },
            chain,
            ipfs,
            database,
        }),
    )
}

/// GET /api/config — public deployment info for the frontend and third-party verifiers.
async fn public_config(State(state): State<AppState>) -> Json<Value> {
    let c = &state.chain_client;
    Json(json!({
        "chain_id": c.chain_id,
        "registry": hexaddr(c.registry_address),
        "provenance_nft": hexaddr(c.provenance_address),
        "credential_nft": hexaddr(c.credential_address),
        "ipfs_gateway": state.config.ipfs.gateway_url,
        "record_schema": crate::canonical::RECORD_SCHEMA,
        "ranking_model": format!("{}/{}", ranking::MODEL_NAME, ranking::MODEL_VERSION),
        "advisory_configured": state.config.advisory.base_url.is_some(),
        "crops": sc::OILSEED_CROPS,
    }))
}

/// GET /api/farmers/{id}/verify — verifies the (encrypted) registration record; discloses nothing from it.
async fn verify_farmer(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<verify::RecordVerification>, ApiError> {
    let farmer_id = sc::parse_id("farmer_id", &id)?;
    let logs = state
        .chain_client
        .record_logs(farmer_id)
        .await
        .map_err(|e| ApiError::Chain(e.to_string()))?;
    let log = logs
        .iter()
        .find(|l| l.record_type == crate::canonical::RecordType::FarmerRegistration.as_u8())
        .ok_or_else(|| {
            ApiError::NotFound(format!("farmer {} is not registered", hex32(farmer_id)))
        })?;
    Ok(Json(
        verify::verify_log(&state, log, &Default::default()).await,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FarmerBody {
    farmer_id: String,
}

async fn verify_farmer_body(
    state: State<AppState>,
    Json(b): Json<FarmerBody>,
) -> Result<Json<verify::RecordVerification>, ApiError> {
    verify_farmer(state, Path(b.farmer_id)).await
}
