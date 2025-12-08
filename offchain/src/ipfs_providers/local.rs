//! Local/Self-hosted IPFS Provider Implementation
//!
//! Provides integration with self-hosted IPFS nodes (kubo/go-ipfs).
//! Supports both single node and cluster configurations.

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
use tracing::{debug, info, warn};

/// Local IPFS API response for add operation
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LocalAddResponse {
    hash: String,
    #[allow(dead_code)]
    name: String,
    size: String,
}

/// Local IPFS API response for version
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VersionResponse {
    version: String,
    #[allow(dead_code)]
    commit: Option<String>,
}

/// Local IPFS pin response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PinResponse {
    pins: Vec<String>,
}

/// Local IPFS pin ls response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PinLsResponse {
    keys: std::collections::HashMap<String, PinType>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PinType {
    r#type: String,
}

/// Local/Self-hosted IPFS Provider
pub struct LocalProvider {
    client: Client,
    config: LocalConfig,
    stats: Arc<ProviderStats>,
    circuit_breaker: Arc<CircuitBreaker>,
    retry_config: RetryConfig,
}

/// Configuration for local IPFS node
#[derive(Debug, Clone)]
pub struct LocalConfig {
    /// API URL (e.g., "http://localhost:5001")
    pub api_url: String,
    /// Gateway URL (e.g., "http://localhost:8080/ipfs/")
    pub gateway_url: String,
    /// Optional basic auth username
    pub username: Option<String>,
    /// Optional basic auth password
    pub password: Option<String>,
    /// Request timeout
    pub timeout: Duration,
    /// Whether to pin by default on upload
    pub pin_on_upload: bool,
    /// Whether this is an IPFS Cluster API
    pub is_cluster: bool,
    /// Cluster API URL (if using cluster)
    pub cluster_api_url: Option<String>,
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            api_url: "http://localhost:5001".to_string(),
            gateway_url: "http://localhost:8080/ipfs/".to_string(),
            username: None,
            password: None,
            timeout: Duration::from_secs(60),
            pin_on_upload: true,
            is_cluster: false,
            cluster_api_url: None,
        }
    }
}

impl LocalConfig {
    /// Create from environment variables
    pub fn from_env() -> Self {
        Self {
            api_url: std::env::var("IPFS_API_URL")
                .unwrap_or_else(|_| "http://localhost:5001".to_string()),
            gateway_url: std::env::var("IPFS_GATEWAY_URL")
                .unwrap_or_else(|_| "http://localhost:8080/ipfs/".to_string()),
            username: std::env::var("IPFS_API_USERNAME").ok(),
            password: std::env::var("IPFS_API_PASSWORD").ok(),
            timeout: Duration::from_secs(
                std::env::var("IPFS_TIMEOUT_SECONDS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(60),
            ),
            pin_on_upload: std::env::var("IPFS_PIN_ON_UPLOAD")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true),
            is_cluster: std::env::var("IPFS_IS_CLUSTER")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),
            cluster_api_url: std::env::var("IPFS_CLUSTER_API_URL").ok(),
        }
    }

    /// Create from generic provider config
    pub fn from_provider_config(config: &ProviderConfig) -> Self {
        let (username, password) = match &config.auth {
            Some(ProviderAuth::Basic { username, password }) => {
                (Some(username.clone()), Some(password.clone()))
            }
            _ => (None, None),
        };

        Self {
            api_url: config.api_url.clone(),
            gateway_url: config.gateway_url.clone(),
            username,
            password,
            timeout: Duration::from_secs(config.timeout_seconds),
            pin_on_upload: true,
            is_cluster: false,
            cluster_api_url: None,
        }
    }

    /// Configure for IPFS Cluster
    pub fn with_cluster(mut self, cluster_api_url: &str) -> Self {
        self.is_cluster = true;
        self.cluster_api_url = Some(cluster_api_url.to_string());
        self
    }
}

impl LocalProvider {
    /// Create a new local provider with default config
    pub fn new() -> Result<Self, IpfsError> {
        Self::with_config(LocalConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(config: LocalConfig) -> Result<Self, IpfsError> {
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
        Self::with_config(LocalConfig::from_env())
    }

    /// Create with custom retry and circuit breaker configs
    pub fn with_full_config(
        config: LocalConfig,
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

    /// Add authentication to request if configured
    fn add_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match (&self.config.username, &self.config.password) {
            (Some(user), Some(pass)) => request.basic_auth(user, Some(pass)),
            _ => request,
        }
    }

    /// Make an API request to the IPFS node
    fn api_request(&self, method: reqwest::Method, endpoint: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.config.api_url, endpoint);
        self.add_auth(self.client.request(method, &url))
    }

    /// Make an API request to the IPFS Cluster (if configured)
    fn cluster_request(&self, method: reqwest::Method, endpoint: &str) -> Option<reqwest::RequestBuilder> {
        self.config.cluster_api_url.as_ref().map(|cluster_url| {
            let url = format!("{}{}", cluster_url, endpoint);
            self.add_auth(self.client.request(method, &url))
        })
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

    /// Get IPFS node version
    pub async fn version(&self) -> IpfsResult<String> {
        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/version")
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(IpfsError::ProviderUnavailable(
                "Failed to get IPFS version".into(),
            ));
        }

        let version_info: VersionResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse version response: {}", e))
        })?;

        Ok(version_info.version)
    }

    /// Get node ID and peer info
    pub async fn id(&self) -> IpfsResult<String> {
        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/id")
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(IpfsError::ProviderUnavailable(
                "Failed to get IPFS node ID".into(),
            ));
        }

        let id_info: serde_json::Value = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse ID response: {}", e))
        })?;

        Ok(id_info["ID"].as_str().unwrap_or("unknown").to_string())
    }

    /// List all pinned CIDs
    pub async fn list_pins(&self, pin_type: Option<&str>) -> IpfsResult<Vec<String>> {
        let mut request = self.api_request(reqwest::Method::POST, "/api/v0/pin/ls");

        if let Some(t) = pin_type {
            request = request.query(&[("type", t)]);
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

        let pin_response: PinLsResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse pin list: {}", e))
        })?;

        Ok(pin_response.keys.into_keys().collect())
    }

    /// Run garbage collection
    pub async fn gc(&self) -> IpfsResult<()> {
        debug!(provider = self.name(), "Running garbage collection");

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/repo/gc")
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(IpfsError::Internal(format!(
                "Garbage collection failed: {}",
                error_body
            )));
        }

        info!(provider = self.name(), "Garbage collection completed");
        Ok(())
    }

    /// Get repo statistics
    pub async fn repo_stat(&self) -> IpfsResult<RepoStats> {
        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/repo/stat")
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(IpfsError::Internal(format!(
                "Failed to get repo stats: {}",
                error_body
            )));
        }

        let stats: RepoStats = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse repo stats: {}", e))
        })?;

        Ok(stats)
    }

    /// Add content to IPFS Cluster (if configured)
    async fn cluster_pin(&self, cid: &Cid) -> IpfsResult<()> {
        if let Some(request) = self.cluster_request(reqwest::Method::POST, &format!("/pins/{}", cid)) {
            debug!(
                provider = self.name(),
                cid = %cid,
                "Pinning to IPFS Cluster"
            );

            let response = request.send().await.map_err(|e| {
                IpfsError::NetworkError(format!("Cluster request failed: {}", e))
            })?;

            if !response.status().is_success() {
                let error_body = response.text().await.unwrap_or_default();
                warn!(
                    provider = self.name(),
                    cid = %cid,
                    error = %error_body,
                    "Cluster pin failed"
                );
                // Don't fail the overall operation if cluster pin fails
            } else {
                info!(
                    provider = self.name(),
                    cid = %cid,
                    "Cluster pin successful"
                );
            }
        }
        Ok(())
    }
}

/// Repository statistics
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RepoStats {
    pub num_objects: u64,
    pub repo_size: u64,
    pub storage_max: u64,
    pub repo_path: String,
    pub version: String,
}

impl RepoStats {
    /// Get usage percentage
    pub fn usage_percent(&self) -> f64 {
        if self.storage_max == 0 {
            0.0
        } else {
            (self.repo_size as f64 / self.storage_max as f64) * 100.0
        }
    }

    /// Get free space in bytes
    pub fn free_space(&self) -> u64 {
        self.storage_max.saturating_sub(self.repo_size)
    }
}

#[async_trait]
impl IpfsProvider for LocalProvider {
    fn name(&self) -> &str {
        if self.config.is_cluster {
            "local-cluster"
        } else {
            "local"
        }
    }

    fn gateway_url(&self, cid: &Cid) -> String {
        format!("{}{}", self.config.gateway_url, cid)
    }

    async fn upload(&self, data: &[u8]) -> IpfsResult<UploadResult> {
        self.check_circuit_breaker().await?;

        let start = Instant::now();
        let data_len = data.len();

        debug!(
            provider = self.name(),
            size_bytes = data_len,
            pin = self.config.pin_on_upload,
            "Uploading to local IPFS"
        );

        // Build multipart form
        let part = Part::bytes(data.to_vec())
            .file_name("file")
            .mime_str("application/octet-stream")
            .map_err(|e| IpfsError::Internal(format!("Failed to create form part: {}", e)))?;

        let form = Form::new().part("file", part);

        // Make request with optional pinning
        let mut request = self.api_request(reqwest::Method::POST, "/api/v0/add");
        if self.config.pin_on_upload {
            request = request.query(&[("pin", "true")]);
        }

        let response = request
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
            let error = IpfsError::UploadFailed(format!("HTTP {}: {}", status, error_body));
            self.record_failure(&error).await;
            return Err(error);
        }

        let add_response: LocalAddResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse response: {}", e))
        })?;

        let elapsed = start.elapsed();
        let latency_ms = elapsed.as_millis() as u64;

        self.record_success(data_len, latency_ms, true).await;

        let cid = Cid::new(&add_response.hash);

        // Also pin to cluster if configured
        if self.config.is_cluster {
            let _ = self.cluster_pin(&cid).await;
        }

        info!(
            provider = self.name(),
            cid = %cid,
            size_bytes = data_len,
            latency_ms = latency_ms,
            "Upload successful"
        );

        Ok(UploadResult {
            cid,
            size_bytes: add_response.size.parse().unwrap_or(data_len),
            provider: self.name().to_string(),
            upload_time_ms: latency_ms,
            pinned: self.config.pin_on_upload,
        })
    }

    async fn download(&self, cid: &Cid) -> IpfsResult<Vec<u8>> {
        self.check_circuit_breaker().await?;

        let start = Instant::now();

        debug!(
            provider = self.name(),
            cid = %cid,
            "Downloading from local IPFS"
        );

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/cat")
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
            let error = if status.as_u16() == 500 && error_body.contains("not found") {
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
            "Pinning on local IPFS"
        );

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/pin/add")
            .query(&[("arg", cid.as_str())])
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            let error = IpfsError::PinFailed(format!("HTTP {}: {}", status, error_body));
            self.record_failure(&error).await;
            return Err(error);
        }

        // Verify pin was added
        let pin_response: PinResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse pin response: {}", e))
        })?;

        if !pin_response.pins.contains(&cid.to_string()) {
            return Err(IpfsError::PinFailed("CID not in pin response".into()));
        }

        self.circuit_breaker.record_success().await;
        self.stats.record_pin();

        // Also pin to cluster if configured
        if self.config.is_cluster {
            let _ = self.cluster_pin(cid).await;
        }

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
            "Unpinning from local IPFS"
        );

        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/pin/rm")
            .query(&[("arg", cid.as_str())])
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();

            // "not pinned" is not really an error
            if error_body.contains("not pinned") {
                return Ok(());
            }

            let error = IpfsError::UnpinFailed(format!("HTTP {}: {}", status, error_body));
            self.record_failure(&error).await;
            return Err(error);
        }

        self.circuit_breaker.record_success().await;

        // Also unpin from cluster if configured
        if let Some(request) = self.cluster_request(reqwest::Method::DELETE, &format!("/pins/{}", cid)) {
            let _ = request.send().await;
        }

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
            .query(&[("arg", cid.as_str()), ("type", "all")])
            .send()
            .await
            .map_err(|e| IpfsError::NetworkError(e.to_string()))?;

        // 500 error with "not pinned" message means it's not pinned
        if response.status().as_u16() == 500 {
            let error_body = response.text().await.unwrap_or_default();
            if error_body.contains("not pinned") {
                return Ok(false);
            }
            return Err(IpfsError::Internal(format!(
                "Failed to check pin status: {}",
                error_body
            )));
        }

        if !response.status().is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(IpfsError::Internal(format!(
                "Failed to check pin status: {}",
                error_body
            )));
        }

        let pin_response: PinLsResponse = response.json().await.map_err(|e| {
            IpfsError::Internal(format!("Failed to parse response: {}", e))
        })?;

        Ok(pin_response.keys.contains_key(cid.as_str()))
    }

    async fn health_check(&self) -> IpfsResult<bool> {
        let response = self
            .api_request(reqwest::Method::POST, "/api/v0/id")
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

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new().expect("Failed to create default LocalProvider")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> LocalConfig {
        LocalConfig {
            api_url: "http://localhost:5001".to_string(),
            gateway_url: "http://localhost:8080/ipfs/".to_string(),
            username: None,
            password: None,
            timeout: Duration::from_secs(30),
            pin_on_upload: true,
            is_cluster: false,
            cluster_api_url: None,
        }
    }

    #[test]
    fn test_gateway_url() {
        let provider = LocalProvider::with_config(test_config()).unwrap();
        let cid = Cid::new("QmTest123");

        assert_eq!(
            provider.gateway_url(&cid),
            "http://localhost:8080/ipfs/QmTest123"
        );
    }

    #[test]
    fn test_provider_name() {
        let provider = LocalProvider::with_config(test_config()).unwrap();
        assert_eq!(provider.name(), "local");
    }

    #[test]
    fn test_cluster_provider_name() {
        let mut config = test_config();
        config.is_cluster = true;
        let provider = LocalProvider::with_config(config).unwrap();
        assert_eq!(provider.name(), "local-cluster");
    }

    #[test]
    fn test_config_with_cluster() {
        let config = LocalConfig::default().with_cluster("http://localhost:9094");
        assert!(config.is_cluster);
        assert_eq!(config.cluster_api_url, Some("http://localhost:9094".to_string()));
    }

    #[test]
    fn test_repo_stats_usage() {
        let stats = RepoStats {
            num_objects: 100,
            repo_size: 500_000_000,  // 500 MB
            storage_max: 10_000_000_000,  // 10 GB
            repo_path: "/data/ipfs".to_string(),
            version: "11".to_string(),
        };

        assert!((stats.usage_percent() - 5.0).abs() < 0.01);
        assert_eq!(stats.free_space(), 9_500_000_000);
    }

    #[tokio::test]
    async fn test_circuit_breaker_integration() {
        let provider = LocalProvider::with_config(test_config()).unwrap();
        assert!(provider.check_circuit_breaker().await.is_ok());
    }

    #[test]
    fn test_config_with_auth() {
        let config = LocalConfig {
            username: Some("admin".to_string()),
            password: Some("secret".to_string()),
            ..Default::default()
        };

        assert!(config.username.is_some());
        assert!(config.password.is_some());
    }
}
