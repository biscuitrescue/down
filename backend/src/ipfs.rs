//! Kubo-compatible IPFS client.
//!
//! Records are written with `block/put?cid-codec=raw&mhtype=sha2-256&pin=true`, so the CID is computed locally
//! beforehand, the node's answer is checked against it, retries are naturally idempotent and every fetched
//! block is verified against its CID. Pinning on our node is not permanent availability: run a pinning service
//! or replicate the node for production (docs/ARCHITECTURE.md).

use crate::cid::Cid;
use crate::config::IpfsConfig;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue},
    Json,
};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Kubo's default block size limit is 1 MiB; larger records must be chunked (not supported here).
pub const MAX_BLOCK_SIZE: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum IpfsError {
    #[error("payload of {0} bytes exceeds the {MAX_BLOCK_SIZE}-byte block limit")]
    TooLarge(usize),
    #[error("IPFS node unreachable or timed out: {0}")]
    Unavailable(String),
    #[error("IPFS API returned {status}: {body}")]
    Api { status: u16, body: String },
    #[error("IPFS node returned CID {got}, expected {expected}")]
    CidMismatch { expected: String, got: String },
    #[error("content for {0} not found on the IPFS node")]
    NotFound(String),
    #[error("content for {cid} does not match its CID")]
    IntegrityFailure { cid: String },
    #[error("unexpected IPFS response: {0}")]
    BadResponse(String),
}

impl From<IpfsError> for ApiError {
    fn from(e: IpfsError) -> Self {
        match e {
            IpfsError::TooLarge(_) => ApiError::BadRequest(e.to_string()),
            IpfsError::NotFound(_) => ApiError::NotFound(e.to_string()),
            _ => ApiError::Ipfs(e.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BlockPutResponse {
    key: String,
}

#[derive(Debug, Clone)]
pub struct IpfsClient {
    http_client: reqwest::Client,
    config: IpfsConfig,
}

impl IpfsClient {
    pub fn new(config: IpfsConfig) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self {
            http_client,
            config,
        })
    }

    pub fn gateway_url(&self, cid: &str) -> String {
        let base = &self.config.gateway_url;
        if base.ends_with('/') {
            format!("{base}{cid}")
        } else {
            format!("{base}/{cid}")
        }
    }

    fn request(&self, path_and_query: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.config.api_url, path_and_query);
        let mut req = self.http_client.post(url);
        if let Some(auth) = &self.config.auth_header {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }
        req
    }

    /// Bounded retries with exponential backoff for transport errors and 5xx responses.
    async fn send_with_retry<F>(&self, build: F) -> Result<reqwest::Response, IpfsError>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let attempts = self.config.max_retries.max(1);
        let mut last_err = String::new();
        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(200 * (1 << attempt.min(5)))).await;
            }
            match build().send().await {
                Ok(resp) if resp.status().is_server_error() => {
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    last_err = format!("{status}: {}", truncate(&body));
                    // Kubo answers 500 for missing blocks under a timeout; do not hammer it.
                    if body.contains("not found") || body.contains("context deadline exceeded") {
                        return Err(IpfsError::Api {
                            status,
                            body: truncate(&body),
                        });
                    }
                }
                Ok(resp) => return Ok(resp),
                Err(e) => last_err = e.to_string(),
            }
            tracing::warn!(attempt = attempt + 1, error = %last_err, "IPFS request failed");
        }
        Err(IpfsError::Unavailable(last_err))
    }

    /// Store `bytes` as a raw block, pin it and return its CID (verified against the local computation).
    pub async fn put_block(&self, bytes: &[u8]) -> Result<Cid, IpfsError> {
        if bytes.len() > MAX_BLOCK_SIZE {
            return Err(IpfsError::TooLarge(bytes.len()));
        }
        let expected = Cid::raw_sha256(bytes);
        let resp = self
            .send_with_retry(|| {
                let part = Part::bytes(bytes.to_vec()).file_name("record");
                self.request("/api/v0/block/put?cid-codec=raw&mhtype=sha2-256&pin=true")
                    .multipart(Form::new().part("data", part))
            })
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(IpfsError::Api {
                status: status.as_u16(),
                body: truncate(&body),
            });
        }
        let parsed: BlockPutResponse = resp
            .json()
            .await
            .map_err(|e| IpfsError::BadResponse(e.to_string()))?;
        let got = Cid::parse(&parsed.key).map_err(|e| IpfsError::BadResponse(e.to_string()))?;
        if got != expected {
            return Err(IpfsError::CidMismatch {
                expected: expected.to_string(),
                got: parsed.key,
            });
        }
        tracing::info!(cid = %expected, size = bytes.len(), "stored block on IPFS");
        Ok(expected)
    }

    /// Fetch a block and verify it against its CID when the CID type allows (raw + sha2-256).
    pub async fn get_block(&self, cid: &Cid) -> Result<Vec<u8>, IpfsError> {
        let cid_str = cid.to_string();
        let resp = self
            .send_with_retry(|| {
                // Bound the node's own network search so a missing block fails fast instead of hanging.
                let secs = self.config.timeout.as_secs().saturating_sub(1).max(1);
                self.request(&format!("/api/v0/block/get?arg={cid_str}&timeout={secs}s"))
            })
            .await
            .map_err(|e| match e {
                IpfsError::Api {
                    status: 500,
                    ref body,
                } if body.contains("not found") || body.contains("deadline") => {
                    IpfsError::NotFound(cid_str.clone())
                }
                other => other,
            })?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(IpfsError::Api {
                status: status.as_u16(),
                body: truncate(&body),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| IpfsError::Unavailable(e.to_string()))?;
        if bytes.len() > MAX_BLOCK_SIZE {
            return Err(IpfsError::TooLarge(bytes.len()));
        }
        if cid.verify_bytes(&bytes) == Some(false) {
            return Err(IpfsError::IntegrityFailure { cid: cid_str });
        }
        Ok(bytes.to_vec())
    }

    /// Node identity/version, used by the health check.
    pub async fn version(&self) -> Result<String, IpfsError> {
        let resp = self
            .send_with_retry(|| self.request("/api/v0/version"))
            .await?;
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| IpfsError::BadResponse(e.to_string()))?;
        Ok(v.get("Version")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string())
    }
}

fn truncate(s: &str) -> String {
    s.chars().take(300).collect()
}

// ==================== HTTP HANDLER ====================

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub cid: String,
    pub size: usize,
    pub sha256: String,
    pub gateway_url: String,
}

/// Upload an attachment (document, image, lab PDF) as a raw block. Authenticated, size-limited.
/// Do not upload personal data here: attachments are public on IPFS.
pub async fn upload_to_ipfs(
    State(state): State<AppState>,
    actor: crate::auth::Actor,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<UploadResponse> {
    actor.require_any_role()?;
    if body.is_empty() {
        return Err(ApiError::bad_request("empty upload"));
    }
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v: &HeaderValue| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    tracing::info!(size = body.len(), content_type, "attachment upload");
    let cid = state.ipfs_client.put_block(&body).await?;
    crate::pinning::enqueue(&state, &cid).await;
    let cid_str = cid.to_string();
    Ok(Json(UploadResponse {
        gateway_url: state.ipfs_client.gateway_url(&cid_str),
        sha256: format!("0x{}", hex::encode(crate::crypto::sha256(&body))),
        size: body.len(),
        cid: cid_str,
    }))
}
