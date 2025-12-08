//! IPFS Provider Router
//!
//! Provides intelligent routing across multiple IPFS providers with:
//! - Primary provider for writes with fallback chain
//! - Async secondary pinning for redundancy
//! - Load balancing for reads
//! - Circuit breaker integration
//! - Health-aware routing

use super::{
    Cid, CircuitBreaker, CircuitBreakerConfig, CircuitState, IpfsError, IpfsProvider, IpfsResult,
    ProviderHealth, ProviderStats, ProviderStatsSnapshot, UploadResult,
};
#[allow(unused_imports)]
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, info, warn};

/// Configuration for the provider router
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Whether to enable async secondary pinning
    pub enable_async_pinning: bool,
    /// Timeout for async pinning operations
    pub async_pin_timeout: Duration,
    /// Maximum retries for async pinning
    pub async_pin_retries: u32,
    /// Whether to enable read load balancing
    pub enable_read_load_balancing: bool,
    /// Health check interval
    pub health_check_interval: Duration,
    /// Minimum healthy providers required for operation
    pub min_healthy_providers: usize,
    /// Whether to fail fast if primary is unhealthy
    pub fail_fast_on_primary_unhealthy: bool,
    /// Channel buffer size for async operations
    pub channel_buffer_size: usize,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            enable_async_pinning: true,
            async_pin_timeout: Duration::from_secs(60),
            async_pin_retries: 3,
            enable_read_load_balancing: true,
            health_check_interval: Duration::from_secs(30),
            min_healthy_providers: 1,
            fail_fast_on_primary_unhealthy: false,
            channel_buffer_size: 1000,
        }
    }
}

/// Provider entry with metadata
struct ProviderEntry {
    provider: Arc<dyn IpfsProvider + Send + Sync>,
    priority: u32,
    circuit_breaker: CircuitBreaker,
    health: RwLock<ProviderHealthState>,
    stats: RwLock<RouterProviderStats>,
}

#[derive(Debug, Clone)]
struct ProviderHealthState {
    is_healthy: bool,
    last_check: Option<Instant>,
    last_success: Option<Instant>,
    last_failure: Option<Instant>,
    consecutive_failures: u32,
    latency_samples: Vec<u64>,
}

impl Default for ProviderHealthState {
    fn default() -> Self {
        Self {
            is_healthy: true, // Assume healthy until proven otherwise
            last_check: None,
            last_success: None,
            last_failure: None,
            consecutive_failures: 0,
            latency_samples: Vec::with_capacity(100),
        }
    }
}

impl ProviderHealthState {
    fn average_latency_ms(&self) -> Option<u64> {
        if self.latency_samples.is_empty() {
            return None;
        }
        Some(self.latency_samples.iter().sum::<u64>() / self.latency_samples.len() as u64)
    }

    fn record_success(&mut self, latency_ms: u64) {
        self.is_healthy = true;
        self.last_success = Some(Instant::now());
        self.consecutive_failures = 0;
        self.latency_samples.push(latency_ms);
        if self.latency_samples.len() > 100 {
            self.latency_samples.remove(0);
        }
    }

    fn record_failure(&mut self) {
        self.last_failure = Some(Instant::now());
        self.consecutive_failures += 1;
        if self.consecutive_failures >= 3 {
            self.is_healthy = false;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct RouterProviderStats {
    uploads: u64,
    downloads: u64,
    pins: u64,
    failures: u64,
    bytes_uploaded: u64,
    bytes_downloaded: u64,
}

/// Async pinning job
#[derive(Debug, Clone)]
pub struct PinningJob {
    pub cid: Cid,
    pub provider_name: String,
    pub priority: PinPriority,
    pub retry_count: u32,
    pub created_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinPriority {
    High,
    Normal,
    Low,
}

/// Result of an async pinning operation
#[derive(Debug, Clone)]
pub struct PinningResult {
    pub cid: Cid,
    pub provider_name: String,
    pub success: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Router statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterStats {
    pub total_uploads: u64,
    pub total_downloads: u64,
    pub total_failures: u64,
    pub async_pins_queued: u64,
    pub async_pins_completed: u64,
    pub async_pins_failed: u64,
    pub providers: Vec<RouterProviderStatsSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterProviderStatsSnapshot {
    pub name: String,
    pub priority: u32,
    pub is_healthy: bool,
    pub circuit_state: String,
    pub uploads: u64,
    pub downloads: u64,
    pub failures: u64,
    pub average_latency_ms: Option<u64>,
}

/// Multi-provider IPFS router
pub struct IpfsRouter {
    providers: Vec<ProviderEntry>,
    config: RouterConfig,
    pin_sender: mpsc::Sender<PinningJob>,
    pin_receiver: Arc<RwLock<Option<mpsc::Receiver<PinningJob>>>>,
    shutdown_sender: broadcast::Sender<()>,
    read_index: AtomicUsize,
    stats: RouterStatsInner,
}

struct RouterStatsInner {
    total_uploads: AtomicU64,
    total_downloads: AtomicU64,
    total_failures: AtomicU64,
    async_pins_queued: AtomicU64,
    async_pins_completed: AtomicU64,
    async_pins_failed: AtomicU64,
}

impl IpfsRouter {
    /// Create a new router with the given providers
    pub fn new(config: RouterConfig) -> Self {
        let (pin_sender, pin_receiver) = mpsc::channel(config.channel_buffer_size);
        let (shutdown_sender, _) = broadcast::channel(1);

        Self {
            providers: Vec::new(),
            config,
            pin_sender,
            pin_receiver: Arc::new(RwLock::new(Some(pin_receiver))),
            shutdown_sender,
            read_index: AtomicUsize::new(0),
            stats: RouterStatsInner {
                total_uploads: AtomicU64::new(0),
                total_downloads: AtomicU64::new(0),
                total_failures: AtomicU64::new(0),
                async_pins_queued: AtomicU64::new(0),
                async_pins_completed: AtomicU64::new(0),
                async_pins_failed: AtomicU64::new(0),
            },
        }
    }

    /// Add a provider with the given priority (lower = higher priority)
    pub fn add_provider<P: IpfsProvider + Send + Sync + 'static>(
        &mut self,
        provider: P,
        priority: u32,
    ) -> &mut Self {
        let entry = ProviderEntry {
            provider: Arc::new(provider),
            priority,
            circuit_breaker: CircuitBreaker::new(CircuitBreakerConfig::default()),
            health: RwLock::new(ProviderHealthState::default()),
            stats: RwLock::new(RouterProviderStats::default()),
        };

        self.providers.push(entry);
        // Sort by priority (lower = higher priority)
        self.providers.sort_by_key(|e| e.priority);
        self
    }

    /// Add a provider with custom circuit breaker config
    pub fn add_provider_with_config<P: IpfsProvider + Send + Sync + 'static>(
        &mut self,
        provider: P,
        priority: u32,
        circuit_config: CircuitBreakerConfig,
    ) -> &mut Self {
        let entry = ProviderEntry {
            provider: Arc::new(provider),
            priority,
            circuit_breaker: CircuitBreaker::new(circuit_config),
            health: RwLock::new(ProviderHealthState::default()),
            stats: RwLock::new(RouterProviderStats::default()),
        };

        self.providers.push(entry);
        self.providers.sort_by_key(|e| e.priority);
        self
    }

    /// Build the router and return it
    pub fn build(self) -> Self {
        self
    }

    /// Start the background pinning worker
    pub async fn start_pinning_worker(&self) {
        let receiver = {
            let mut guard = self.pin_receiver.write().await;
            guard.take()
        };

        let Some(mut receiver) = receiver else {
            warn!("Pinning worker already started or receiver unavailable");
            return;
        };

        let providers: Vec<_> = self
            .providers
            .iter()
            .map(|e| (e.provider.clone(), e.provider.name().to_string()))
            .collect();
        let mut shutdown = self.shutdown_sender.subscribe();
        let config = self.config.clone();
        let stats_completed = Arc::new(AtomicU64::new(0));
        let stats_failed = Arc::new(AtomicU64::new(0));

        // Clone counters for the task
        let completed_counter = stats_completed.clone();
        let failed_counter = stats_failed.clone();

        tokio::spawn(async move {
            info!("Starting async pinning worker");

            loop {
                tokio::select! {
                    Some(job) = receiver.recv() => {
                        debug!(cid = %job.cid, provider = %job.provider_name, "Processing pinning job");

                        let start = Instant::now();
                        let mut success = false;

                        // Find the provider
                        for (provider, name) in &providers {
                            if name == &job.provider_name {
                                let result = tokio::time::timeout(
                                    config.async_pin_timeout,
                                    provider.pin(&job.cid),
                                )
                                .await;

                                match result {
                                    Ok(Ok(())) => {
                                        success = true;
                                        completed_counter.fetch_add(1, Ordering::Relaxed);
                                        info!(
                                            cid = %job.cid,
                                            provider = %name,
                                            duration_ms = start.elapsed().as_millis() as u64,
                                            "Async pin successful"
                                        );
                                    }
                                    Ok(Err(e)) => {
                                        failed_counter.fetch_add(1, Ordering::Relaxed);
                                        warn!(
                                            cid = %job.cid,
                                            provider = %name,
                                            error = %e,
                                            "Async pin failed"
                                        );
                                    }
                                    Err(_) => {
                                        failed_counter.fetch_add(1, Ordering::Relaxed);
                                        warn!(
                                            cid = %job.cid,
                                            provider = %name,
                                            "Async pin timed out"
                                        );
                                    }
                                }
                                break;
                            }
                        }

                        if !success && job.retry_count < config.async_pin_retries {
                            // Re-queue with incremented retry count
                            debug!(
                                cid = %job.cid,
                                retry = job.retry_count + 1,
                                "Re-queueing failed pin job"
                            );
                        }
                    }
                    _ = shutdown.recv() => {
                        info!("Shutting down pinning worker");
                        break;
                    }
                }
            }
        });
    }

    /// Shutdown the router and its workers
    pub fn shutdown(&self) {
        let _ = self.shutdown_sender.send(());
    }

    /// Upload data to the primary provider with fallback
    pub async fn upload(&self, data: &[u8]) -> IpfsResult<UploadResult> {
        if self.providers.is_empty() {
            return Err(IpfsError::AllProvidersFailed {
                errors: vec!["No providers configured".to_string()],
            });
        }

        let mut errors = Vec::new();

        for entry in &self.providers {
            let name = entry.provider.name().to_string();

            // Check circuit breaker
            if !entry.circuit_breaker.should_allow().await {
                debug!(provider = %name, "Circuit breaker open, skipping");
                errors.push(format!("{}: circuit breaker open", name));
                continue;
            }

            // Check health if configured
            if self.config.fail_fast_on_primary_unhealthy {
                let health = entry.health.read().await;
                if !health.is_healthy {
                    debug!(provider = %name, "Provider unhealthy, skipping");
                    errors.push(format!("{}: unhealthy", name));
                    continue;
                }
            }

            let start = Instant::now();
            match entry.provider.upload(data).await {
                Ok(result) => {
                    let latency_ms = start.elapsed().as_millis() as u64;

                    // Update stats
                    entry.circuit_breaker.record_success().await;
                    {
                        let mut health = entry.health.write().await;
                        health.record_success(latency_ms);
                    }
                    {
                        let mut stats = entry.stats.write().await;
                        stats.uploads += 1;
                        stats.bytes_uploaded += result.size_bytes as u64;
                    }
                    self.stats.total_uploads.fetch_add(1, Ordering::Relaxed);

                    // Queue async pinning to secondary providers
                    if self.config.enable_async_pinning {
                        self.queue_secondary_pins(&result.cid, &name).await;
                    }

                    info!(
                        cid = %result.cid,
                        provider = %name,
                        size = result.size_bytes,
                        latency_ms = latency_ms,
                        "Upload successful"
                    );

                    return Ok(result);
                }
                Err(e) => {
                    entry.circuit_breaker.record_failure().await;
                    {
                        let mut health = entry.health.write().await;
                        health.record_failure();
                    }
                    {
                        let mut stats = entry.stats.write().await;
                        stats.failures += 1;
                    }

                    warn!(provider = %name, error = %e, "Upload failed, trying next provider");
                    errors.push(format!("{}: {}", name, e));
                }
            }
        }

        self.stats.total_failures.fetch_add(1, Ordering::Relaxed);
        Err(IpfsError::AllProvidersFailed { errors })
    }

    /// Download data with load balancing across healthy providers
    pub async fn download(&self, cid: &Cid) -> IpfsResult<Vec<u8>> {
        if self.providers.is_empty() {
            return Err(IpfsError::AllProvidersFailed {
                errors: vec!["No providers configured".to_string()],
            });
        }

        let mut errors = Vec::new();

        // Get providers to try (load balanced or priority order)
        let providers_to_try = if self.config.enable_read_load_balancing {
            self.get_load_balanced_providers().await
        } else {
            (0..self.providers.len()).collect()
        };

        for idx in providers_to_try {
            let entry = &self.providers[idx];
            let name = entry.provider.name().to_string();

            // Check circuit breaker
            if !entry.circuit_breaker.should_allow().await {
                debug!(provider = %name, "Circuit breaker open, skipping");
                errors.push(format!("{}: circuit breaker open", name));
                continue;
            }

            let start = Instant::now();
            match entry.provider.download(cid).await {
                Ok(data) => {
                    let latency_ms = start.elapsed().as_millis() as u64;

                    entry.circuit_breaker.record_success().await;
                    {
                        let mut health = entry.health.write().await;
                        health.record_success(latency_ms);
                    }
                    {
                        let mut stats = entry.stats.write().await;
                        stats.downloads += 1;
                        stats.bytes_downloaded += data.len() as u64;
                    }
                    self.stats.total_downloads.fetch_add(1, Ordering::Relaxed);

                    debug!(
                        cid = %cid,
                        provider = %name,
                        size = data.len(),
                        latency_ms = latency_ms,
                        "Download successful"
                    );

                    return Ok(data);
                }
                Err(e) => {
                    entry.circuit_breaker.record_failure().await;
                    {
                        let mut health = entry.health.write().await;
                        health.record_failure();
                    }
                    {
                        let mut stats = entry.stats.write().await;
                        stats.failures += 1;
                    }

                    warn!(provider = %name, cid = %cid, error = %e, "Download failed, trying next provider");
                    errors.push(format!("{}: {}", name, e));
                }
            }
        }

        self.stats.total_failures.fetch_add(1, Ordering::Relaxed);
        Err(IpfsError::AllProvidersFailed { errors })
    }

    /// Pin content to all providers
    pub async fn pin_to_all(&self, cid: &Cid) -> IpfsResult<Vec<(String, Result<(), IpfsError>)>> {
        let mut results = Vec::new();

        for entry in &self.providers {
            let name = entry.provider.name().to_string();
            let result = entry.provider.pin(cid).await;

            if result.is_ok() {
                let mut stats = entry.stats.write().await;
                stats.pins += 1;
            }

            results.push((name, result));
        }

        Ok(results)
    }

    /// Check if content is pinned on any provider
    pub async fn is_pinned_anywhere(&self, cid: &Cid) -> IpfsResult<bool> {
        for entry in &self.providers {
            if let Ok(true) = entry.provider.is_pinned(cid).await {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Check pin status across all providers
    pub async fn pin_status(&self, cid: &Cid) -> Vec<(String, Result<bool, IpfsError>)> {
        let mut results = Vec::new();

        for entry in &self.providers {
            let name = entry.provider.name().to_string();
            let result = entry.provider.is_pinned(cid).await;
            results.push((name, result));
        }

        results
    }

    /// Run health checks on all providers
    pub async fn health_check_all(&self) -> Vec<ProviderHealth> {
        let mut health_results = Vec::new();

        for entry in &self.providers {
            let name = entry.provider.name().to_string();
            let start = Instant::now();
            let is_healthy = entry.provider.health_check().await.is_ok();
            let latency_ms = start.elapsed().as_millis() as u64;

            // Update health state
            {
                let mut health = entry.health.write().await;
                health.last_check = Some(Instant::now());
                if is_healthy {
                    health.record_success(latency_ms);
                } else {
                    health.record_failure();
                }
            }

            let health_state = entry.health.read().await;
            let circuit_state = entry.circuit_breaker.state().await;

            health_results.push(ProviderHealth {
                provider_name: name,
                is_healthy,
                last_check: Some(chrono::Utc::now().to_rfc3339()),
                last_success: health_state
                    .last_success
                    .map(|_| chrono::Utc::now().to_rfc3339()),
                last_failure: health_state
                    .last_failure
                    .map(|_| chrono::Utc::now().to_rfc3339()),
                success_count: 0, // We don't track cumulative success count
                failure_count: health_state.consecutive_failures as u64,
                average_latency_ms: health_state.average_latency_ms(),
                circuit_breaker_state: circuit_state,
            });
        }

        health_results
    }

    /// Get router statistics
    pub async fn stats(&self) -> RouterStats {
        let mut provider_stats = Vec::new();

        for entry in &self.providers {
            let name = entry.provider.name().to_string();
            let stats = entry.stats.read().await;
            let health = entry.health.read().await;
            let circuit_state = entry.circuit_breaker.state().await;

            provider_stats.push(RouterProviderStatsSnapshot {
                name,
                priority: entry.priority,
                is_healthy: health.is_healthy,
                circuit_state: format!("{:?}", circuit_state),
                uploads: stats.uploads,
                downloads: stats.downloads,
                failures: stats.failures,
                average_latency_ms: health.average_latency_ms(),
            });
        }

        RouterStats {
            total_uploads: self.stats.total_uploads.load(Ordering::Relaxed),
            total_downloads: self.stats.total_downloads.load(Ordering::Relaxed),
            total_failures: self.stats.total_failures.load(Ordering::Relaxed),
            async_pins_queued: self.stats.async_pins_queued.load(Ordering::Relaxed),
            async_pins_completed: self.stats.async_pins_completed.load(Ordering::Relaxed),
            async_pins_failed: self.stats.async_pins_failed.load(Ordering::Relaxed),
            providers: provider_stats,
        }
    }

    /// Get the primary gateway base URL
    pub fn gateway_url(&self) -> Option<String> {
        let dummy_cid = Cid::new("QmDummy");
        self.providers
            .first()
            .map(|e| e.provider.gateway_url(&dummy_cid))
            .map(|url| {
                // Remove the CID suffix to get base URL
                url.rsplit_once("/ipfs/")
                    .map(|(base, _)| base.to_string())
                    .unwrap_or(url)
            })
    }

    /// Get gateway URL for a specific CID
    pub fn gateway_url_for_cid(&self, cid: &Cid) -> Option<String> {
        self.providers.first().map(|e| e.provider.gateway_url(cid))
    }

    // Internal: Queue async pins to secondary providers
    async fn queue_secondary_pins(&self, cid: &Cid, exclude_provider: &str) {
        for entry in self.providers.iter().skip(1) {
            let name = entry.provider.name().to_string();
            if name == exclude_provider {
                continue;
            }

            let job = PinningJob {
                cid: cid.clone(),
                provider_name: name.clone(),
                priority: PinPriority::Normal,
                retry_count: 0,
                created_at: Instant::now(),
            };

            if let Err(e) = self.pin_sender.try_send(job) {
                warn!(
                    cid = %cid,
                    provider = %name,
                    error = %e,
                    "Failed to queue async pin job"
                );
            } else {
                self.stats.async_pins_queued.fetch_add(1, Ordering::Relaxed);
                debug!(cid = %cid, provider = %name, "Queued async pin job");
            }
        }
    }

    // Internal: Get load-balanced provider indices
    async fn get_load_balanced_providers(&self) -> Vec<usize> {
        let mut indices: Vec<(usize, u32, bool, Option<u64>)> = Vec::new();

        for (idx, entry) in self.providers.iter().enumerate() {
            let health = entry.health.read().await;
            indices.push((
                idx,
                entry.priority,
                health.is_healthy,
                health.average_latency_ms(),
            ));
        }

        // Sort by: healthy first, then by priority, then by latency
        indices.sort_by(|a, b| {
            match (a.2, b.2) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    // Both same health status, compare by priority
                    match a.1.cmp(&b.1) {
                        std::cmp::Ordering::Equal => {
                            // Same priority, compare by latency
                            match (a.3, b.3) {
                                (Some(lat_a), Some(lat_b)) => lat_a.cmp(&lat_b),
                                (Some(_), None) => std::cmp::Ordering::Less,
                                (None, Some(_)) => std::cmp::Ordering::Greater,
                                (None, None) => std::cmp::Ordering::Equal,
                            }
                        }
                        other => other,
                    }
                }
            }
        });

        // Round-robin among equally-good providers
        let read_idx = self.read_index.fetch_add(1, Ordering::Relaxed);
        let mut result: Vec<usize> = indices.iter().map(|(idx, _, _, _)| *idx).collect();

        // Rotate the list based on read index for distribution
        if !result.is_empty() {
            let rotation = read_idx % result.len();
            result.rotate_left(rotation);
        }

        result
    }
}

/// Builder for IpfsRouter
pub struct IpfsRouterBuilder {
    config: RouterConfig,
    providers: Vec<(
        Box<dyn IpfsProvider + Send + Sync>,
        u32,
        Option<CircuitBreakerConfig>,
    )>,
}

impl IpfsRouterBuilder {
    pub fn new() -> Self {
        Self {
            config: RouterConfig::default(),
            providers: Vec::new(),
        }
    }

    pub fn with_config(mut self, config: RouterConfig) -> Self {
        self.config = config;
        self
    }

    pub fn add_provider<P: IpfsProvider + Send + Sync + 'static>(
        mut self,
        provider: P,
        priority: u32,
    ) -> Self {
        self.providers.push((Box::new(provider), priority, None));
        self
    }

    pub fn add_provider_with_circuit_config<P: IpfsProvider + Send + Sync + 'static>(
        mut self,
        provider: P,
        priority: u32,
        circuit_config: CircuitBreakerConfig,
    ) -> Self {
        self.providers
            .push((Box::new(provider), priority, Some(circuit_config)));
        self
    }

    pub fn enable_async_pinning(mut self, enabled: bool) -> Self {
        self.config.enable_async_pinning = enabled;
        self
    }

    pub fn enable_read_load_balancing(mut self, enabled: bool) -> Self {
        self.config.enable_read_load_balancing = enabled;
        self
    }

    pub fn health_check_interval(mut self, interval: Duration) -> Self {
        self.config.health_check_interval = interval;
        self
    }

    pub fn min_healthy_providers(mut self, min: usize) -> Self {
        self.config.min_healthy_providers = min;
        self
    }

    pub fn build(self) -> IpfsRouter {
        let (pin_sender, pin_receiver) = mpsc::channel(self.config.channel_buffer_size);
        let (shutdown_sender, _) = broadcast::channel(1);

        let mut providers: Vec<ProviderEntry> = self
            .providers
            .into_iter()
            .map(|(provider, priority, circuit_config)| {
                let circuit_breaker = match circuit_config {
                    Some(config) => CircuitBreaker::new(config),
                    None => CircuitBreaker::new(CircuitBreakerConfig::default()),
                };

                ProviderEntry {
                    provider: Arc::from(provider),
                    priority,
                    circuit_breaker,
                    health: RwLock::new(ProviderHealthState::default()),
                    stats: RwLock::new(RouterProviderStats::default()),
                }
            })
            .collect();

        // Sort by priority
        providers.sort_by_key(|e| e.priority);

        IpfsRouter {
            providers,
            config: self.config,
            pin_sender,
            pin_receiver: Arc::new(RwLock::new(Some(pin_receiver))),
            shutdown_sender,
            read_index: AtomicUsize::new(0),
            stats: RouterStatsInner {
                total_uploads: AtomicU64::new(0),
                total_downloads: AtomicU64::new(0),
                total_failures: AtomicU64::new(0),
                async_pins_queued: AtomicU64::new(0),
                async_pins_completed: AtomicU64::new(0),
                async_pins_failed: AtomicU64::new(0),
            },
        }
    }
}

impl Default for IpfsRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock provider for testing
    struct MockProvider {
        name: String,
        should_fail: bool,
        latency_ms: u64,
    }

    impl MockProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                should_fail: false,
                latency_ms: 10,
            }
        }

        fn failing(name: &str) -> Self {
            Self {
                name: name.to_string(),
                should_fail: true,
                latency_ms: 10,
            }
        }
    }

    #[async_trait]
    impl IpfsProvider for MockProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn gateway_url(&self, cid: &Cid) -> String {
            format!("https://{}.example.com/ipfs/{}", self.name, cid)
        }

        async fn upload(&self, data: &[u8]) -> IpfsResult<UploadResult> {
            tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;

            if self.should_fail {
                return Err(IpfsError::UploadFailed("Mock failure".to_string()));
            }

            Ok(UploadResult {
                cid: Cid::new(format!(
                    "Qm{}",
                    hex::encode(&data[..std::cmp::min(22, data.len())])
                )),
                size_bytes: data.len(),
                provider: self.name.clone(),
                upload_time_ms: self.latency_ms,
                pinned: true,
            })
        }

        async fn download(&self, cid: &Cid) -> IpfsResult<Vec<u8>> {
            tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;

            if self.should_fail {
                return Err(IpfsError::DownloadFailed("Mock failure".to_string()));
            }

            Ok(format!("data for {}", cid).into_bytes())
        }

        async fn pin(&self, _cid: &Cid) -> IpfsResult<()> {
            tokio::time::sleep(Duration::from_millis(self.latency_ms)).await;

            if self.should_fail {
                return Err(IpfsError::PinFailed("Mock failure".to_string()));
            }

            Ok(())
        }

        async fn unpin(&self, _cid: &Cid) -> IpfsResult<()> {
            if self.should_fail {
                return Err(IpfsError::UnpinFailed("Mock failure".to_string()));
            }
            Ok(())
        }

        async fn is_pinned(&self, _cid: &Cid) -> IpfsResult<bool> {
            Ok(!self.should_fail)
        }

        async fn health_check(&self) -> IpfsResult<bool> {
            if self.should_fail {
                return Err(IpfsError::ProviderUnavailable("Mock unhealthy".to_string()));
            }
            Ok(true)
        }

        fn stats(&self) -> ProviderStats {
            ProviderStats::new()
        }
    }

    #[tokio::test]
    async fn test_router_upload_with_fallback() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::failing("primary"), 1)
            .add_provider(MockProvider::new("secondary"), 2)
            .enable_async_pinning(false)
            .build();

        let data = b"test data";
        let result = router.upload(data).await;

        assert!(result.is_ok());
        let upload_result = result.unwrap();
        assert_eq!(upload_result.provider, "secondary");
    }

    #[tokio::test]
    async fn test_router_upload_primary_success() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::new("primary"), 1)
            .add_provider(MockProvider::new("secondary"), 2)
            .enable_async_pinning(false)
            .build();

        let data = b"test data";
        let result = router.upload(data).await;

        assert!(result.is_ok());
        let upload_result = result.unwrap();
        assert_eq!(upload_result.provider, "primary");
    }

    #[tokio::test]
    async fn test_router_all_providers_fail() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::failing("primary"), 1)
            .add_provider(MockProvider::failing("secondary"), 2)
            .enable_async_pinning(false)
            .build();

        let data = b"test data";
        let result = router.upload(data).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            IpfsError::AllProvidersFailed { errors } => {
                assert_eq!(errors.len(), 2);
            }
            _ => panic!("Expected AllProvidersFailed error"),
        }
    }

    #[tokio::test]
    async fn test_router_download_load_balanced() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::new("provider1"), 1)
            .add_provider(MockProvider::new("provider2"), 1)
            .enable_read_load_balancing(true)
            .build();

        let cid = Cid::new("QmTestCid123456789012345678901234567890123");

        // Multiple downloads should distribute across providers
        for _ in 0..4 {
            let result = router.download(&cid).await;
            assert!(result.is_ok());
        }

        let stats = router.stats().await;
        assert_eq!(stats.total_downloads, 4);
    }

    #[tokio::test]
    async fn test_router_health_check() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::new("healthy"), 1)
            .add_provider(MockProvider::failing("unhealthy"), 2)
            .build();

        let health = router.health_check_all().await;

        assert_eq!(health.len(), 2);
        assert!(health[0].is_healthy);
        assert!(!health[1].is_healthy);
    }

    #[tokio::test]
    async fn test_router_pin_status() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::new("provider1"), 1)
            .add_provider(MockProvider::failing("provider2"), 2)
            .build();

        let cid = Cid::new("QmTestCid123456789012345678901234567890123");
        let status = router.pin_status(&cid).await;

        assert_eq!(status.len(), 2);
        assert!(status[0].1.as_ref().unwrap_or(&false) == &true);
        assert!(status[1].1.as_ref().unwrap_or(&true) == &false);
    }

    #[tokio::test]
    async fn test_router_stats() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::new("primary"), 1)
            .enable_async_pinning(false)
            .build();

        let data = b"test data";
        let _ = router.upload(data).await;
        let _ = router.upload(data).await;

        let stats = router.stats().await;
        assert_eq!(stats.total_uploads, 2);
        assert_eq!(stats.providers.len(), 1);
        assert_eq!(stats.providers[0].uploads, 2);
    }

    #[tokio::test]
    async fn test_router_gateway_url() {
        let router = IpfsRouterBuilder::new()
            .add_provider(MockProvider::new("primary"), 1)
            .build();

        assert!(router.gateway_url().is_some());

        let cid = Cid::new("QmTestCid");
        let url = router.gateway_url_for_cid(&cid);
        assert!(url.is_some());
        assert!(url.unwrap().contains("QmTestCid"));
    }

    #[test]
    fn test_router_builder() {
        let builder = IpfsRouterBuilder::new()
            .enable_async_pinning(true)
            .enable_read_load_balancing(true)
            .min_healthy_providers(2)
            .health_check_interval(Duration::from_secs(60));

        let router = builder.add_provider(MockProvider::new("test"), 1).build();

        assert!(router.gateway_url().is_some());
    }
}
