//! Infura IPFS Provider Implementation
//!
//! Provides integration with Infura's IPFS API service.
//! https://docs.infura.io/networks/ipfs

use super::{
    Cid, CircuitBreaker, CircuitBreakerConfig, IpfsError, IpfsProvider, IpfsResult,
    ProviderAuth, ProviderConfig, ProviderStats, RetryConfig, UploadResult,
};
use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Infura IPFS API response for add operation
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InfuraAddResponse {
    hash: String,
    #[allow(dead_code)]
    name: String,
    size: String,
}

/// Infura IPFS API response for pin/ls operation
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InfuraPinLsResponse {
    keys: std::collections::HashMap<String, PinInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PinInfo {
    #[allow(dead_code)]
    r#type: String,
}

/// Infura IPFS Provider
pub struct InfuraProvider {
    client: Client,
    config: InfuraConfig,
    stats: Arc<ProviderStats>,
    circuit_breaker: Arc<CircuitBreaker>,
    retry_config: RetryConfig,
}

/// Configuration specific to Infura
#[derive(Debug, Clone)]
pub struct InfuraConfig {
    pub api_url: String,
    pub gateway_url: String,
    pub project_id: String,
    pub project_secret: String,
    pub timeout: Duration,
    pub dedicated_gateway: Option<String>,
}

impl InfuraConfig {
    /// Create from environment variables
    pub fn from_env() -> Result<Self, std::env::VarError> {
        Ok(Self {
            api_url: std::env::var("INFURA_IPFS_API_URL")
                .unwrap_or_else(|_| "https://ipfs.infura.io:5001".to_string()),
            gateway_url: std::env::var("INFURA_IPFS_GATEWAY_URL")
                .unwrap_or_else(|_| "https://ipfs.io/ipfs/".to_string()),
            project_id: std::env::var("INFURA_IPFS_PROJECT_ID")?,
            project_secret: std::env::var("INFURA_IPFS_PROJECT_SECRET")?,
            timeout: Duration::from_secs(
                std::env::var("INFURA_IPFS_TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30),
            ),
            dedicated_gateway: std::env::var("INFURA_IPFS_DEDICATED_GATEWAY").ok(),
        })
    }

    /// Create from generic provider config
    pub fn from_provider_config(config: &ProviderConfig) -> Result<Self, IpfsError> {
        let (project_id, project_secret) = match &config.auth {
            Some(ProviderAuth::Basic { username, password }) => {
                (username.clone(), password.clone())
            }
            Some(ProviderAuth::ApiKey { key, secret }) => {
                (key.clone(), secret.clone().unwrap_or_default())
            }
            _ => {
                return Err(IpfsError::AuthenticationFailed(
                    "Infura requires project ID and secret".into(),
                ))
            }
        };

        Ok(Self {
            api_url: config.api_url.clone(),
            gateway_url: config.gateway_url.clone(),
            project_id,
            project_secret,
            timeout: Duration::from_secs(config.timeout_seconds),
            dedicated_gateway: None,
        })
    }
}

impl InfuraProvider {
    /// Create a new Infura provider
    pub fn new(config: InfuraConfig) -> Result<Self, IpfsError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| IpfsError::Internal(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            config,
            stats: Arc::new(ProviderStats::new()),
            circuit_breaker: Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default())),
            retry_config: RetryConfig::default(),
        })
    }

    /// Create from environment variables
    pub fn from_env() -> Result<Self, IpfsError> {
        let config = InfuraConfig::from_env()
            .map_err(|e| IpfsError::AuthenticationFailed(format!("Missing env var: {}", e)))?;
        Self::new(config)
    }

    /// Create with custom retry and circuit breaker configs
    pub fn with_configs(
        config: InfuraConfig,
        retry_config: RetryConfig,
        circuit_breaker_config: CircuitBreakerConfig,
    ) -> Result<Self, IpfsError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| IpfsError::Internal(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            client,
            config,
            stats: Arc::new(ProviderStats::new()),
            circuit_breaker: Arc::new(CircuitBreaker::new(circuit_breaker_config)),
            retry_config,
        })
    }

    /// Get basic auth header value
    fn basic_auth(&self) -> String {
        use base64::Engine;
        let credentials = format!("{}:{}", self.config.project_id, self.config.project_secret);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        format!("Basic {}", encoded)
    }

    /// Make an authenticated request to Infura API
    async fn api_request(
        &self,
        method: reqwest::Method,
        endpoint: &str,
    ) -> Result<reqwest::RequestBuilder, IpfsError> {
        let url = format!("{}{}", self.config.api_url, endpoint);
        Ok(self
            .client
            .request(method, &url)
            .header("Authorization", self.basic_auth()))
    }

    /// Check if circuit breaker allows request
    async fn check_circuit_breaker(&self) -> IpfsResult<()> {
        if !self.circuit_breaker.should_allow().await {
            return Err(IpfsError::CircuitBreakerOpen(self.name().to_string()));
        }
        Ok(())
    }

    /// Record success and update stats
    async fn record_success(&self, bytes: usize, latency_ms: u64, is_upload: bool) {
        self.circuit_breaker.record_success().await;
        if is_upload {
            self.stats.record_upload(bytes, latency_ms);
        } else {
            self.stats.record_download(bytes, latency_ms);
        }
    }

    /// Record failure and update stats
    async fn record_failure(&self, error: &IpfsError) {
        if error.should_trip_circuit() {
            self.circuit_breaker.record_failure().await;
        }
        self.stats.record_failure();
    }
}

#[async_trait]
impl IpfsProvider for InfuraProvider {
    fn name(&self) -> &str {
        "infura"
    }

    fn gateway_url(&self, cid: &Cid) -> String {
        if let Some(ref dedicated) = self.config.dedicated_gateway {
            format!("{}/ipfs/{}", dedicated.trim_end_matches('/'), cid)
        } else {
            format!("{}{}", self.config.gateway_url, cid)
        }
    }

    async fn upload(&self, data: &[u8]) -> IpfsResult<UploadResult> {
        self.check_circuit_breaker().await?;

        let start = Instant::now();
        let data_len = data.len();

        debug!(
            provider = self.name(),
            size_bytes = data_len,
            "Uploading to Infura IPFS"
        );

        // Build multipart form
        let part = Part::bytes(data.to_vec())
            .file_name("file")
            .mime_str("application/octet-stream")
            .map_err(|e| IpfsError::Internal(format!("Failed to create form part: {}", e)))?;

        let form = Form::new().part("file", part);

        // Make request
        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/add")
            .await?
            .query(&[("pin", "true")])
            .multipart(form)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    IpfsError::Timeout(self.config.timeout)
                } else if e.is_connect() {
                    IpfsError::ProviderUnavailable(format!("Connection failed: {}", e))
                } else {
                    IpfsError::NetworkError(e.to_string())
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let error = if status.as_u16() == 401 {
                IpfsError::AuthenticationFailed("Invalid Infura credentials".into())
            } else if status.as_u16() == 429 {
                IpfsError::RateLimited("Infura rate limit exceeded".into())
            } else {
                IpfsError::UploadFailed(format!("HTTP {}: {}", status, error_body))
            };
            self.record_failure(&error).await;
            return Err(error);
        }

        let infura_response: InfuraAddResponse = response.json().await.map_err(|e| {
            let error = IpfsError::Internal(format!("Failed to parse response: {}", e));
            error
        })?;

        let elapsed = start.elapsed();
        let latency_ms = elapsed.as_millis() as u64;

        self.record_success(data_len, latency_ms, true).await;

        let cid = Cid::new(&infura_response.hash);

        info!(
            provider = self.name(),
            cid = %cid,
            size_bytes = data_len,
            latency_ms = latency_ms,
            "Upload successful"
        );

        Ok(UploadResult {
            cid,
            size_bytes: infura_response.size.parse().unwrap_or(data_len),
            provider: self.name().to_string(),
            upload_time_ms: latency_ms,
            pinned: true,
        })
    }

    async fn download(&self, cid: &Cid) -> IpfsResult<Vec<u8>> {
        self.check_circuit_breaker().await?;

        let start = Instant::now();

        debug!(
            provider = self.name(),
            cid = %cid,
            "Downloading from Infura IPFS"
        );

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/cat")
            .await?
            .query(&[("arg", cid.as_str())])
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    IpfsError::Timeout(self.config.timeout)
                } else {
                    IpfsError::NetworkError(e.to_string())
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let error = if status.as_u16() == 404 || error_body.contains("not found") {
                IpfsError::NotFound(cid.clone())
            } else {
                IpfsError::DownloadFailed(format!("HTTP {}: {}", status, error_body))
            };
            self.record_failure(&error).await;
            return Err(error);
        }

        let bytes = response.bytes().await.map_err(|e| {
            IpfsError::DownloadFailed(format!("Failed to read response body: {}", e))
        })?;

        let elapsed = start.elapsed();
        let latency_ms = elapsed.as_millis() as u64;
        let data_len = bytes.len();

        self.record_success(data_len, latency_ms, false).await;

        debug!(
            provider = self.name(),
            cid = %cid,
            size_bytes = data_len,
            latency_ms = latency_ms,
            "Download successful"
        );

        Ok(bytes.to_vec())
    }

    async fn pin(&self, cid: &Cid) -> IpfsResult<()> {
        self.check_circuit_breaker().await?;

        debug!(
            provider = self.name(),
            cid = %cid,
            "Pinning on Infura IPFS"
        );

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/pin/add")
            .await?
            .query(&[("arg", cid.as_str())])
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let error = IpfsError::PinFailed(format!(
                "HTTP {}: {}",
                response.status(),
                error_body
            ));
            self.record_failure(&error).await;
            return Err(error);
        }

        self.circuit_breaker.record_success().await;
        self.stats.record_pin();

        info!(
            provider = self.name(),
            cid = %cid,
            "Pin successful"
        );

        Ok(())
    }

    async fn unpin(&self, cid: &Cid) -> IpfsResult<()> {
        self.check_circuit_breaker().await?;

        debug!(
            provider = self.name(),
            cid = %cid,
            "Unpinning from Infura IPFS"
        );

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/pin/rm")
            .await?
            .query(&[("arg", cid.as_str())])
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let error = IpfsError::UnpinFailed(format!(
                "HTTP {}: {}",
                status,
                error_body
            ));
            self.record_failure(&error).await;
            return Err(error);
        }

        self.circuit_breaker.record_success().await;

        info!(
            provider = self.name(),
            cid = %cid,
            "Unpin successful"
        );

        Ok(())
    }

    async fn is_pinned(&self, cid: &Cid) -> IpfsResult<bool> {
        self.check_circuit_breaker().await?;

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/pin/ls")
            .await?
            .query(&[("arg", cid.as_str()), ("type", "all")])
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        if response.status().as_u16() == 500 {
            // Infura returns 500 when CID is not pinned
            return Ok(false);
        }

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            if error_body.contains("not pinned") {
                return Ok(false);
            }
            return Err(IpfsError::Internal(format!(
                "Failed to check pin status: {}",
                error_body
            )));
        }

        let pin_response: InfuraPinLsResponse = response
            .json()
            .await
            .map_err(|e| IpfsError::Internal(format!("Failed to parse response: {}", e)))?;

        Ok(pin_response.keys.contains_key(cid.as_str()))
    }

    async fn health_check(&self) -> IpfsResult<bool> {
        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/version")
            .await?
            .send()
            .await;

        match response {
            Ok(resp) => {
                let is_healthy = resp.status().is_success();
                if is_healthy {
                    self.circuit_breaker.record_success().await;
                }
                Ok(is_healthy)
            }
            Err(e) => {
                warn!(
                    provider = self.name(),
                    error = %e,
                    "Health check failed"
                );
                Ok(false)
            }
        }
    }

    fn stats(&self) -> ProviderStats {
        ProviderStats {
            uploads: std::sync::atomic::AtomicU64::new(
                self.stats.uploads.load(std::sync::atomic::Ordering::Relaxed),
            ),
            downloads: std::sync::atomic::AtomicU64::new(
                self.stats.downloads.load(std::sync::atomic::Ordering::Relaxed),
            ),
            pins: std::sync::atomic::AtomicU64::new(
                self.stats.pins.load(std::sync::atomic::Ordering::Relaxed),
            ),
            failures: std::sync::atomic::AtomicU64::new(
                self.stats.failures.load(std::sync::atomic::Ordering::Relaxed),
            ),
            bytes_uploaded: std::sync::atomic::AtomicU64::new(
                self.stats.bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed),
            ),
            bytes_downloaded: std::sync::atomic::AtomicU64::new(
                self.stats.bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed),
            ),
            total_latency_ms: std::sync::atomic::AtomicU64::new(
                self.stats.total_latency_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> InfuraConfig {
        InfuraConfig {
            api_url: "https://ipfs.infura.io:5001".to_string(),
            gateway_url: "https://ipfs.io/ipfs/".to_string(),
            project_id: "test_project_id".to_string(),
            project_secret: "test_project_secret".to_string(),
            timeout: Duration::from_secs(30),
            dedicated_gateway: None,
        }
    }

    #[test]
    fn test_gateway_url() {
        let provider = InfuraProvider::new(test_config()).unwrap();
        let cid = Cid::new("QmTest123");

        assert_eq!(
            provider.gateway_url(&cid),
            "https://ipfs.io/ipfs/QmTest123"
        );
    }

    #[test]
    fn test_gateway_url_with_dedicated() {
        let mut config = test_config();
        config.dedicated_gateway = Some("https://myproject.infura-ipfs.io".to_string());

        let provider = InfuraProvider::new(config).unwrap();
        let cid = Cid::new("QmTest123");

        assert_eq!(
            provider.gateway_url(&cid),
            "https://myproject.infura-ipfs.io/ipfs/QmTest123"
        );
    }

    #[test]
    fn test_basic_auth() {
        let provider = InfuraProvider::new(test_config()).unwrap();
        let auth = provider.basic_auth();

        assert!(auth.starts_with("Basic "));
        // Verify it's valid base64
        let encoded = auth.strip_prefix("Basic ").unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let decoded_str = String::from_utf8(decoded).unwrap();
        assert_eq!(decoded_str, "test_project_id:test_project_secret");
    }

    #[test]
    fn test_provider_name() {
        let provider = InfuraProvider::new(test_config()).unwrap();
        assert_eq!(provider.name(), "infura");
    }

    #[tokio::test]
    async fn test_circuit_breaker_integration() {
        let provider = InfuraProvider::new(test_config()).unwrap();

        // Circuit should be closed initially
        assert!(provider.check_circuit_breaker().await.is_ok());
    }
}
