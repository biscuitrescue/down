use crate::chain::{hash_bytes, hash_string};
use crate::state::AppState;
use alloy::primitives::FixedBytes;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

// ======================== COMMON TYPES ========================

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct TxResponse {
    pub tx_hash: String,
    pub message: String,
}

// ======================== STAGE 1: FARMER REGISTRATION ========================

#[derive(Debug, Deserialize)]
pub struct RegisterFarmerRequest {
    pub farmer_did: String,
    pub crop_id: String,
    pub metadata: serde_json::Value, // Full farmer profile, land docs, etc.
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
) -> Result<Json<RegisterFarmerResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(farmer_did = %payload.farmer_did, "Registering farmer");

    // 1. Upload metadata to IPFS
    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.metadata)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to upload farmer metadata to IPFS");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    tracing::info!(cid = %metadata_cid, "Farmer metadata uploaded to IPFS");

    // 2. Parse farmer DID
    let farmer_did: FixedBytes<32> = payload.farmer_did.parse().map_err(|e| {
        tracing::error!(error = ?e, "Invalid farmer DID format");
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid farmer DID: {}", e),
            }),
        )
    })?;

    // 3. Hash the crop ID
    let crop_id_hash = hash_string(&payload.crop_id);

    // 4. Register on blockchain
    let receipt = state
        .chain_client
        .register_farmer(farmer_did, crop_id_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Blockchain registration failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", metadata_cid);

    tracing::info!(
        tx_hash = ?receipt.transaction_hash,
        metadata_cid = %metadata_cid,
        "Farmer registered successfully"
    );

    Ok(Json(RegisterFarmerResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        farmer_did: payload.farmer_did,
        crop_id_hash: format!("{:?}", crop_id_hash),
        metadata_cid,
        ipfs_url,
    }))
}

// ======================== STAGE 2: FPO PURCHASE ========================

#[derive(Debug, Deserialize)]
pub struct FpoPurchaseRequest {
    pub batch_id: String,
    pub farmer_did: String,
    pub metadata: serde_json::Value, // Quality reports, weight slips, photos
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
) -> Result<Json<FpoPurchaseResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(batch_id = %payload.batch_id, "Recording FPO purchase");

    // 1. Upload metadata to IPFS
    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.metadata)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    // 2. Parse IDs
    let batch_hash = hash_string(&payload.batch_id);
    let farmer_did: FixedBytes<32> = payload.farmer_did.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid farmer DID: {}", e),
            }),
        )
    })?;

    // 3. Record on blockchain
    let receipt = state
        .chain_client
        .fpo_purchase(batch_hash, farmer_did)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", metadata_cid);

    Ok(Json(FpoPurchaseResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        batch_hash: format!("{:?}", batch_hash),
        metadata_cid,
        ipfs_url,
    }))
}

// ======================== STAGE 3: WAREHOUSE STORAGE ========================

#[derive(Debug, Deserialize)]
pub struct WarehouseUpdateRequest {
    pub warehouse_id: String,
    pub iot_data: serde_json::Value, // Temperature, humidity, etc.
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
) -> Result<Json<WarehouseUpdateResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(warehouse_id = %payload.warehouse_id, "Updating warehouse state");

    // 1. Upload IoT data to IPFS
    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.iot_data)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    // 2. Hash the IoT data for on-chain storage
    let iot_json = serde_json::to_vec(&payload.iot_data).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("JSON serialization failed: {}", e),
            }),
        )
    })?;
    let state_hash = hash_bytes(&iot_json);

    // 3. Parse warehouse ID
    let warehouse_id = hash_string(&payload.warehouse_id);

    // 4. Update on blockchain
    let receipt = state
        .chain_client
        .update_warehouse_state(warehouse_id, state_hash)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", metadata_cid);

    Ok(Json(WarehouseUpdateResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        warehouse_id: payload.warehouse_id,
        state_hash: format!("{:?}", state_hash),
        metadata_cid,
        ipfs_url,
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
) -> Result<Json<TxResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(count = payload.updates.len(), "Batch updating warehouses");

    let mut warehouse_ids = Vec::new();
    let mut state_hashes = Vec::new();

    for update in &payload.updates {
        let warehouse_id = hash_string(&update.warehouse_id);
        let iot_json = serde_json::to_vec(&update.iot_data).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("JSON serialization failed: {}", e),
                }),
            )
        })?;
        let state_hash = hash_bytes(&iot_json);

        warehouse_ids.push(warehouse_id);
        state_hashes.push(state_hash);
    }

    let receipt = state
        .chain_client
        .batch_update_warehouse(warehouse_ids, state_hashes)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    Ok(Json(TxResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        message: format!("Successfully updated {} warehouses", payload.updates.len()),
    }))
}

// ======================== STAGE 4: LOGISTICS TRACKING ========================

#[derive(Debug, Deserialize)]
pub struct LogisticsUpdateRequest {
    pub shipment_id: String,
    pub location: String,
    pub is_delivered: bool,
    pub gps_data: serde_json::Value, // GPS trace, photos, signatures
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
) -> Result<Json<LogisticsUpdateResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(shipment_id = %payload.shipment_id, "Recording logistics milestone");

    // 1. Upload GPS data to IPFS
    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.gps_data)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    // 2. Hash location
    let shipment_id = hash_string(&payload.shipment_id);
    let location_hash = hash_string(&payload.location);

    // 3. Record on blockchain
    let receipt = state
        .chain_client
        .record_logistics(shipment_id, location_hash, payload.is_delivered)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", metadata_cid);

    Ok(Json(LogisticsUpdateResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        shipment_id: payload.shipment_id,
        location_hash: format!("{:?}", location_hash),
        metadata_cid,
        ipfs_url,
    }))
}

// ======================== STAGE 5: PROCESSING ========================

#[derive(Debug, Deserialize)]
pub struct ProcessBatchRequest {
    pub input_batch_id: String,
    pub output_batch_ids: Vec<String>,
    pub process_metadata: serde_json::Value, // Yield %, lab results, etc.
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
) -> Result<Json<ProcessBatchResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(input_batch = %payload.input_batch_id, "Processing batch");

    // 1. Upload process metadata to IPFS
    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.process_metadata)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    // 2. Hash batch IDs
    let input_batch_hash = hash_string(&payload.input_batch_id);
    let output_batch_hashes: Vec<FixedBytes<32>> = payload
        .output_batch_ids
        .iter()
        .map(|id| hash_string(id))
        .collect();

    // 3. Create transform hash from metadata
    let metadata_json = serde_json::to_vec(&payload.process_metadata).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("JSON serialization failed: {}", e),
            }),
        )
    })?;
    let transform_hash = hash_bytes(&metadata_json);

    // 4. Record on blockchain
    let receipt = state
        .chain_client
        .process_batch(
            input_batch_hash,
            transform_hash,
            output_batch_hashes.clone(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", metadata_cid);

    Ok(Json(ProcessBatchResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        input_batch_hash: format!("{:?}", input_batch_hash),
        transform_hash: format!("{:?}", transform_hash),
        output_batch_hashes: output_batch_hashes
            .iter()
            .map(|h| format!("{:?}", h))
            .collect(),
        metadata_cid,
        ipfs_url,
    }))
}

// ======================== STAGE 6: PACKAGING ========================

#[derive(Debug, Deserialize)]
pub struct CreateSkuRequest {
    pub sku_id: String,
    pub parent_batch_id: String,
    pub unit_ids: Vec<String>, // Individual unit IDs for Merkle tree
    pub packaging_metadata: serde_json::Value, // Labels, barcodes, etc.
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
) -> Result<Json<CreateSkuResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(sku_id = %payload.sku_id, "Creating SKU");

    // 1. Upload packaging metadata to IPFS
    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.packaging_metadata)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    // 2. Generate Merkle root from unit IDs
    let unit_hashes: Vec<FixedBytes<32>> =
        payload.unit_ids.iter().map(|id| hash_string(id)).collect();

    // Simple Merkle root (hash of all hashes concatenated)
    let mut merkle_data = Vec::new();
    for hash in &unit_hashes {
        merkle_data.extend_from_slice(hash.as_slice());
    }
    let merkle_root = hash_bytes(&merkle_data);

    // 3. Hash IDs
    let sku_id = hash_string(&payload.sku_id);
    let parent_batch_hash = hash_string(&payload.parent_batch_id);

    // 4. Record on blockchain
    let receipt = state
        .chain_client
        .create_sku(sku_id, parent_batch_hash, merkle_root)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", metadata_cid);

    Ok(Json(CreateSkuResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        sku_id: payload.sku_id,
        parent_batch_hash: format!("{:?}", parent_batch_hash),
        merkle_root: format!("{:?}", merkle_root),
        metadata_cid,
        ipfs_url,
    }))
}

// ======================== STAGE 7: FRAUD REPORTING ========================

#[derive(Debug, Deserialize)]
pub struct ReportFraudRequest {
    pub sku_id: String,
    pub evidence: serde_json::Value, // Photos, lab reports, complaints
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
) -> Result<Json<ReportFraudResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(sku_id = %payload.sku_id, "Reporting fraud");

    // 1. Upload evidence to IPFS
    let evidence_cid = state
        .ipfs_client
        .upload_json(&payload.evidence)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    // 2. Hash evidence
    let evidence_json = serde_json::to_vec(&payload.evidence).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("JSON serialization failed: {}", e),
            }),
        )
    })?;
    let evidence_hash = hash_bytes(&evidence_json);

    // 3. Hash SKU ID
    let sku_id = hash_string(&payload.sku_id);

    // 4. Report on blockchain
    let receipt = state
        .chain_client
        .report_fraud(sku_id, evidence_hash)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", evidence_cid);

    Ok(Json(ReportFraudResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        sku_id: payload.sku_id,
        evidence_hash: format!("{:?}", evidence_hash),
        evidence_cid,
        ipfs_url,
    }))
}

// ======================== STAGE 8: AI SCORING ========================

#[derive(Debug, Deserialize)]
pub struct CommitAiScoreRequest {
    pub batch_id: String,
    pub reveal_hash: String, // Hash of the actual score
    pub nonce: String,       // Random nonce for commit-reveal
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
) -> Result<Json<CommitAiScoreResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(batch_id = %payload.batch_id, "Committing AI score");

    // 1. Parse hashes
    let batch_hash = hash_string(&payload.batch_id);
    let reveal_hash: FixedBytes<32> = payload.reveal_hash.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid reveal hash: {}", e),
            }),
        )
    })?;
    let nonce: FixedBytes<32> = payload.nonce.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid nonce: {}", e),
            }),
        )
    })?;

    // 2. Generate commit hash
    let commit_hash = crate::chain::generate_commit_hash(reveal_hash, nonce);

    // 3. Commit on blockchain
    let receipt = state
        .chain_client
        .commit_ai_score(batch_hash, commit_hash)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    Ok(Json(CommitAiScoreResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        batch_id: payload.batch_id,
        commit_hash: format!("{:?}", commit_hash),
    }))
}

#[derive(Debug, Deserialize)]
pub struct RevealAiScoreRequest {
    pub batch_id: String,
    pub nonce: String,
    pub score_data: serde_json::Value, // Full AI model outputs, explainability
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
) -> Result<Json<RevealAiScoreResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(batch_id = %payload.batch_id, "Revealing AI score");

    // 1. Upload score data to IPFS
    let metadata_cid = state
        .ipfs_client
        .upload_json(&payload.score_data)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("IPFS upload failed: {}", e),
                }),
            )
        })?;

    // 2. Hash score data for reveal
    let score_json = serde_json::to_vec(&payload.score_data).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("JSON serialization failed: {}", e),
            }),
        )
    })?;
    let reveal_hash = hash_bytes(&score_json);

    // 3. Parse IDs
    let batch_hash = hash_string(&payload.batch_id);
    let nonce: FixedBytes<32> = payload.nonce.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid nonce: {}", e),
            }),
        )
    })?;

    // 4. Reveal on blockchain
    let receipt = state
        .chain_client
        .reveal_ai_score(batch_hash, reveal_hash, nonce)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Blockchain transaction failed: {}", e),
                }),
            )
        })?;

    let ipfs_url = format!("https://ipfs.io/ipfs/{}", metadata_cid);

    Ok(Json(RevealAiScoreResponse {
        tx_hash: format!("{:?}", receipt.transaction_hash),
        batch_id: payload.batch_id,
        reveal_hash: format!("{:?}", reveal_hash),
        metadata_cid,
        ipfs_url,
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
) -> Result<Json<VerifySkuResponse>, (StatusCode, Json<ErrorResponse>)> {
    let sku_id = hash_string(&payload.sku_id);

    let result = state
        .chain_client
        .verify_package_origin(sku_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Verification failed: {}", e),
                }),
            )
        })?;

    Ok(Json(VerifySkuResponse {
        sku_id: payload.sku_id,
        parent_batch_hash: format!("{:?}", result.0),
        merkle_root: format!("{:?}", result.1),
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
) -> Result<Json<VerifyFarmerResponse>, (StatusCode, Json<ErrorResponse>)> {
    let farmer_did: FixedBytes<32> = payload.farmer_did.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid farmer DID: {}", e),
            }),
        )
    })?;

    let result = state
        .chain_client
        .verify_farmer(farmer_did)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Verification failed: {}", e),
                }),
            )
        })?;

    Ok(Json(VerifyFarmerResponse {
        farmer_did: payload.farmer_did,
        exists: result.0,
        crop_id_hash: format!("{:?}", result.1),
        registered_at: result.2,
    }))
}
