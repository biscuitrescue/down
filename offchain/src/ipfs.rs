use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tracing;

// ======================================================
//                 CONFIGURATION
// ======================================================

#[derive(Debug, Clone)]
pub struct IpfsConfig {
    pub api_url: String,
    pub project_id: Option<String>,
    pub project_secret: Option<String>,
}

impl IpfsConfig {
    pub fn from_env() -> Result<Self> {
        let api_url =
            env::var("IPFS_API_URL").context("Missing required env: IPFS_API_URL")?;

        let project_id = env::var("IPFS_PROJECT_ID").ok();
        let project_secret = env::var("IPFS_PROJECT_SECRET").ok();

        if project_id.is_some() != project_secret.is_some() {
            tracing::warn!(
                "Both IPFS_PROJECT_ID and IPFS_PROJECT_SECRET should be set for authentication"
            );
        }

        Ok(Self {
            api_url,
            project_id,
            project_secret,
        })
    }
}

// ======================================================
//                 IPFS RESPONSE
// ======================================================

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct IpfsAddResponse {
    hash: String,
    name: String,
    size: String,
}

// ======================================================
//                 IPFS CLIENT
// ======================================================

#[derive(Debug, Clone)]
pub struct IpfsClient {
    http_client: reqwest::Client,
    config: IpfsConfig,
}

impl IpfsClient {
    pub fn new(config: IpfsConfig) -> Self {
        Self {
            http_client: reqwest::Client::new(),
            config,
        }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(IpfsConfig::from_env()?))
    }

    // -------------------------------
    // Upload JSON → IPFS
    // -------------------------------
    pub async fn upload_json<T: Serialize>(&self, value: &T) -> Result<String> {
        tracing::info!("Serializing JSON payload for IPFS upload");

        let json_bytes = serde_json::to_vec(value)
            .context("Failed to serialize JSON to bytes")?;

        tracing::debug!(
            size = json_bytes.len(),
            "JSON serialized; uploading to IPFS"
        );

        self.upload_bytes(json_bytes).await
    }

    // -------------------------------
    // Upload raw bytes → IPFS
    // -------------------------------
    pub async fn upload_bytes(&self, bytes: Vec<u8>) -> Result<String> {
        let url = format!("{}/api/v0/add", self.config.api_url);

        tracing::info!(
            %url,
            size = bytes.len(),
            "Uploading file bytes to IPFS"
        );

        let part = Part::bytes(bytes)
            .file_name("file")
            .mime_str("application/octet-stream")
            .context("Failed to set MIME type for IPFS upload")?;

        let form = Form::new().part("file", part);

        let mut request = self.http_client.post(&url).multipart(form);

        // Add authentication if provided
        if let (Some(id), Some(secret)) =
            (&self.config.project_id, &self.config.project_secret)
        {
            tracing::debug!("Using Basic Auth for IPFS upload");
            request = request.basic_auth(id, Some(secret));
        }

        let response = request
            .send()
            .await
            .context("Failed to send IPFS upload request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!(%status, %body, "IPFS API returned an error");
            anyhow::bail!("IPFS API error ({}): {}", status, body);
        }

        let parsed: IpfsAddResponse = response
            .json()
            .await
            .context("Failed to parse IPFS response JSON")?;

        tracing::info!(cid = %parsed.hash, "File uploaded to IPFS");

        Ok(parsed.hash)
    }

    pub fn api_url(&self) -> &str {
        &self.config.api_url
    }

    pub fn has_auth(&self) -> bool {
        self.config.project_id.is_some() && self.config.project_secret.is_some()
    }
}

// ======================================================
//                 APP STATE
// ======================================================

#[derive(Clone)]
pub struct AppState {
    pub ipfs_client: Arc<IpfsClient>,
}

impl AppState {
    pub fn new() -> Result<Self> {
        Ok(Self {
            ipfs_client: Arc::new(IpfsClient::from_env()?),
        })
    }

    pub fn with_ipfs_client(ipfs_client: IpfsClient) -> Self {
        Self {
            ipfs_client: Arc::new(ipfs_client),
        }
    }
}

// ======================================================
//                 REQUEST / RESPONSE MODELS
// ======================================================

use axum::{extract::State, http::StatusCode, Json};

#[derive(Debug, Deserialize)]
pub struct UploadRequest {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub cid: String,
    pub gateway_url: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ======================================================
//                 ROUTE HANDLER
// ======================================================

pub async fn upload_to_ipfs(
    State(state): State<AppState>,
    Json(payload): Json<UploadRequest>,
) -> Result<Json<UploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!("Received IPFS upload request");

    let cid = state
        .ipfs_client
        .upload_json(&payload.data)
        .await
        .map_err(|e| {
            tracing::error!(%e, "IPFS upload failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to upload to IPFS: {}", e),
                }),
            )
        })?;

    let gateway_url = format!("https://ipfs.io/ipfs/{}", cid);

    tracing::info!(%cid, "Successfully uploaded JSON to IPFS");

    Ok(Json(UploadResponse { cid, gateway_url }))
}

// ======================================================
//                 ROUTER
// ======================================================

use axum::{routing::post, Router};

pub fn ipfs_router(state: AppState) -> Router {
    Router::new()
        .route("/api/ipfs/upload", post(upload_to_ipfs))
        .with_state(state)
}

