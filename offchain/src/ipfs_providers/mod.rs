//! IPFS Provider Abstraction Layer
//!
//! Provides a unified interface for multiple IPFS providers with:
//! - Trait-based provider abstraction
//! - Automatic failover between providers
//! - Circuit breakers to prevent cascading failures
//! - Retry logic with exponential backoff
//! - Health monitoring for each provider
//!
//! Supported providers:
//! - Infura IPFS
//! - Pinata
//! - Self-hosted IPFS node
//! - Local IPFS node (development)

pub mod infura;
pub mod pinata;
pub mod local;
pub mod router;

pub use router::IpfsRouter;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Content Identifier (CID) wrapper
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cid(pub String);

impl Cid {
    pub fn new(cid: impl Into<String>) -> Self {
        Self(cid.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Check if this looks like a valid CID
    pub fn is_valid(&self) -> bool {
        // CIDv0: starts with Qm, 46 chars
        // CIDv1: starts with bafy, variable length
        (self.0.starts_with("Qm") && self.0.len() == 46)
            || (self.0.starts_with("bafy") && self.0.len() >= 52)
    }
}

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Cid {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Cid {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Result type for IPFS operations
pub type IpfsResult<T> = Result<T, IpfsError>;

/// IPFS operation errors
#[derive(Debug, thiserror::Error)]
pub enum IpfsError {
    #[error("Upload failed: {0}")]
    UploadFailed(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Pin failed: {0}")]
    PinFailed(String),

    #[error("Unpin failed: {0}")]
    UnpinFailed(String),

    #[error("CID not found: {0}")]
    NotFound(Cid),

    #[error("Provider unavailable: {0}")]
    ProviderUnavailable(String),

    #[error("Circuit breaker open for provider: {0}")]
    CircuitBreakerOpen(String),

    #[error("All providers failed: {errors:?}")]
    AllProvidersFailed { errors: Vec<String> },

    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    #[error("Invalid CID: {0}")]
    InvalidCid(String),

    #[error("Rate limited by provider: {0}")]
    RateLimited(String),

    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("Payload too large: {size} bytes (max: {max} bytes)")]
    PayloadTooLarge { size: usize, max: usize },

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IpfsError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::NetworkError(_)
                | Self::Timeout(_)
                | Self::ProviderUnavailable(_)
                | Self::RateLimited(_)
        )
    }

    /// Check if this error should trigger circuit breaker
    pub fn should_trip_circuit(&self) -> bool {
        matches!(
            self,
            Self::ProviderUnavailable(_)
                | Self::Timeout(_)
                | Self::NetworkError(_)
                | Self::AuthenticationFailed(_)
        )
    }
}

/// Upload result containing CID and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResult {
    pub cid: Cid,
    pub size_bytes: usize,
    pub provider: String,
    pub upload_time_ms: u64,
    pub pinned: bool,
}

/// Provider health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider_name: String,
    pub is_healthy: bool,
    pub last_check: Option<String>,
    pub last_success: Option<String>,
    pub last_failure: Option<String>,
    pub success_count: u64,
    pub failure_count: u64,
    pub average_latency_ms: Option<u64>,
    pub circuit_breaker_state: CircuitState,
}

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl Default for CircuitState {
    fn default() -> Self {
        Self::Closed
    }
}

/// Configuration for an IPFS provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub api_url: String,
    pub gateway_url: String,
    pub auth: Option<ProviderAuth>,
    pub timeout_seconds: u64,
    pub max_retries: u32,
    pub priority: u32,
    pub enabled: bool,
}

/// Provider authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderAuth {
    Basic { username: String, password: String },
    Bearer { token: String },
    ApiKey { key: String, secret: Option<String> },
    None,
}

/// Trait for IPFS providers
#[async_trait]
pub trait IpfsProvider: Send + Sync {
    /// Get the provider name
    fn name(&self) -> &str;

    /// Get the gateway URL for a CID
    fn gateway_url(&self, cid: &Cid) -> String;

    /// Upload bytes to IPFS
    async fn upload(&self, data: &[u8]) -> IpfsResult<UploadResult>;

    /// Download content by CID
    async fn download(&self, cid: &Cid) -> IpfsResult<Vec<u8>>;

    /// Pin content by CID
    async fn pin(&self, cid: &Cid) -> IpfsResult<()>;

    /// Unpin content by CID
    async fn unpin(&self, cid: &Cid) -> IpfsResult<()>;

    /// Check if content is pinned
    async fn is_pinned(&self, cid: &Cid) -> IpfsResult<bool>;

    /// Check provider health
    async fn health_check(&self) -> IpfsResult<bool>;

    /// Get provider statistics
    fn stats(&self) -> ProviderStats;
}

/// Provider statistics
#[derive(Debug, Clone, Default)]
pub struct ProviderStats {
    pub uploads: AtomicU64,
    pub downloads: AtomicU64,
    pub pins: AtomicU64,
    pub failures: AtomicU64,
    pub bytes_uploaded: AtomicU64,
    pub bytes_downloaded: AtomicU64,
    pub total_latency_ms: AtomicU64,
}

impl ProviderStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_upload(&self, bytes: usize, latency_ms: u64) {
        self.uploads.fetch_add(1, Ordering::Relaxed);
        self.bytes_uploaded.fetch_add(bytes as u64, Ordering::Relaxed);
        self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
    }

    pub fn record_download(&self, bytes: usize, latency_ms: u64) {
        self.downloads.fetch_add(1, Ordering::Relaxed);
        self.bytes_downloaded.fetch_add(bytes as u64, Ordering::Relaxed);
        self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
    }

    pub fn record_pin(&self) {
        self.pins.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn average_latency_ms(&self) -> Option<u64> {
        let total_ops = self.uploads.load(Ordering::Relaxed)
            + self.downloads.load(Ordering::Relaxed);
        if total_ops == 0 {
            None
        } else {
            Some(self.total_latency_ms.load(Ordering::Relaxed) / total_ops)
        }
    }

    pub fn to_snapshot(&self) -> ProviderStatsSnapshot {
        ProviderStatsSnapshot {
            uploads: self.uploads.load(Ordering::Relaxed),
            downloads: self.downloads.load(Ordering::Relaxed),
            pins: self.pins.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            bytes_uploaded: self.bytes_uploaded.load(Ordering::Relaxed),
            bytes_downloaded: self.bytes_downloaded.load(Ordering::Relaxed),
            average_latency_ms: self.average_latency_ms(),
        }
    }
}

/// Snapshot of provider stats (for serialization)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatsSnapshot {
    pub uploads: u64,
    pub downloads: u64,
    pub pins: u64,
    pub failures: u64,
    pub bytes_uploaded: u64,
    pub bytes_downloaded: u64,
    pub average_latency_ms: Option<u64>,
}

/// Circuit breaker for provider failover
pub struct CircuitBreaker {
    state: RwLock<CircuitBreakerState>,
    config: CircuitBreakerConfig,
}

struct CircuitBreakerState {
    state: CircuitState,
    failure_count: u32,
    last_failure: Option<Instant>,
    last_success: Option<Instant>,
    half_open_successes: u32,
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening circuit
    pub failure_threshold: u32,
    /// Time to wait before trying half-open
    pub reset_timeout: Duration,
    /// Number of successes in half-open to close circuit
    pub half_open_successes_required: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(60),
            half_open_successes_required: 3,
        }
    }
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: RwLock::new(CircuitBreakerState {
                state: CircuitState::Closed,
                failure_count: 0,
                last_failure: None,
                last_success: None,
                half_open_successes: 0,
            }),
            config,
        }
    }

    /// Check if request should be allowed
    pub async fn should_allow(&self) -> bool {
        let mut state = self.state.write().await;

        match state.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if we should transition to half-open
                if let Some(last_failure) = state.last_failure {
                    if last_failure.elapsed() >= self.config.reset_timeout {
                        state.state = CircuitState::HalfOpen;
                        state.half_open_successes = 0;
                        tracing::info!("Circuit breaker transitioning to half-open");
                        return true;
                    }
                }
                false
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record a successful operation
    pub async fn record_success(&self) {
        let mut state = self.state.write().await;
        state.last_success = Some(Instant::now());

        match state.state {
            CircuitState::Closed => {
                // Reset failure count on success
                state.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                state.half_open_successes += 1;
                if state.half_open_successes >= self.config.half_open_successes_required {
                    state.state = CircuitState::Closed;
                    state.failure_count = 0;
                    tracing::info!("Circuit breaker closed after successful half-open period");
                }
            }
            CircuitState::Open => {
                // Shouldn't happen, but handle gracefully
                state.state = CircuitState::HalfOpen;
            }
        }
    }

    /// Record a failed operation
    pub async fn record_failure(&self) {
        let mut state = self.state.write().await;
        state.last_failure = Some(Instant::now());
        state.failure_count += 1;

        match state.state {
            CircuitState::Closed => {
                if state.failure_count >= self.config.failure_threshold {
                    state.state = CircuitState::Open;
                    tracing::warn!(
                        failures = state.failure_count,
                        "Circuit breaker opened due to failures"
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Single failure in half-open returns to open
                state.state = CircuitState::Open;
                state.half_open_successes = 0;
                tracing::warn!("Circuit breaker re-opened from half-open state");
            }
            CircuitState::Open => {
                // Already open, just update failure count
            }
        }
    }

    /// Get current circuit state
    pub async fn state(&self) -> CircuitState {
        self.state.read().await.state
    }

    /// Force reset the circuit breaker
    pub async fn reset(&self) {
        let mut state = self.state.write().await;
        state.state = CircuitState::Closed;
        state.failure_count = 0;
        state.half_open_successes = 0;
        tracing::info!("Circuit breaker manually reset");
    }
}

/// Retry configuration
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Calculate backoff duration for a given attempt
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let backoff_ms = self.initial_backoff.as_millis() as f64
            * self.backoff_multiplier.powi(attempt as i32);
        let backoff = Duration::from_millis(backoff_ms as u64);
        std::cmp::min(backoff, self.max_backoff)
    }
}

/// Execute an async operation with retry logic
pub async fn with_retry<T, F, Fut>(
    config: &RetryConfig,
    operation_name: &str,
    mut operation: F,
) -> IpfsResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = IpfsResult<T>>,
{
    let mut last_error = None;

    for attempt in 0..=config.max_retries {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                if !e.is_retryable() || attempt == config.max_retries {
                    return Err(e);
                }

                let backoff = config.backoff_for_attempt(attempt);
                tracing::warn!(
                    operation = operation_name,
                    attempt = attempt + 1,
                    max_retries = config.max_retries,
                    backoff_ms = backoff.as_millis(),
                    error = %e,
                    "Operation failed, retrying"
                );

                tokio::time::sleep(backoff).await;
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| IpfsError::Internal("Retry loop exited unexpectedly".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cid_validation() {
        // Valid CIDv0
        let cid_v0 = Cid::new("QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG");
        assert!(cid_v0.is_valid());

        // Valid CIDv1
        let cid_v1 = Cid::new("bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi");
        assert!(cid_v1.is_valid());

        // Invalid CID
        let invalid = Cid::new("invalid-cid");
        assert!(!invalid.is_valid());
    }

    #[test]
    fn test_retry_backoff() {
        let config = RetryConfig::default();

        let b0 = config.backoff_for_attempt(0);
        let b1 = config.backoff_for_attempt(1);
        let b2 = config.backoff_for_attempt(2);

        assert!(b1 > b0);
        assert!(b2 > b1);
        assert!(b2 <= config.max_backoff);
    }

    #[test]
    fn test_error_retryable() {
        assert!(IpfsError::NetworkError("test".into()).is_retryable());
        assert!(IpfsError::Timeout(Duration::from_secs(1)).is_retryable());
        assert!(!IpfsError::InvalidCid("test".into()).is_retryable());
        assert!(!IpfsError::NotFound(Cid::new("test")).is_retryable());
    }

    #[tokio::test]
    async fn test_circuit_breaker_states() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            reset_timeout: Duration::from_millis(100),
            half_open_successes_required: 1,
        });

        // Initially closed
        assert_eq!(cb.state().await, CircuitState::Closed);
        assert!(cb.should_allow().await);

        // Record failures to open
        cb.record_failure().await;
        assert_eq!(cb.state().await, CircuitState::Closed);
        cb.record_failure().await;
        assert_eq!(cb.state().await, CircuitState::Open);

        // Should not allow when open
        assert!(!cb.should_allow().await);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should transition to half-open
        assert!(cb.should_allow().await);
        assert_eq!(cb.state().await, CircuitState::HalfOpen);

        // Success in half-open should close
        cb.record_success().await;
        assert_eq!(cb.state().await, CircuitState::Closed);
    }

    #[test]
    fn test_provider_stats() {
        let stats = ProviderStats::new();

        stats.record_upload(1000, 50);
        stats.record_upload(2000, 100);
        stats.record_download(500, 25);

        let snapshot = stats.to_snapshot();
        assert_eq!(snapshot.uploads, 2);
        assert_eq!(snapshot.downloads, 1);
        assert_eq!(snapshot.bytes_uploaded, 3000);
        assert_eq!(snapshot.bytes_downloaded, 500);
        assert!(snapshot.average_latency_ms.is_some());
    }
}
