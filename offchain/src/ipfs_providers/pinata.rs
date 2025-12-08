//! Pinata IPFS Provider Implementation
//!
//! Provides integration with Pinata's IPFS pinning service.
//! https://docs.pinata.cloud/

use super::{
    Cid, CircuitBreaker, CircuitBreakerConfig, IpfsError, IpfsProvider, IpfsResult,
    ProviderAuth, ProviderConfig, ProviderStats, RetryConfig, UploadResult,
};
use async_trait::async_trait;
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Pinata API response for pinning file
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PinataPinResponse {
    ipfs_hash: String,
    pin_size: u64,
    #[allow(dead_code)]
    timestamp: String,
}

/// Pinata API response for pin by hash
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinataPinByHashResponse {
    id: String,
    ipfs_pin_hash: String,
    #[allow(dead_code)]
    status: String,
}

/// Pinata pin list response
#[derive(Debug, Deserialize)]
struct PinataPinListResponse {
    count: u32,
    rows: Vec<PinataPinRow>,
}

#[derive(Debug, Deserialize)]
struct PinataPinRow {
    ipfs_pin_hash: String,
    #[allow(dead_code)]
    size: u64,
    #[allow(dead_code)]
    date_pinned: String,
}

/// Pinata metadata for pins
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinataMetadata {
    pub name: Option<String>,
    #[serde(rename = "keyvalues")]
    pub key_values: Option<HashMap<String, String>>,
}

impl Default for PinataMetadata {
    fn default() -> Self {
        Self {
            name: None,
            key_values: None,
        }
    }
}

impl PinataMetadata {
    pub fn new(name: &str) -> Self {
        Self {
            name: Some(name.to_string()),
            key_values: None,
        }
    }

    pub fn with_key_value(mut self, key: &str, value: &str) -> Self {
        let kv = self.key_values.get_or_insert_with(HashMap::new);
        kv.insert(key.to_string(), value.to_string());
        self
    }
}

/// Pinata pinning options
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PinataOptions {
    cid_version: u8,
}

impl Default for PinataOptions {
    fn default() -> Self {
        Self { cid_version: 1 }
    }
}

/// Pinata IPFS Provider
pub struct PinataProvider {
    client: Client,
    config: PinataConfig,
    stats: Arc<ProviderStats>,
    circuit_breaker: Arc<CircuitBreaker>,
    retry_config: RetryConfig,
}

/// Configuration specific to Pinata
#[derive(Debug, Clone)]
pub struct PinataConfig {
    pub api_url: String,
    pub gateway_url: String,
    pub api_key: String,
    pub secret_key: String,
    pub jwt: Option<String>,
    pub timeout: Duration,
    pub use_v2_api: bool,
}

impl PinataConfig {
    /// Create from environment variables
    pub fn from_env() -> Result<Self, std::env::VarError> {
        Ok(Self {
            api_url: std::env::var("PINATA_API_URL")
                .unwrap_or_else(|_| "https://api.pinata.cloud".to_string()),
            gateway_url: std::env::var("PINATA_GATEWAY_URL")
                .unwrap_or_else(|_| "https://gateway.pinata.cloud/ipfs/".to_string()),
            api_key: std::env::var("PINATA_API_KEY")?,
            secret_key: std::env::var("PINATA_SECRET_KEY")?,
            jwt: std::env::var("PINATA_JWT").ok(),
            timeout: Duration::from_secs(
                std::env::var("PINATA_TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60),
            ),
            use_v2_api: std::env::var("PINATA_USE_V2")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
        })
    }

    /// Create from generic provider config
    pub fn from_provider_config(config: &ProviderConfig) -> Result<Self, IpfsError> {
        let (api_key, secret_key) = match &config.auth {
            Some(ProviderAuth::ApiKey { key, secret }) => {
                (key.clone(), secret.clone().unwrap_or_default())
            }
            Some(ProviderAuth::Bearer { token }) => {
                // If using JWT, we'll store it differently
                return Ok(Self {
                    api_url: config.api_url.clone(),
                    gateway_url: config.gateway_url.clone(),
                    api_key: String::new(),
                    secret_key: String::new(),
                    jwt: Some(token.clone()),
                    timeout: Duration::from_secs(config.timeout_seconds),
                    use_v2_api: false,
                });
            }
            _ => {
                return Err(IpfsError::AuthenticationFailed(
                    "Pinata requires API key/secret or JWT".into(),
                ))
            }
        };

        Ok(Self {
            api_url: config.api_url.clone(),
            gateway_url: config.gateway_url.clone(),
            api_key,
            secret_key,
            jwt: None,
            timeout: Duration::from_secs(config.timeout_seconds),
            use_v2_api: false,
        })
    }
}

impl PinataProvider {
    /// Create a new Pinata provider
    pub fn new(config: PinataConfig) -> Result<Self, IpfsError> {
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
        let config = PinataConfig::from_env()
            .map_err(|e| IpfsError::AuthenticationFailed(format!("Missing env var: {}", e)))?;
        Self::new(config)
    }

    /// Create with custom retry and circuit breaker configs
    pub fn with_configs(
        config: PinataConfig,
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

    /// Add authentication headers to request
    fn add_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref jwt) = self.config.jwt {
            request.header("Authorization", format!("Bearer {}", jwt))
        } else {
            request
                .header("pinata_api_key", &self.config.api_key)
                .header("pinata_secret_api_key", &self.config.secret_key)
        }
    }

    /// Make an authenticated request to Pinata API
    fn api_request(&self, method: reqwest::Method, endpoint: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.config.api_url, endpoint);
        self.add_auth(self.client.request(method, &url))
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

    /// Upload with metadata
    pub async fn upload_with_metadata(
        &self,
        data: &[u8],
        metadata: PinataMetadata,
    ) -> IpfsResult<UploadResult> {
        self.check_circuit_breaker().await?;

        let start = Instant::now();
        let data_len = data.len();

        debug!(
            provider = self.name(),
            size_bytes = data_len,
            name = ?metadata.name,
            "Uploading to Pinata IPFS"
        );

        // Build multipart form
        let file_part = Part::bytes(data.to_vec())
            .file_name(metadata.name.clone().unwrap_or_else(|| "file".to_string()))
            .mime_str("application/octet-stream")
            .map_err(|e| IpfsError::Internal(format!("Failed to create form part: {}", e)))?;

        let mut form = Form::new().part("file", file_part);

        // Add metadata
        let metadata_json = serde_json::json!({
            "name": metadata.name.unwrap_or_else(|| "unnamed".to_string()),
            "keyvalues": metadata.key_values.unwrap_or_default()
        });
        form = form.text(
            "pinataMetadata",
            serde_json::to_string(&metadata_json)
                .map_err(|e| IpfsError::Internal(format!("Failed to serialize metadata: {}", e)))?,
        );

        // Add pinning options
        let options = PinataOptions::default();
        form = form.text(
            "pinataOptions",
            serde_json::to_string(&options)
                .map_err(|e| IpfsError::Internal(format!("Failed to serialize options: {}", e)))?,
        );

        // Make request
        let response = self
            .api_request(reqwest::Method::POST, "/pinning/pinFileToIPFS")
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
                IpfsError::AuthenticationFailed("Invalid Pinata credentials".into())
            } else if status.as_u16() == 429 {
                IpfsError::RateLimited("Pinata rate limit exceeded".into())
            } else if status.as_u16() == 413 {
                IpfsError::PayloadTooLarge {
                    size: data_len,
                    max: 100 * 1024 * 1024, // Pinata limit
                }
            } else {
                IpfsError::UploadFailed(format!("HTTP {}: {}", status, error_body))
            };
            self.record_failure(&error).await;
            return Err(error);
        }

        let pinata_response: PinataPinResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse response: {}", e))
        })?;

        let elapsed = start.elapsed();
        let latency_ms = elapsed.as_millis() as u64;

        self.record_success(data_len, latency_ms, true).await;

        let cid = Cid::new(&pinata_response.ipfs_hash);

        info!(
            provider = self.name(),
            cid = %cid,
            size_bytes = data_len,
            latency_ms = latency_ms,
            "Upload successful"
        );

        Ok(UploadResult {
            cid,
            size_bytes: pinata_response.pin_size as usize,
            provider: self.name().to_string(),
            upload_time_ms: latency_ms,
            pinned: true,
        })
    }

    /// Pin an existing CID by hash
    pub async fn pin_by_hash(&self, cid: &Cid, metadata: Option<PinataMetadata>) -> IpfsResult<()> {
        self.check_circuit_breaker().await?;

        debug!(
            provider = self.name(),
            cid = %cid,
            "Pinning by hash on Pinata"
        );

        let mut body = serde_json::json!({
            "hashToPin": cid.as_str()
        });

        if let Some(meta) = metadata {
            body["pinataMetadata"] = serde_json::json!({
                "name": meta.name.unwrap_or_else(|| format!("pin-{}", cid)),
                "keyvalues": meta.key_values.unwrap_or_default()
            });
        }

        let response = self
            .api_request(reqwest::Method::POST, "/pinning/pinByHash")
            .json(&body)
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let error = if error_body.contains("already pinned") {
                // Not really an error, content is already pinned
                return Ok(());
            } else {
                IpfsError::PinFailed(format!("HTTP {}: {}", status, error_body))
            };
            self.record_failure(&error).await;
            return Err(error);
        }

        self.circuit_breaker.record_success().await;
        self.stats.record_pin();

        info!(
            provider = self.name(),
            cid = %cid,
            "Pin by hash successful"
        );

        Ok(())
    }

    /// List all pins with optional filters
    pub async fn list_pins(
        &self,
        status: Option<&str>,
        page_limit: Option<u32>,
    ) -> IpfsResult<Vec<String>> {
        self.check_circuit_breaker().await?;

        let mut request = self.api_request(reqwest::Method::GET, "/data/pinList");

        if let Some(s) = status {
            request = request.query(&[("status", s)]);
        }
        if let Some(limit) = page_limit {
            request = request.query(&[("pageLimit", limit.to_string())]);
        }

        let response = request.send().await.map_err(|e| {
            IpfsError::NetworkError(e.to_string())
        })?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(IpfsError::Internal(format!(
                "Failed to list pins: {}",
                error_body
            )));
        }

        let pin_list: PinataPinListResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse pin list: {}", e))
        })?;

        Ok(pin_list.rows.into_iter().map(|r| r.ipfs_pin_hash).collect())
    }
}

#[async_trait]
impl IpfsProvider for PinataProvider {
    fn name(&self) -> &str {
        "pinata"
    }

    fn gateway_url(&self, cid: &Cid) -> String {
        format!("{}{}", self.config.gateway_url, cid)
    }

    async fn upload(&self, data: &[u8]) -> IpfsResult<UploadResult> {
        self.upload_with_metadata(data, PinataMetadata::default()).await
    }

    async fn download(&self, cid: &Cid) -> IpfsResult<Vec<u8>> {
        self.check_circuit_breaker().await?;

        let start = Instant::now();

        debug!(
            provider = self.name(),
            cid = %cid,
            "Downloading from Pinata gateway"
        );

        // Use gateway for downloads
        let url = self.gateway_url(cid);
        let response = self
            .client
            .get(&url)
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
            let error = if status.as_u16() == 404 {
                IpfsError::NotFound(cid.clone())
            } else {
                IpfsError::DownloadFailed(format!("HTTP {}", status))
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
        self.pin_by_hash(cid, None).await
    }

    async fn unpin(&self, cid: &Cid) -> IpfsResult<()> {
        self.check_circuit_breaker().await?;

        debug!(
            provider = self.name(),
            cid = %cid,
            "Unpinning from Pinata"
        );

        let response = self
            .api_request(
                reqwest::Method::DELETE,
                &format!("/pinning/unpin/{}", cid.as_str()),
            )
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();

            // 404 means already unpinned, which is fine
            if status.as_u16() == 404 {
                return Ok(());
            }

            let error = IpfsError::UnpinFailed(format!("HTTP {}: {}", status, error_body));
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
            .api_request(reqwest::Method::GET, "/data/pinList")
            .query(&[("hashContains", cid.as_str()), ("status", "pinned")])
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(IpfsError::Internal(format!(
                "Failed to check pin status: {}",
                error_body
            )));
        }

        let pin_list: PinataPinListResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse response: {}", e))
        })?;

        Ok(pin_list.count > 0)
    }

    async fn health_check(&self) -> IpfsResult<bool> {
        let response = self
            .api_request(reqwest::Method::GET, "/data/testAuthentication")
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

    fn test_config() -> PinataConfig {
        PinataConfig {
            api_url: "https://api.pinata.cloud".to_string(),
            gateway_url: "https://gateway.pinata.cloud/ipfs/".to_string(),
            api_key: "test_api_key".to_string(),
            secret_key: "test_secret_key".to_string(),
            jwt: None,
            timeout: Duration::from_secs(30),
            use_v2_api: false,
        }
    }

    #[test]
    fn test_gateway_url() {
        let provider = PinataProvider::new(test_config()).unwrap();
        let cid = Cid::new("QmTest123");

        assert_eq!(
            provider.gateway_url(&cid),
            "https://gateway.pinata.cloud/ipfs/QmTest123"
        );
    }

    #[test]
    fn test_provider_name() {
        let provider = PinataProvider::new(test_config()).unwrap();
        assert_eq!(provider.name(), "pinata");
    }

    #[test]
    fn test_metadata_builder() {
        let metadata = PinataMetadata::new("my-file.json")
            .with_key_value("app", "oilseed-chain")
            .with_key_value("stage", "farmer_registration");

        assert_eq!(metadata.name, Some("my-file.json".to_string()));
        let kv = metadata.key_values.unwrap();
        assert_eq!(kv.get("app"), Some(&"oilseed-chain".to_string()));
        assert_eq!(kv.get("stage"), Some(&"farmer_registration".to_string()));
    }

    #[tokio::test]
    async fn test_circuit_breaker_integration() {
        let provider = PinataProvider::new(test_config()).unwrap();

        // Circuit should be closed initially
        assert!(provider.check_circuit_breaker().await.is_ok());
    }

    #[test]
    fn test_config_with_jwt() {
        let provider_config = ProviderConfig {
            name: "pinata".to_string(),
            api_url: "https://api.pinata.cloud".to_string(),
            gateway_url: "https://gateway.pinata.cloud/ipfs/".to_string(),
            auth: Some(ProviderAuth::Bearer {
                token: "my_jwt_token".to_string(),
            }),
            timeout_seconds: 30,
            max_retries: 3,
            priority: 2,
            enabled: true,
        };

        let config = PinataConfig::from_provider_config(&provider_config).unwrap();
        assert_eq!(config.jwt, Some("my_jwt_token".to_string()));
        assert!(config.api_key.is_empty());
    }

    #[test]
    fn test_pinata_options_default() {
        let options = PinataOptions::default();
        assert_eq!(options.cid_version, 1);
    }
}
