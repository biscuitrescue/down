use anyhow::{Context, Result};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone)]
pub struct IpfsConfig {
    pub api_url: String,
    pub project_id: Option<String>,
    pub project_secret: Option<String>,
}

impl IpfsConfig {
    //  IPFS_API_URL
    pub fn from_env() -> Result<Self> {
        let api_url =
            env::var("IPFS_API_URL").context("IPFS_API_URL environment variable is required")?;

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IpfsAddResponse {
    // CID
    hash: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    size: String,
}

#[derive(Debug, Clone)]
pub struct IpfsClient {
    http_client: reqwest::Client,
    config: IpfsConfig,
}

impl IpfsClient {
    pub fn new(config: IpfsConfig) -> Self {
        let http_client = reqwest::Client::new();
        Self {
            http_client,
            config,
        }
    }

    pub fn from_env() -> Result<Self> {
        let config = IpfsConfig::from_env()?;
        Ok(Self::new(config))
    }

    // Upload JSON to IPFS
    pub async fn upload_json<T: Serialize>(&self, value: &T) -> Result<String> {
        tracing::debug!("Serializing JSON data for IPFS upload");

        let json_bytes = serde_json::to_vec(value).context("Failed to serialize value to JSON")?;

        tracing::debug!(
            size_bytes = json_bytes.len(),
            "JSON serialized, uploading to IPFS"
        );

        self.upload_bytes(json_bytes).await
    }

    // Uploads raw bytes to IPFS
    //
    // Uploads the provided bytes directly to IPFS.
    // Returns the CID (Content Identifier) hash as a string.
    //
    // # Arguments
    // * `bytes` - Raw bytes to upload
    //
    // # Returns
    // * `Result<String>` - The CID hash on success
    //
    // # Example
    // ```ignore
    // let data = b"Hello, IPFS!".to_vec();
    // let cid = ipfs_client.upload_bytes(data).await?;
    // ```
    pub async fn upload_bytes(&self, bytes: Vec<u8>) -> Result<String> {
        let url = format!("{}/api/v0/add", self.config.api_url);

        tracing::info!(
            url = %url,
            size_bytes = bytes.len(),
            "Uploading data to IPFS"
        );

        // Create multipart form with the file data
        let part = Part::bytes(bytes)
            .file_name("file")
            .mime_str("application/octet-stream")
            .context("Failed to set MIME type")?;

        let form = Form::new().part("file", part);

        // Build the request
        let mut request = self.http_client.post(&url).multipart(form);

        // Add Basic Auth if credentials are provided
        if let (Some(ref project_id), Some(ref project_secret)) =
            (&self.config.project_id, &self.config.project_secret)
        {
            tracing::debug!("Using Basic Auth for IPFS request");
            request = request.basic_auth(project_id, Some(project_secret));
        }

        // Send the request
        let response = request
            .send()
            .await
            .context("Failed to send request to IPFS API")?;

        // Check for HTTP errors
        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            tracing::error!(
                status = %status,
                body = %error_body,
                "IPFS API returned error"
            );
            anyhow::bail!("IPFS API error ({}): {}", status, error_body);
        }

        // Parse the response
        let ipfs_response: IpfsAddResponse = response
            .json()
            .await
            .context("Failed to parse IPFS API response")?;

        tracing::info!(
            cid = %ipfs_response.hash,
            "Successfully uploaded to IPFS"
        );

        Ok(ipfs_response.hash)
    }
}

// ==================== IPFS HTTP HANDLERS ====================
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

pub async fn upload_to_ipfs(
    State(state): State<crate::state::AppState>,
    Json(payload): Json<UploadRequest>,
) -> Result<Json<UploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!("Received IPFS upload request");

    // Upload the JSON data to IPFS
    let cid = state
        .ipfs_client
        .upload_json(&payload.data)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to upload to IPFS");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to upload to IPFS: {}", e),
                }),
            )
        })?;

    // Construct the gateway URL (using public IPFS gateway)
    let gateway_url = format!("https://ipfs.io/ipfs/{}", cid);

    tracing::info!(cid = %cid, "Successfully uploaded to IPFS");

    Ok(Json(UploadResponse { cid, gateway_url }))
}
