use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use anyhow::{Context, Result};
use axum::{extract::State, Json};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

// ======================== CONFIGURATION ========================

#[derive(Debug, Clone)]
pub struct IpfsConfig {
    pub primary_api_url: String,
    pub backup_api_url: Option<String>,
    pub gateway_url: String,
    pub project_id: Option<String>,
    pub project_secret: Option<String>,
    pub pinata_api_key: Option<String>,
    pub pinata_secret_key: Option<String>,
    pub timeout_seconds: u64,
    pub max_file_size_mb: usize,
    pub retry_attempts: usize,
}

impl IpfsConfig {
    pub fn from_env() -> Result<Self> {
        let primary_api_url =
            env::var("IPFS_API_URL").context("IPFS_API_URL environment variable is required")?;

        let backup_api_url = env::var("IPFS_BACKUP_API_URL").ok();

        let gateway_url =
            env::var("IPFS_GATEWAY_URL").unwrap_or_else(|_| "https://ipfs.io/ipfs/".to_string());

        let project_id = env::var("IPFS_PROJECT_ID").ok();
        let project_secret = env::var("IPFS_PROJECT_SECRET").ok();

        let pinata_api_key = env::var("PINATA_API_KEY").ok();
        let pinata_secret_key = env::var("PINATA_SECRET_KEY").ok();

        let timeout_seconds = env::var("IPFS_TIMEOUT_SECONDS")
            .unwrap_or_else(|_| "30".to_string())
            .parse()
            .unwrap_or(30);

        let max_file_size_mb = env::var("IPFS_MAX_FILE_SIZE_MB")
            .unwrap_or_else(|_| "100".to_string())
            .parse()
            .unwrap_or(100);

        let retry_attempts = env::var("IPFS_RETRY_ATTEMPTS")
            .unwrap_or_else(|_| "3".to_string())
            .parse()
            .unwrap_or(3);

        if project_id.is_some() != project_secret.is_some() {
            warn!("Both IPFS_PROJECT_ID and IPFS_PROJECT_SECRET should be set for Infura authentication");
        }

        if pinata_api_key.is_some() != pinata_secret_key.is_some() {
            warn!("Both PINATA_API_KEY and PINATA_SECRET_KEY should be set for Pinata backup");
        }

        Ok(Self {
            primary_api_url,
            backup_api_url,
            gateway_url,
            project_id,
            project_secret,
            pinata_api_key,
            pinata_secret_key,
            timeout_seconds,
            max_file_size_mb,
            retry_attempts,
        })
    }
}

// ======================== RESPONSE TYPES ========================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IpfsAddResponse {
    hash: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    size: String,
}

#[derive(Debug, Deserialize)]
struct PinataResponse {
    #[serde(rename = "IpfsHash")]
    ipfs_hash: String,
    #[allow(dead_code)]
    #[serde(rename = "PinSize")]
    pin_size: u64,
    #[allow(dead_code)]
    #[serde(rename = "Timestamp")]
    timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IpfsMetadata {
    pub version: String,
    pub timestamp: String,
    pub stage: String,
    pub content_hash: String,
    pub size_bytes: usize,
    pub upload_source: String,
}

// ======================== SUPPLY CHAIN METADATA TYPES ========================

#[derive(Debug, Serialize, Deserialize)]
pub struct FarmerMetadata {
    pub version: String,
    pub timestamp: String,
    pub stage: String,
    pub farmer_profile: FarmerProfile,
    pub verification: VerificationData,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FarmerProfile {
    pub name: String,
    pub contact_info: ContactInfo,
    pub land_details: LandDetails,
    pub crop_info: CropInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactInfo {
    pub phone: String,
    pub address: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LandDetails {
    pub total_area: String,
    pub soil_type: String,
    pub coordinates: [f64; 2],
    pub documents: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CropInfo {
    pub variety: String,
    pub planting_date: String,
    pub expected_yield: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationData {
    pub document_hashes: Vec<String>,
    pub satellite_imagery: Option<String>,
    pub certifications: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FPOPurchaseMetadata {
    pub version: String,
    pub timestamp: String,
    pub stage: String,
    pub transaction: TransactionDetails,
    pub documents: PurchaseDocuments,
    pub fpo_info: FPOInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TransactionDetails {
    pub batch_id: String,
    pub quantity: String,
    pub price_per_kg: String,
    pub total_amount: String,
    pub quality_grade: String,
    pub moisture_content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PurchaseDocuments {
    pub receipt: String,
    pub quality_report: String,
    pub weighment_slip: String,
    pub photos: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FPOInfo {
    pub name: String,
    pub license: String,
    pub address: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WarehouseMetadata {
    pub version: String,
    pub timestamp: String,
    pub stage: String,
    pub warehouse_info: WarehouseInfo,
    pub storage_details: StorageDetails,
    pub iot_data: IoTData,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WarehouseInfo {
    pub id: String,
    pub location: [f64; 2],
    pub capacity: String,
    pub certifications: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StorageDetails {
    pub batch_id: String,
    pub storage_unit: String,
    pub entry_date: String,
    pub fumigation: FumigationData,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FumigationData {
    pub done: bool,
    pub date: String,
    pub certificate: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IoTData {
    pub rollup_period: String,
    pub averages: IoTAverages,
    pub alerts: Vec<String>,
    pub raw_data_cid: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IoTAverages {
    pub temperature: String,
    pub humidity: String,
    pub air_quality: String,
}

// ======================== IPFS CLIENT ========================

#[derive(Debug, Clone)]
pub struct IpfsClient {
    http_client: reqwest::Client,
    config: IpfsConfig,
}

impl IpfsClient {
    pub fn new(config: IpfsConfig) -> Self {
        let timeout_duration = Duration::from_secs(config.timeout_seconds);

        let http_client = reqwest::Client::builder()
            .timeout(timeout_duration)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http_client,
            config,
        }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(IpfsConfig::from_env()?))
    }

    // Upload JSON data with metadata wrapper
    pub async fn upload_json<T: Serialize>(&self, value: &T) -> Result<String> {
        let json_bytes = serde_json::to_vec(value).context("Failed to serialize value to JSON")?;

        // Check file size
        if json_bytes.len() > self.config.max_file_size_mb * 1024 * 1024 {
            anyhow::bail!(
                "File size exceeds maximum limit of {} MB",
                self.config.max_file_size_mb
            );
        }

        self.upload_bytes_with_retry(json_bytes).await
    }

    // Upload raw bytes with retry logic
    async fn upload_bytes_with_retry(&self, bytes: Vec<u8>) -> Result<String> {
        let mut last_error = None;

        for attempt in 1..=self.config.retry_attempts {
            match self.upload_bytes_single(&bytes).await {
                Ok(cid) => {
                    info!(
                        cid = %cid,
                        size = bytes.len(),
                        attempt = attempt,
                        "Successfully uploaded to IPFS"
                    );

                    // Try to pin to backup service if available
                    if let Err(e) = self.pin_to_backup(&cid, &bytes).await {
                        warn!(cid = %cid, error = %e, "Failed to pin to backup service");
                    }

                    return Ok(cid);
                }
                Err(e) => {
                    warn!(
                        attempt = attempt,
                        max_attempts = self.config.retry_attempts,
                        error = %e,
                        "IPFS upload attempt failed"
                    );
                    last_error = Some(e);

                    if attempt < self.config.retry_attempts {
                        tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt as u32))).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("All upload attempts failed")))
    }

    // Single upload attempt to primary IPFS service
    async fn upload_bytes_single(&self, bytes: &[u8]) -> Result<String> {
        let url = format!("{}/api/v0/add", self.config.primary_api_url);

        debug!(
            url = %url,
            size = bytes.len(),
            "Uploading to primary IPFS service"
        );

        let part = Part::bytes(bytes.to_vec())
            .file_name("file")
            .mime_str("application/octet-stream")
            .context("Failed to set MIME type")?;

        let form = Form::new().part("file", part);
        let mut request = self.http_client.post(&url).multipart(form);

        // Add authentication if available
        if let (Some(ref id), Some(ref secret)) =
            (&self.config.project_id, &self.config.project_secret)
        {
            request = request.basic_auth(id, Some(secret));
        }

        let response = timeout(
            Duration::from_secs(self.config.timeout_seconds),
            request.send(),
        )
        .await
        .context("Request timed out")?
        .context("Failed to send request to IPFS API")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("IPFS API error ({}): {}", status, body);
        }

        let ipfs_response: IpfsAddResponse = response
            .json()
            .await
            .context("Failed to parse IPFS API response")?;

        Ok(ipfs_response.hash)
    }

    // Pin content to backup service (Pinata)
    async fn pin_to_backup(&self, cid: &str, _bytes: &[u8]) -> Result<()> {
        if let (Some(ref api_key), Some(ref secret_key)) =
            (&self.config.pinata_api_key, &self.config.pinata_secret_key)
        {
            debug!(cid = %cid, "Pinning to Pinata backup service");

            let url = "https://api.pinata.cloud/pinning/pinByHash";

            let pin_request = serde_json::json!({
                "hashToPin": cid,
                "pinataMetadata": {
                    "name": format!("oilseed-{}", cid),
                    "keyvalues": {
                        "service": "oilseed-value-chain",
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    }
                }
            });

            let response = self
                .http_client
                .post(url)
                .header("pinata_api_key", api_key)
                .header("pinata_secret_api_key", secret_key)
                .json(&pin_request)
                .send()
                .await?;

            if response.status().is_success() {
                info!(cid = %cid, "Successfully pinned to Pinata");
            } else {
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Pinata pinning failed: {}", error_text);
            }
        }

        Ok(())
    }

    // Retrieve content from IPFS
    pub async fn get_content(&self, cid: &str) -> Result<Vec<u8>> {
        let url = format!("{}/api/v0/cat?arg={}", self.config.primary_api_url, cid);

        debug!(cid = %cid, url = %url, "Retrieving content from IPFS");

        let mut request = self.http_client.post(&url);

        // Add authentication if available
        if let (Some(ref id), Some(ref secret)) =
            (&self.config.project_id, &self.config.project_secret)
        {
            request = request.basic_auth(id, Some(secret));
        }

        let response = timeout(
            Duration::from_secs(self.config.timeout_seconds),
            request.send(),
        )
        .await
        .context("Request timed out")?
        .context("Failed to send request to IPFS API")?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to retrieve content: {}", response.status());
        }

        let bytes = response
            .bytes()
            .await
            .context("Failed to read response body")?;
        Ok(bytes.to_vec())
    }

    // Get JSON content and deserialize
    pub async fn get_json<T: for<'de> Deserialize<'de>>(&self, cid: &str) -> Result<T> {
        let bytes = self.get_content(cid).await?;
        let value = serde_json::from_slice(&bytes).context("Failed to deserialize JSON content")?;
        Ok(value)
    }

    // Generate gateway URL
    pub fn gateway_url(&self, cid: &str) -> String {
        format!("{}{}", self.config.gateway_url, cid)
    }

    // Health check
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/v0/version", self.config.primary_api_url);

        let mut request = self.http_client.post(&url);

        if let (Some(ref id), Some(ref secret)) =
            (&self.config.project_id, &self.config.project_secret)
        {
            request = request.basic_auth(id, Some(secret));
        }

        match timeout(Duration::from_secs(5), request.send()).await {
            Ok(Ok(response)) => Ok(response.status().is_success()),
            _ => Ok(false),
        }
    }

    // Create standardized metadata wrapper
    pub fn create_metadata<T: Serialize>(&self, stage: &str, content: &T) -> Result<IpfsMetadata> {
        let content_json =
            serde_json::to_string(content).context("Failed to serialize content for metadata")?;

        let content_hash = format!("{:x}", md5::compute(content_json.as_bytes()));

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("Failed to get timestamp")?
            .as_secs();

        Ok(IpfsMetadata {
            version: "1.0".to_string(),
            timestamp: chrono::DateTime::from_timestamp(timestamp as i64, 0)
                .unwrap_or_default()
                .to_rfc3339(),
            stage: stage.to_string(),
            content_hash,
            size_bytes: content_json.len(),
            upload_source: "rust-backend".to_string(),
        })
    }
}

// ======================== HTTP HANDLERS ========================

#[derive(Debug, Deserialize)]
pub struct UploadRequest {
    pub data: serde_json::Value,
    pub stage: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub cid: String,
    pub gateway_url: String,
    pub size_bytes: usize,
    pub timestamp: String,
}

#[derive(Debug, Deserialize)]
pub struct RetrieveRequest {
    pub cid: String,
}

#[derive(Debug, Serialize)]
pub struct RetrieveResponse {
    pub data: serde_json::Value,
    pub metadata: IpfsMetadata,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub ipfs_connected: bool,
    pub timestamp: String,
}

// Upload generic data to IPFS
pub async fn upload_to_ipfs(
    State(state): State<AppState>,
    Json(payload): Json<UploadRequest>,
) -> ApiResult<UploadResponse> {
    let stage = payload.stage.as_deref().unwrap_or("generic");

    // Create metadata wrapper
    let metadata = state
        .ipfs_client
        .create_metadata(stage, &payload.data)
        .map_err(ApiError::ipfs_upload_failed)?;

    // Create the final document with metadata
    let document = serde_json::json!({
        "metadata": metadata,
        "content": payload.data
    });

    let document_json = serde_json::to_string(&document).map_err(ApiError::ipfs_upload_failed)?;

    let size_bytes = document_json.len();

    let cid = state
        .ipfs_client
        .upload_bytes_with_retry(document_json.into_bytes())
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    info!(cid = %cid, stage = %stage, "Successfully uploaded to IPFS");

    Ok(Json(UploadResponse {
        gateway_url: state.ipfs_client.gateway_url(&cid),
        cid,
        size_bytes,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }))
}

// Retrieve data from IPFS
pub async fn retrieve_from_ipfs(
    State(state): State<AppState>,
    Json(payload): Json<RetrieveRequest>,
) -> ApiResult<RetrieveResponse> {
    let document: serde_json::Value = state
        .ipfs_client
        .get_json(&payload.cid)
        .await
        .map_err(ApiError::ipfs_retrieval_failed)?;

    // Extract metadata and content
    let metadata = document
        .get("metadata")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .ok_or_else(|| {
            ApiError::ipfs_retrieval_failed(anyhow::anyhow!("Invalid document format"))
        })?;

    let content = document
        .get("content")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    Ok(Json(RetrieveResponse {
        data: content,
        metadata,
    }))
}

// Health check endpoint
pub async fn ipfs_health(State(state): State<AppState>) -> ApiResult<HealthResponse> {
    let ipfs_connected = state.ipfs_client.health_check().await.unwrap_or(false);

    Ok(Json(HealthResponse {
        status: if ipfs_connected {
            "healthy".to_string()
        } else {
            "unhealthy".to_string()
        },
        ipfs_connected,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }))
}

// ======================== SUPPLY CHAIN SPECIFIC HANDLERS ========================

// Upload farmer registration data
pub async fn upload_farmer_metadata(
    State(state): State<AppState>,
    Json(metadata): Json<FarmerMetadata>,
) -> ApiResult<UploadResponse> {
    let cid = state
        .ipfs_client
        .upload_json(&metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    info!(cid = %cid, farmer = %metadata.farmer_profile.name, "Farmer metadata uploaded");

    Ok(Json(UploadResponse {
        gateway_url: state.ipfs_client.gateway_url(&cid),
        cid,
        size_bytes: serde_json::to_string(&metadata).unwrap_or_default().len(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }))
}

// Upload FPO purchase data
pub async fn upload_fpo_purchase_metadata(
    State(state): State<AppState>,
    Json(metadata): Json<FPOPurchaseMetadata>,
) -> ApiResult<UploadResponse> {
    let cid = state
        .ipfs_client
        .upload_json(&metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    info!(cid = %cid, batch_id = %metadata.transaction.batch_id, "FPO purchase metadata uploaded");

    Ok(Json(UploadResponse {
        gateway_url: state.ipfs_client.gateway_url(&cid),
        cid,
        size_bytes: serde_json::to_string(&metadata).unwrap_or_default().len(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }))
}

// Upload warehouse data
pub async fn upload_warehouse_metadata(
    State(state): State<AppState>,
    Json(metadata): Json<WarehouseMetadata>,
) -> ApiResult<UploadResponse> {
    let cid = state
        .ipfs_client
        .upload_json(&metadata)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    info!(cid = %cid, warehouse_id = %metadata.warehouse_info.id, "Warehouse metadata uploaded");

    Ok(Json(UploadResponse {
        gateway_url: state.ipfs_client.gateway_url(&cid),
        cid,
        size_bytes: serde_json::to_string(&metadata).unwrap_or_default().len(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }))
}

// Helper function to create gateway URL (for backward compatibility)
pub fn ipfs_gateway_url(cid: &str) -> String {
    env::var("IPFS_GATEWAY_URL").unwrap_or_else(|_| "https://ipfs.io/ipfs/".to_string()) + cid
}
