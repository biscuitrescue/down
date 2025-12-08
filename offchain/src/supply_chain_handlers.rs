use crate::chain::{hash_bytes, hash_string};
use crate::error::{format_hash, format_tx_hash, ipfs_gateway_url, ApiError, ApiResult};
use crate::state::AppState;
use alloy::primitives::FixedBytes;
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

// ======================== SHARED RESPONSE STRUCTS ========================

#[derive(Debug, Serialize)]
pub struct TxResponse {
    pub tx_hash: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct TxWithCidResponse {
    pub tx_hash: String,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

// ======================== STAGE 1: FARMER REGISTRATION ========================

#[derive(Debug, Deserialize)]
pub struct RegisterFarmerRequest {
    pub farmer_did: String,
    pub crop_id: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RegisterFarmerResponse {
    pub tx_hash: String,
    pub farmer_did: String,
    pub crop_id_hash: String,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

pub async fn register_farmer(
    State(state): State<AppState>,
    Json(payload): Json<RegisterFarmerRequest>,
) -> ApiResult<RegisterFarmerResponse> {
    tracing::info!(farmer_did = %payload.farmer_did, "Registering farmer");

    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    let farmer_did: FixedBytes<32> = payload
        .farmer_did
        .parse()
        .map_err(|e| ApiError::invalid_did(e))?;

    let crop_id_hash = hash_string(&payload.crop_id);

    let receipt = state
        .chain_client
        .register_farmer(farmer_did, crop_id_hash, metadata_cid.clone())
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(RegisterFarmerResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        farmer_did: payload.farmer_did,
        crop_id_hash: format_hash(crop_id_hash),
        metadata_cid: metadata_cid.clone(),
        ipfs_url: ipfs_gateway_url(&metadata_cid),
    }))
}

// ======================== STAGE 2: FPO PURCHASE ========================

#[derive(Debug, Deserialize)]
pub struct FpoPurchaseRequest {
    pub batch_id: String,
    pub farmer_did: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct FpoPurchaseResponse {
    pub tx_hash: String,
    pub batch_hash: String,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

pub async fn fpo_purchase(
    State(state): State<AppState>,
    Json(payload): Json<FpoPurchaseRequest>,
) -> ApiResult<FpoPurchaseResponse> {
    tracing::info!(batch_id = %payload.batch_id, "Recording FPO purchase");

    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    let batch_hash = hash_string(&payload.batch_id);
    let farmer_did: FixedBytes<32> = payload
        .farmer_did
        .parse()
        .map_err(|e| ApiError::invalid_did(e))?;

    let receipt = state
        .chain_client
        .fpo_purchase(batch_hash, farmer_did, metadata_cid.clone())
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(FpoPurchaseResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        batch_hash: format_hash(batch_hash),
        metadata_cid: metadata_cid.clone(),
        ipfs_url: ipfs_gateway_url(&metadata_cid),
    }))
}

// ======================== STAGE 3: WAREHOUSE STORAGE ========================

#[derive(Debug, Deserialize)]
pub struct WarehouseUpdateRequest {
    pub warehouse_id: String,
    pub iot_data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct WarehouseUpdateResponse {
    pub tx_hash: String,
    pub warehouse_id: String,
    pub state_hash: String,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

pub async fn update_warehouse_state(
    State(state): State<AppState>,
    Json(payload): Json<WarehouseUpdateRequest>,
) -> ApiResult<WarehouseUpdateResponse> {
    tracing::info!(warehouse_id = %payload.warehouse_id, "Updating warehouse state");

    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.iot_data)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    let iot_json = serde_json::to_vec(&payload.iot_data).map_err(ApiError::json_failed)?;
    let state_hash = hash_bytes(&iot_json);
    let warehouse_id = hash_string(&payload.warehouse_id);

    let receipt = state
        .chain_client
        .update_warehouse_state(warehouse_id, state_hash, metadata_cid.clone())
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(WarehouseUpdateResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        warehouse_id: payload.warehouse_id,
        state_hash: format_hash(state_hash),
        metadata_cid: metadata_cid.clone(),
        ipfs_url: ipfs_gateway_url(&metadata_cid),
    }))
}

// Batch warehouse update
#[derive(Debug, Deserialize)]
pub struct BatchWarehouseUpdate {
    pub warehouse_id: String,
    pub iot_data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct BatchWarehouseUpdateRequest {
    pub updates: Vec<BatchWarehouseUpdate>,
}

pub async fn batch_update_warehouse(
    State(state): State<AppState>,
    Json(payload): Json<BatchWarehouseUpdateRequest>,
) -> ApiResult<TxResponse> {
    tracing::info!(count = payload.updates.len(), "Batch updating warehouses");

    let mut warehouse_ids = Vec::with_capacity(payload.updates.len());
    let mut state_hashes = Vec::with_capacity(payload.updates.len());

    for update in &payload.updates {
        let warehouse_id = hash_string(&update.warehouse_id);
        let iot_json = serde_json::to_vec(&update.iot_data).map_err(ApiError::json_failed)?;
        let state_hash = hash_bytes(&iot_json);
        warehouse_ids.push(warehouse_id);
        state_hashes.push(state_hash);
    }

    let receipt = state
        .chain_client
        .batch_update_warehouse(warehouse_ids, state_hashes)
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(TxResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        message: format!("Successfully updated {} warehouses", payload.updates.len()),
    }))
}

// ======================== STAGE 4: LOGISTICS TRACKING ========================

#[derive(Debug, Deserialize)]
pub struct LogisticsUpdateRequest {
    pub shipment_id: String,
    pub location: String,
    pub is_delivered: bool,
    pub gps_data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct LogisticsUpdateResponse {
    pub tx_hash: String,
    pub shipment_id: String,
    pub location_hash: String,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

pub async fn record_logistics(
    State(state): State<AppState>,
    Json(payload): Json<LogisticsUpdateRequest>,
) -> ApiResult<LogisticsUpdateResponse> {
    tracing::info!(shipment_id = %payload.shipment_id, "Recording logistics milestone");

    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.gps_data)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    let shipment_id = hash_string(&payload.shipment_id);
    let location_hash = hash_string(&payload.location);

    let receipt = state
        .chain_client
        .record_logistics(
            shipment_id,
            location_hash,
            payload.is_delivered,
            metadata_cid.clone(),
        )
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(LogisticsUpdateResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        shipment_id: payload.shipment_id,
        location_hash: format_hash(location_hash),
        metadata_cid: metadata_cid.clone(),
        ipfs_url: ipfs_gateway_url(&metadata_cid),
    }))
}

// ======================== STAGE 5: PROCESSING ========================

#[derive(Debug, Deserialize)]
pub struct ProcessBatchRequest {
    pub input_batch_id: String,
    pub output_batch_ids: Vec<String>,
    pub process_metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ProcessBatchResponse {
    pub tx_hash: String,
    pub input_batch_hash: String,
    pub transform_hash: String,
    pub output_batch_hashes: Vec<String>,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

pub async fn process_batch(
    State(state): State<AppState>,
    Json(payload): Json<ProcessBatchRequest>,
) -> ApiResult<ProcessBatchResponse> {
    tracing::info!(input_batch = %payload.input_batch_id, "Processing batch");

    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.process_metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    let input_batch_hash = hash_string(&payload.input_batch_id);
    let output_batch_hashes: Vec<FixedBytes<32>> = payload
        .output_batch_ids
        .iter()
        .map(|id| hash_string(id))
        .collect();

    let metadata_json =
        serde_json::to_vec(&payload.process_metadata).map_err(ApiError::json_failed)?;
    let transform_hash = hash_bytes(&metadata_json);

    let receipt = state
        .chain_client
        .process_batch(
            input_batch_hash,
            transform_hash,
            output_batch_hashes.clone(),
            metadata_cid.clone(),
        )
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(ProcessBatchResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        input_batch_hash: format_hash(input_batch_hash),
        transform_hash: format_hash(transform_hash),
        output_batch_hashes: output_batch_hashes.iter().map(|h| format_hash(h)).collect(),
        metadata_cid: metadata_cid.clone(),
        ipfs_url: ipfs_gateway_url(&metadata_cid),
    }))
}

// ======================== STAGE 6: PACKAGING ========================

#[derive(Debug, Deserialize)]
pub struct CreateSkuRequest {
    pub sku_id: String,
    pub parent_batch_id: String,
    pub unit_ids: Vec<String>,
    pub packaging_metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct CreateSkuResponse {
    pub tx_hash: String,
    pub sku_id: String,
    pub parent_batch_hash: String,
    pub merkle_root: String,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

pub async fn create_sku(
    State(state): State<AppState>,
    Json(payload): Json<CreateSkuRequest>,
) -> ApiResult<CreateSkuResponse> {
    tracing::info!(sku_id = %payload.sku_id, "Creating SKU");

    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.packaging_metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    // Generate Merkle root from unit IDs
    let unit_hashes: Vec<FixedBytes<32>> =
        payload.unit_ids.iter().map(|id| hash_string(id)).collect();

    let mut merkle_data = Vec::with_capacity(unit_hashes.len() * 32);
    for hash in &unit_hashes {
        merkle_data.extend_from_slice(hash.as_slice());
    }
    let merkle_root = hash_bytes(&merkle_data);

    let sku_id = hash_string(&payload.sku_id);
    let parent_batch_hash = hash_string(&payload.parent_batch_id);

    let receipt = state
        .chain_client
        .create_sku(sku_id, parent_batch_hash, merkle_root, metadata_cid.clone())
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(CreateSkuResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        sku_id: payload.sku_id,
        parent_batch_hash: format_hash(parent_batch_hash),
        merkle_root: format_hash(merkle_root),
        metadata_cid: metadata_cid.clone(),
        ipfs_url: ipfs_gateway_url(&metadata_cid),
    }))
}

// ======================== STAGE 7: FRAUD REPORTING ========================

#[derive(Debug, Deserialize)]
pub struct ReportFraudRequest {
    pub sku_id: String,
    pub evidence: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ReportFraudResponse {
    pub tx_hash: String,
    pub sku_id: String,
    pub evidence_hash: String,
    pub evidence_cid: String,
    pub ipfs_url: String,
}

pub async fn report_fraud(
    State(state): State<AppState>,
    Json(payload): Json<ReportFraudRequest>,
) -> ApiResult<ReportFraudResponse> {
    tracing::info!(sku_id = %payload.sku_id, "Reporting fraud");

    let evidence_cid = state
        .ipfs_client
        .upload_json(&payload.evidence)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    let evidence_json = serde_json::to_vec(&payload.evidence).map_err(ApiError::json_failed)?;
    let evidence_hash = hash_bytes(&evidence_json);
    let sku_id = hash_string(&payload.sku_id);

    let receipt = state
        .chain_client
        .report_fraud(sku_id, evidence_hash, evidence_cid.clone())
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(ReportFraudResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        sku_id: payload.sku_id,
        evidence_hash: format_hash(evidence_hash),
        evidence_cid: evidence_cid.clone(),
        ipfs_url: ipfs_gateway_url(&evidence_cid),
    }))
}

// ======================== STAGE 8: AI SCORING ========================

#[derive(Debug, Deserialize)]
pub struct CommitAiScoreRequest {
    pub batch_id: String,
    pub reveal_hash: String,
    pub nonce: String,
}

#[derive(Debug, Serialize)]
pub struct CommitAiScoreResponse {
    pub tx_hash: String,
    pub batch_id: String,
    pub commit_hash: String,
}

pub async fn commit_ai_score(
    State(state): State<AppState>,
    Json(payload): Json<CommitAiScoreRequest>,
) -> ApiResult<CommitAiScoreResponse> {
    tracing::info!(batch_id = %payload.batch_id, "Committing AI score");

    let batch_hash = hash_string(&payload.batch_id);
    let reveal_hash: FixedBytes<32> = payload
        .reveal_hash
        .parse()
        .map_err(|e| ApiError::invalid_hash("reveal hash", e))?;
    let nonce: FixedBytes<32> = payload
        .nonce
        .parse()
        .map_err(|e| ApiError::invalid_hash("nonce", e))?;

    let commit_hash = crate::chain::generate_commit_hash(reveal_hash, nonce);

    let receipt = state
        .chain_client
        .commit_ai_score(batch_hash, commit_hash)
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(CommitAiScoreResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        batch_id: payload.batch_id,
        commit_hash: format_hash(commit_hash),
    }))
}

#[derive(Debug, Deserialize)]
pub struct RevealAiScoreRequest {
    pub batch_id: String,
    pub nonce: String,
    pub score_data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RevealAiScoreResponse {
    pub tx_hash: String,
    pub batch_id: String,
    pub reveal_hash: String,
    pub metadata_cid: String,
    pub ipfs_url: String,
}

pub async fn reveal_ai_score(
    State(state): State<AppState>,
    Json(payload): Json<RevealAiScoreRequest>,
) -> ApiResult<RevealAiScoreResponse> {
    tracing::info!(batch_id = %payload.batch_id, "Revealing AI score");

    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.score_data)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    let score_json = serde_json::to_vec(&payload.score_data).map_err(ApiError::json_failed)?;
    let reveal_hash = hash_bytes(&score_json);

    let batch_hash = hash_string(&payload.batch_id);
    let nonce: FixedBytes<32> = payload
        .nonce
        .parse()
        .map_err(|e| ApiError::invalid_hash("nonce", e))?;

    let receipt = state
        .chain_client
        .reveal_ai_score(batch_hash, reveal_hash, nonce, metadata_cid.clone())
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(RevealAiScoreResponse {
        tx_hash: format_tx_hash(receipt.transaction_hash),
        batch_id: payload.batch_id,
        reveal_hash: format_hash(reveal_hash),
        metadata_cid: metadata_cid.clone(),
        ipfs_url: ipfs_gateway_url(&metadata_cid),
    }))
}

// ======================== VERIFICATION ENDPOINTS ========================

#[derive(Debug, Deserialize)]
pub struct VerifySkuRequest {
    pub sku_id: String,
}

#[derive(Debug, Serialize)]
pub struct VerifySkuResponse {
    pub sku_id: String,
    pub parent_batch_hash: String,
    pub merkle_root: String,
    pub packaged_at: u64,
    pub exists: bool,
}

pub async fn verify_sku(
    State(state): State<AppState>,
    Json(payload): Json<VerifySkuRequest>,
) -> ApiResult<VerifySkuResponse> {
    let sku_id = hash_string(&payload.sku_id);

    let result = state
        .chain_client
        .verify_package_origin(sku_id)
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(VerifySkuResponse {
        sku_id: payload.sku_id,
        parent_batch_hash: format_hash(result.0),
        merkle_root: format_hash(result.1),
        packaged_at: result.2,
        exists: result.2 > 0,
    }))
}

#[derive(Debug, Deserialize)]
pub struct VerifyFarmerRequest {
    pub farmer_did: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyFarmerResponse {
    pub farmer_did: String,
    pub exists: bool,
    pub crop_id_hash: String,
    pub registered_at: u64,
}

pub async fn verify_farmer(
    State(state): State<AppState>,
    Json(payload): Json<VerifyFarmerRequest>,
) -> ApiResult<VerifyFarmerResponse> {
    let farmer_did: FixedBytes<32> = payload
        .farmer_did
        .parse()
        .map_err(|e| ApiError::invalid_did(e))?;

    let result = state
        .chain_client
        .verify_farmer(farmer_did)
        .await
        .map_err(ApiError::blockchain_failed)?;

    Ok(Json(VerifyFarmerResponse {
        farmer_did: payload.farmer_did,
        exists: result.0,
        crop_id_hash: format_hash(result.1),
        registered_at: result.2,
    }))
}
