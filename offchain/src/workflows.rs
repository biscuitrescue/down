//! Supply Chain Workflow Orchestration
//!
//! This module provides high-level workflow functions that orchestrate
//! multiple supply chain operations in sequence, demonstrating how all
//! the components work together from farmer registration to retail.
//!
//! # Workflow Stages
//!
//! 1. Farmer Registration → Register farmer with metadata
//! 2. FPO Purchase → Record crop purchase from farmer
//! 3. Warehouse Storage → Monitor and record storage conditions
//! 4. Logistics → Track shipment from warehouse to processor
//! 5. Processing → Transform raw crops into products
//! 6. Packaging → Create retail SKUs
//! 7. Retail Distribution → SKU verification for consumers
//! 8. Quality Scoring → AI-based quality assessment
//!
//! # Usage Example
//!
//! ```rust,ignore
//! use workflows::SupplyChainWorkflow;
//!
//! let workflow = SupplyChainWorkflow::new(app_state);
//!
//! // Execute complete workflow
//! let result = workflow.execute_full_workflow(workflow_data).await?;
//! ```

use crate::chain::{generate_commit_hash, hash_string};
use crate::state::AppState;
use alloy::primitives::FixedBytes;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
//                         WORKFLOW DATA STRUCTURES
// ============================================================================

/// Complete workflow data for end-to-end supply chain tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteWorkflowData {
    // Stage 1: Farmer
    pub farmer: FarmerData,
    // Stage 2: FPO
    pub fpo_purchase: FpoPurchaseData,
    // Stage 3: Warehouse
    pub warehouse: WarehouseData,
    // Stage 4: Logistics
    pub logistics: LogisticsData,
    // Stage 5: Processing
    pub processing: ProcessingData,
    // Stage 6: Packaging
    pub packaging: PackagingData,
    // Stage 8: AI Scoring
    pub ai_scoring: Option<AiScoringData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmerData {
    pub farmer_did: String,
    pub crop_id: String,
    pub name: String,
    pub location: String,
    pub land_area: String,
    pub crops: Vec<String>,
    pub contact: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FpoPurchaseData {
    pub batch_id: String,
    pub quantity_kg: f64,
    pub quality_grade: String,
    pub moisture_content: String,
    pub purchase_price: f64,
    pub purchase_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarehouseData {
    pub warehouse_id: String,
    pub temperature_celsius: f64,
    pub humidity_percent: f64,
    pub storage_duration_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogisticsData {
    pub shipment_id: String,
    pub origin: String,
    pub destination: String,
    pub checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub location: String,
    pub latitude: f64,
    pub longitude: f64,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingData {
    pub input_batch_id: String,
    pub process_type: String,
    pub yield_percentage: f64,
    pub output_products: Vec<OutputProduct>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputProduct {
    pub product_id: String,
    pub product_type: String,
    pub quantity_kg: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagingData {
    pub sku_prefix: String,
    pub package_type: String,
    pub units_per_package: u32,
    pub total_packages: u32,
    pub expiry_months: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiScoringData {
    pub quality_score: f64,
    pub freshness_score: f64,
    pub purity_score: f64,
    pub model_version: String,
}

// ============================================================================
//                         WORKFLOW RESULTS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    pub farmer_tx: String,
    pub fpo_tx: String,
    pub warehouse_tx: String,
    pub logistics_txs: Vec<String>,
    pub processing_tx: String,
    pub packaging_txs: Vec<String>,
    pub ai_commit_tx: Option<String>,
    pub ai_reveal_tx: Option<String>,
    pub ipfs_cids: WorkflowIpfsCids,
    pub final_skus: Vec<String>,
    pub summary: WorkflowSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowIpfsCids {
    pub farmer_metadata: String,
    pub fpo_metadata: String,
    pub warehouse_metadata: String,
    pub logistics_metadata: Vec<String>,
    pub processing_metadata: String,
    pub packaging_metadata: Vec<String>,
    pub ai_metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub total_stages: u32,
    pub successful_stages: u32,
    pub total_transactions: u32,
    pub total_ipfs_uploads: u32,
    pub workflow_duration_secs: u64,
    pub trace_path: String,
}

// ============================================================================
//                         WORKFLOW ORCHESTRATOR
// ============================================================================

pub struct SupplyChainWorkflow {
    state: AppState,
}

impl SupplyChainWorkflow {
    /// Create new workflow orchestrator
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Execute complete supply chain workflow from farmer to retail
    pub async fn execute_full_workflow(
        &self,
        data: CompleteWorkflowData,
    ) -> Result<WorkflowResult> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        tracing::info!("🚀 Starting complete supply chain workflow");

        // Track all transactions and CIDs
        let mut result = WorkflowResult {
            farmer_tx: String::new(),
            fpo_tx: String::new(),
            warehouse_tx: String::new(),
            logistics_txs: Vec::new(),
            processing_tx: String::new(),
            packaging_txs: Vec::new(),
            ai_commit_tx: None,
            ai_reveal_tx: None,
            ipfs_cids: WorkflowIpfsCids {
                farmer_metadata: String::new(),
                fpo_metadata: String::new(),
                warehouse_metadata: String::new(),
                logistics_metadata: Vec::new(),
                processing_metadata: String::new(),
                packaging_metadata: Vec::new(),
                ai_metadata: None,
            },
            final_skus: Vec::new(),
            summary: WorkflowSummary {
                total_stages: 7,
                successful_stages: 0,
                total_transactions: 0,
                total_ipfs_uploads: 0,
                workflow_duration_secs: 0,
                trace_path: String::new(),
            },
        };

        // Stage 1: Farmer Registration
        tracing::info!("📝 Stage 1/7: Farmer Registration");
        let (farmer_tx, farmer_cid) = self.register_farmer(&data.farmer).await?;
        result.farmer_tx = farmer_tx;
        result.ipfs_cids.farmer_metadata = farmer_cid;
        result.summary.successful_stages += 1;
        result.summary.total_transactions += 1;
        result.summary.total_ipfs_uploads += 1;

        // Stage 2: FPO Purchase
        tracing::info!("🏢 Stage 2/7: FPO Purchase");
        let (fpo_tx, fpo_cid) = self
            .record_fpo_purchase(&data.farmer.farmer_did, &data.fpo_purchase)
            .await?;
        result.fpo_tx = fpo_tx;
        result.ipfs_cids.fpo_metadata = fpo_cid;
        result.summary.successful_stages += 1;
        result.summary.total_transactions += 1;
        result.summary.total_ipfs_uploads += 1;

        // Stage 3: Warehouse Storage
        tracing::info!("🏭 Stage 3/7: Warehouse Storage");
        let (warehouse_tx, warehouse_cid) = self.record_warehouse_storage(&data.warehouse).await?;
        result.warehouse_tx = warehouse_tx;
        result.ipfs_cids.warehouse_metadata = warehouse_cid;
        result.summary.successful_stages += 1;
        result.summary.total_transactions += 1;
        result.summary.total_ipfs_uploads += 1;

        // Stage 4: Logistics Tracking
        tracing::info!("🚚 Stage 4/7: Logistics Tracking");
        let (logistics_txs, logistics_cids) =
            self.record_logistics_journey(&data.logistics).await?;
        result.logistics_txs = logistics_txs.clone();
        result.ipfs_cids.logistics_metadata = logistics_cids;
        result.summary.successful_stages += 1;
        result.summary.total_transactions += logistics_txs.len() as u32;
        result.summary.total_ipfs_uploads += data.logistics.checkpoints.len() as u32;

        // Stage 5: Processing
        tracing::info!("⚙️ Stage 5/7: Batch Processing");
        let (processing_tx, processing_cid, output_batches) = self
            .process_batch(&data.fpo_purchase.batch_id, &data.processing)
            .await?;
        result.processing_tx = processing_tx;
        result.ipfs_cids.processing_metadata = processing_cid;
        result.summary.successful_stages += 1;
        result.summary.total_transactions += 1;
        result.summary.total_ipfs_uploads += 1;

        // Stage 6: Packaging
        tracing::info!("📦 Stage 6/7: SKU Packaging");
        let (packaging_txs, packaging_cids, skus) = self
            .create_retail_packages(&output_batches[0], &data.packaging)
            .await?;
        result.packaging_txs = packaging_txs;
        result.ipfs_cids.packaging_metadata = packaging_cids;
        result.final_skus = skus;
        result.summary.successful_stages += 1;
        result.summary.total_transactions += data.packaging.total_packages;
        result.summary.total_ipfs_uploads += data.packaging.total_packages;

        // Stage 7: AI Quality Scoring (Optional)
        if let Some(ai_data) = &data.ai_scoring {
            tracing::info!("🤖 Stage 7/7: AI Quality Scoring");
            let (commit_tx, reveal_tx, ai_cid) = self
                .execute_ai_scoring(&data.fpo_purchase.batch_id, ai_data)
                .await?;
            result.ai_commit_tx = Some(commit_tx);
            result.ai_reveal_tx = Some(reveal_tx);
            result.ipfs_cids.ai_metadata = Some(ai_cid);
            result.summary.total_transactions += 2;
            result.summary.total_ipfs_uploads += 1;
        }

        // Calculate workflow duration
        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        result.summary.workflow_duration_secs = end_time - start_time;

        // Build trace path
        result.summary.trace_path = format!(
            "Farmer({}) → FPO({}) → Warehouse({}) → Logistics({}) → Processing({}) → Packaging({} SKUs)",
            data.farmer.farmer_did,
            data.fpo_purchase.batch_id,
            data.warehouse.warehouse_id,
            data.logistics.shipment_id,
            data.processing.input_batch_id,
            result.final_skus.len()
        );

        tracing::info!(
            "✅ Workflow completed successfully in {}s",
            result.summary.workflow_duration_secs
        );
        tracing::info!(
            "📊 Summary: {} stages, {} transactions, {} IPFS uploads",
            result.summary.successful_stages,
            result.summary.total_transactions,
            result.summary.total_ipfs_uploads
        );

        Ok(result)
    }

    // ========================================================================
    //                      INDIVIDUAL STAGE METHODS
    // ========================================================================

    async fn register_farmer(&self, data: &FarmerData) -> Result<(String, String)> {
        // Prepare metadata
        let metadata = serde_json::json!({
            "name": data.name,
            "location": data.location,
            "land_area": data.land_area,
            "crops": data.crops,
            "contact": data.contact,
            "registration_timestamp": chrono::Utc::now().to_rfc3339()
        });

        // Upload to IPFS
        let cid = self
            .state
            .ipfs_client
            .upload_json(&metadata)
            .await
            .context("Failed to upload farmer metadata to IPFS")?;

        // Parse farmer DID
        let farmer_did: FixedBytes<32> = data
            .farmer_did
            .parse()
            .context("Invalid farmer DID format")?;

        // Hash crop ID
        let crop_id_hash = hash_string(&data.crop_id);

        // Register on blockchain
        let receipt = self
            .state
            .chain_client
            .register_farmer(farmer_did, crop_id_hash, cid.clone())
            .await
            .context("Blockchain registration failed")?;

        Ok((format!("{:?}", receipt.transaction_hash), cid))
    }

    async fn record_fpo_purchase(
        &self,
        farmer_did: &str,
        data: &FpoPurchaseData,
    ) -> Result<(String, String)> {
        // Prepare metadata
        let metadata = serde_json::json!({
            "batch_id": data.batch_id,
            "quantity_kg": data.quantity_kg,
            "quality_grade": data.quality_grade,
            "moisture_content": data.moisture_content,
            "purchase_price": data.purchase_price,
            "purchase_date": data.purchase_date,
            "verification_timestamp": chrono::Utc::now().to_rfc3339()
        });

        // Upload to IPFS
        let cid = self
            .state
            .ipfs_client
            .upload_json(&metadata)
            .await
            .context("Failed to upload FPO metadata to IPFS")?;

        // Hash batch ID
        let batch_hash = hash_string(&data.batch_id);
        let farmer_did: FixedBytes<32> = farmer_did.parse().context("Invalid farmer DID")?;

        // Record on blockchain
        let receipt = self
            .state
            .chain_client
            .fpo_purchase(batch_hash, farmer_did, cid.clone())
            .await
            .context("Blockchain FPO purchase failed")?;

        Ok((format!("{:?}", receipt.transaction_hash), cid))
    }

    async fn record_warehouse_storage(&self, data: &WarehouseData) -> Result<(String, String)> {
        // Prepare IoT data
        let iot_data = serde_json::json!({
            "warehouse_id": data.warehouse_id,
            "temperature_celsius": data.temperature_celsius,
            "humidity_percent": data.humidity_percent,
            "storage_duration_days": data.storage_duration_days,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "sensor_status": "operational"
        });

        // Upload to IPFS
        let cid = self
            .state
            .ipfs_client
            .upload_json(&iot_data)
            .await
            .context("Failed to upload warehouse data to IPFS")?;

        // Hash warehouse state
        let warehouse_id = hash_string(&data.warehouse_id);
        let iot_json = serde_json::to_vec(&iot_data)?;
        let state_hash = crate::chain::hash_bytes(&iot_json);

        // Update on blockchain
        let receipt = self
            .state
            .chain_client
            .update_warehouse_state(warehouse_id, state_hash, cid.clone())
            .await
            .context("Blockchain warehouse update failed")?;

        Ok((format!("{:?}", receipt.transaction_hash), cid))
    }

    async fn record_logistics_journey(
        &self,
        data: &LogisticsData,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let mut txs = Vec::new();
        let mut cids = Vec::new();

        for (idx, checkpoint) in data.checkpoints.iter().enumerate() {
            let is_delivered = idx == data.checkpoints.len() - 1;

            // Prepare GPS data
            let gps_data = serde_json::json!({
                "shipment_id": data.shipment_id,
                "checkpoint": idx + 1,
                "total_checkpoints": data.checkpoints.len(),
                "location": checkpoint.location,
                "coordinates": {
                    "latitude": checkpoint.latitude,
                    "longitude": checkpoint.longitude
                },
                "timestamp": checkpoint.timestamp,
                "is_delivered": is_delivered
            });

            // Upload to IPFS
            let cid = self
                .state
                .ipfs_client
                .upload_json(&gps_data)
                .await
                .context("Failed to upload logistics data to IPFS")?;

            // Hash location
            let shipment_id = hash_string(&data.shipment_id);
            let location_hash = hash_string(&checkpoint.location);

            // Record on blockchain
            let receipt = self
                .state
                .chain_client
                .record_logistics(shipment_id, location_hash, is_delivered, cid.clone())
                .await
                .context("Blockchain logistics record failed")?;

            txs.push(format!("{:?}", receipt.transaction_hash));
            cids.push(cid);
        }

        Ok((txs, cids))
    }

    async fn process_batch(
        &self,
        input_batch_id: &str,
        data: &ProcessingData,
    ) -> Result<(String, String, Vec<String>)> {
        // Prepare processing metadata
        let metadata = serde_json::json!({
            "input_batch_id": input_batch_id,
            "process_type": data.process_type,
            "yield_percentage": data.yield_percentage,
            "outputs": data.output_products,
            "processing_timestamp": chrono::Utc::now().to_rfc3339()
        });

        // Upload to IPFS
        let cid = self
            .state
            .ipfs_client
            .upload_json(&metadata)
            .await
            .context("Failed to upload processing data to IPFS")?;

        // Hash batch IDs
        let input_hash = hash_string(input_batch_id);
        let output_hashes: Vec<FixedBytes<32>> = data
            .output_products
            .iter()
            .map(|p| hash_string(&p.product_id))
            .collect();

        let metadata_json = serde_json::to_vec(&metadata)?;
        let transform_hash = crate::chain::hash_bytes(&metadata_json);

        // Record on blockchain
        let receipt = self
            .state
            .chain_client
            .process_batch(
                input_hash,
                transform_hash,
                output_hashes.clone(),
                cid.clone(),
            )
            .await
            .context("Blockchain processing failed")?;

        let output_ids: Vec<String> = data
            .output_products
            .iter()
            .map(|p| p.product_id.clone())
            .collect();

        Ok((format!("{:?}", receipt.transaction_hash), cid, output_ids))
    }

    async fn create_retail_packages(
        &self,
        parent_batch_id: &str,
        data: &PackagingData,
    ) -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
        let mut txs = Vec::new();
        let mut cids = Vec::new();
        let mut skus = Vec::new();

        for package_num in 1..=data.total_packages {
            let sku_id = format!("{}-{:04}", data.sku_prefix, package_num);

            // Generate unit IDs for this package
            let unit_ids: Vec<String> = (1..=data.units_per_package)
                .map(|unit| format!("{}-U{:03}", sku_id, unit))
                .collect();

            // Prepare packaging metadata
            let metadata = serde_json::json!({
                "sku_id": sku_id,
                "parent_batch": parent_batch_id,
                "package_type": data.package_type,
                "units_count": data.units_per_package,
                "unit_ids": unit_ids,
                "expiry_date": format!(
                    "{}",
                    chrono::Utc::now() + chrono::Duration::days((data.expiry_months * 30) as i64)
                ),
                "packaging_timestamp": chrono::Utc::now().to_rfc3339()
            });

            // Upload to IPFS
            let cid = self
                .state
                .ipfs_client
                .upload_json(&metadata)
                .await
                .context("Failed to upload packaging data to IPFS")?;

            // Generate Merkle root
            let unit_hashes: Vec<FixedBytes<32>> =
                unit_ids.iter().map(|id| hash_string(id)).collect();
            let mut merkle_data = Vec::new();
            for hash in &unit_hashes {
                merkle_data.extend_from_slice(hash.as_slice());
            }
            let merkle_root = crate::chain::hash_bytes(&merkle_data);

            // Hash IDs
            let sku_hash = hash_string(&sku_id);
            let parent_hash = hash_string(parent_batch_id);

            // Record on blockchain
            let receipt = self
                .state
                .chain_client
                .create_sku(sku_hash, parent_hash, merkle_root, cid.clone())
                .await
                .context("Blockchain SKU creation failed")?;

            txs.push(format!("{:?}", receipt.transaction_hash));
            cids.push(cid);
            skus.push(sku_id);
        }

        Ok((txs, cids, skus))
    }

    async fn execute_ai_scoring(
        &self,
        batch_id: &str,
        data: &AiScoringData,
    ) -> Result<(String, String, String)> {
        // Prepare AI score data
        let score_data = serde_json::json!({
            "batch_id": batch_id,
            "quality_score": data.quality_score,
            "freshness_score": data.freshness_score,
            "purity_score": data.purity_score,
            "overall_score": (data.quality_score + data.freshness_score + data.purity_score) / 3.0,
            "model_version": data.model_version,
            "evaluation_timestamp": chrono::Utc::now().to_rfc3339()
        });

        // Upload to IPFS
        let cid = self
            .state
            .ipfs_client
            .upload_json(&score_data)
            .await
            .context("Failed to upload AI score to IPFS")?;

        // Generate commit-reveal hashes
        let score_json = serde_json::to_vec(&score_data)?;
        let reveal_hash = crate::chain::hash_bytes(&score_json);

        // Generate random nonce
        let nonce_bytes: [u8; 32] = rand::random();
        let nonce = FixedBytes::<32>::from(nonce_bytes);

        let commit_hash = generate_commit_hash(reveal_hash, nonce);

        // Commit on blockchain
        let batch_hash = hash_string(batch_id);
        let commit_receipt = self
            .state
            .chain_client
            .commit_ai_score(batch_hash, commit_hash)
            .await
            .context("Blockchain AI commit failed")?;

        // Wait a bit before reveal (simulating time-lock)
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Reveal on blockchain
        let reveal_receipt = self
            .state
            .chain_client
            .reveal_ai_score(batch_hash, reveal_hash, nonce, cid.clone())
            .await
            .context("Blockchain AI reveal failed")?;

        Ok((
            format!("{:?}", commit_receipt.transaction_hash),
            format!("{:?}", reveal_receipt.transaction_hash),
            cid,
        ))
    }

    // ========================================================================
    //                      VERIFICATION WORKFLOWS
    // ========================================================================

    /// Verify complete supply chain traceability for a SKU
    pub async fn verify_sku_traceability(&self, sku_id: &str) -> Result<SkuTraceability> {
        tracing::info!("🔍 Verifying SKU traceability: {}", sku_id);

        let sku_hash = hash_string(sku_id);

        // Get SKU origin from blockchain
        let (parent_batch_hash, merkle_root, packaged_at) = self
            .state
            .chain_client
            .verify_package_origin(sku_hash)
            .await
            .context("Failed to verify SKU origin")?;

        // Build traceability chain
        let trace = SkuTraceability {
            sku_id: sku_id.to_string(),
            parent_batch_hash: format!("{:?}", parent_batch_hash),
            merkle_root: format!("{:?}", merkle_root),
            packaged_at,
            verified: packaged_at > 0,
            trace_summary: format!(
                "SKU {} → Batch {:?} → Packaged at timestamp {}",
                sku_id, parent_batch_hash, packaged_at
            ),
        };

        Ok(trace)
    }

    /// Verify farmer registration
    pub async fn verify_farmer(&self, farmer_did: &str) -> Result<FarmerVerification> {
        let farmer_did_hash: FixedBytes<32> = farmer_did.parse()?;

        let (exists, crop_id_hash, registered_at) = self
            .state
            .chain_client
            .verify_farmer(farmer_did_hash)
            .await?;

        Ok(FarmerVerification {
            farmer_did: farmer_did.to_string(),
            exists,
            crop_id_hash: format!("{:?}", crop_id_hash),
            registered_at,
        })
    }
}

// ============================================================================
//                         VERIFICATION RESULTS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkuTraceability {
    pub sku_id: String,
    pub parent_batch_hash: String,
    pub merkle_root: String,
    pub packaged_at: u64,
    pub verified: bool,
    pub trace_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmerVerification {
    pub farmer_did: String,
    pub exists: bool,
    pub crop_id_hash: String,
    pub registered_at: u64,
}

// ============================================================================
//                         HTTP HANDLERS
// ============================================================================

pub mod http_handlers {
    use super::*;
    use crate::state::AppState;
    use axum::{extract::State, http::StatusCode, Json};

    #[derive(Debug, Serialize)]
    pub struct ErrorResponse {
        pub error: String,
    }

    /// Execute complete workflow endpoint
    pub async fn execute_workflow(
        State(state): State<AppState>,
        Json(payload): Json<CompleteWorkflowData>,
    ) -> Result<Json<WorkflowResult>, (StatusCode, Json<ErrorResponse>)> {
        tracing::info!("🚀 Received complete workflow execution request");

        let workflow = SupplyChainWorkflow::new(state);

        let result = workflow.execute_full_workflow(payload).await.map_err(|e| {
            tracing::error!(error = %e, "Workflow execution failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Workflow failed: {}", e),
                }),
            )
        })?;

        tracing::info!(
            "✅ Workflow completed: {} stages, {} transactions",
            result.summary.successful_stages,
            result.summary.total_transactions
        );

        Ok(Json(result))
    }

    /// Verify SKU traceability endpoint
    #[derive(Debug, Deserialize)]
    pub struct VerifySkuRequest {
        pub sku_id: String,
    }

    pub async fn verify_sku_handler(
        State(state): State<AppState>,
        Json(payload): Json<VerifySkuRequest>,
    ) -> Result<Json<SkuTraceability>, (StatusCode, Json<ErrorResponse>)> {
        tracing::info!("🔍 Verifying SKU: {}", payload.sku_id);

        let workflow = SupplyChainWorkflow::new(state);

        let result = workflow
            .verify_sku_traceability(&payload.sku_id)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "SKU verification failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Verification failed: {}", e),
                    }),
                )
            })?;

        Ok(Json(result))
    }

    /// Verify farmer endpoint
    #[derive(Debug, Deserialize)]
    pub struct VerifyFarmerRequest {
        pub farmer_did: String,
    }

    pub async fn verify_farmer_handler(
        State(state): State<AppState>,
        Json(payload): Json<VerifyFarmerRequest>,
    ) -> Result<Json<FarmerVerification>, (StatusCode, Json<ErrorResponse>)> {
        tracing::info!("🔍 Verifying farmer: {}", payload.farmer_did);

        let workflow = SupplyChainWorkflow::new(state);

        let result = workflow
            .verify_farmer(&payload.farmer_did)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Farmer verification failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Verification failed: {}", e),
                    }),
                )
            })?;

        Ok(Json(result))
    }
}
