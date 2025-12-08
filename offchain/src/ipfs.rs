use crate::error::{ipfs_gateway_url, ApiError, ApiResult};
use crate::state::AppState;
use anyhow::{Context, Result};
use axum::{extract::State, Json};
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
        Self {
            http_client: reqwest::Client::new(),
            config,
        }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(IpfsConfig::from_env()?))
    }

    pub async fn upload_json<T: Serialize>(&self, value: &T) -> Result<String> {
        let json_bytes = serde_json::to_vec(value).context("Failed to serialize value to JSON")?;
        self.upload_bytes(json_bytes).await
    }

    pub async fn upload_bytes(&self, bytes: Vec<u8>) -> Result<String> {
        let url = format!("{}/api/v0/add", self.config.api_url);

        tracing::debug!(url = %url, size = bytes.len(), "Uploading to IPFS");

        let part = Part::bytes(bytes)
            .file_name("file")
            .mime_str("application/octet-stream")
            .context("Failed to set MIME type")?;

        let form = Form::new().part("file", part);
        let mut request = self.http_client.post(&url).multipart(form);

        if let (Some(ref id), Some(ref secret)) =
            (&self.config.project_id, &self.config.project_secret)
        {
            request = request.basic_auth(id, Some(secret));
        }

        let response = request
            .send()
            .await
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

        tracing::info!(cid = %ipfs_response.hash, "Uploaded to IPFS");
        Ok(ipfs_response.hash)
    }
}

// ==================== HTTP HANDLER ====================

#[derive(Debug, Deserialize)]
pub struct UploadRequest {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub cid: String,
    pub gateway_url: String,
}

pub async fn upload_to_ipfs(
    State(state): State<AppState>,
    Json(payload): Json<UploadRequest>,
) -> ApiResult<UploadResponse> {
    let cid = state
        .ipfs_client
        .upload_json(&payload.data)
        .await
        .map_err(ApiError::ipfs_upload_failed)?;

    Ok(Json(UploadResponse {
        gateway_url: ipfs_gateway_url(&cid),
        cid,
    }))
}
