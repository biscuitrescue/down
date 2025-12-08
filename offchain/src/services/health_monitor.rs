//! Health Monitor Service
//!
//! Provides continuous health monitoring for IPFS providers with:
//! - Periodic health checks
//! - Latency tracking
//! - Alert generation
//! - Provider availability metrics
//! - CID availability auditing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Health check result for a single provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthCheck {
    pub provider_name: String,
    pub is_healthy: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub checked_at: String,
    pub consecutive_failures: u32,
    pub uptime_percent: f64,
}

/// Overall health report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub timestamp: String,
    pub overall_healthy: bool,
    pub healthy_providers: usize,
    pub total_providers: usize,
    pub providers: Vec<ProviderHealthCheck>,
    pub alerts: Vec<HealthAlert>,
    pub cid_audit: Option<CidAuditReport>,
}

/// Health alert
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthAlert {
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub provider: Option<String>,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertType {
    ProviderDown,
    ProviderDegraded,
    HighLatency,
    AllProvidersDown,
    CidUnavailable,
    CidPartiallyAvailable,
    CircuitBreakerOpen,
    ErrorRateHigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

/// CID audit result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CidAuditReport {
    pub total_cids_checked: usize,
    pub available_everywhere: usize,
    pub partially_available: usize,
    pub unavailable: usize,
    pub details: Vec<CidAuditDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CidAuditDetail {
    pub cid: String,
    pub availability: HashMap<String, bool>,
    pub issue: Option<String>,
}

/// Configuration for the health monitor
#[derive(Debug, Clone)]
pub struct HealthMonitorConfig {
    /// Interval between health checks
    pub check_interval: Duration,
    /// Timeout for each health check
    pub check_timeout: Duration,
    /// Number of consecutive failures before marking unhealthy
    pub failure_threshold: u32,
    /// Latency threshold for degraded status (ms)
    pub latency_warning_threshold_ms: u64,
    /// Latency threshold for high latency alert (ms)
    pub latency_critical_threshold_ms: u64,
    /// Enable CID auditing
    pub enable_cid_audit: bool,
    /// Interval for CID audits
    pub cid_audit_interval: Duration,
    /// Maximum CIDs to audit per cycle
    pub max_cids_per_audit: usize,
    /// Alert channel buffer size
    pub alert_buffer_size: usize,
    /// Minimum providers required for healthy status
    pub min_healthy_providers: usize,
}

impl Default for HealthMonitorConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(30),
            check_timeout: Duration::from_secs(10),
            failure_threshold: 3,
            latency_warning_threshold_ms: 1000,
            latency_critical_threshold_ms: 5000,
            enable_cid_audit: true,
            cid_audit_interval: Duration::from_secs(300),
            max_cids_per_audit: 100,
            alert_buffer_size: 1000,
            min_healthy_providers: 1,
        }
    }
}

/// Provider health state
#[derive(Debug, Clone)]
struct ProviderState {
    name: String,
    is_healthy: bool,
    consecutive_failures: u32,
    total_checks: u64,
    successful_checks: u64,
    latency_samples: Vec<u64>,
    last_check: Option<Instant>,
    last_success: Option<Instant>,
    last_failure: Option<Instant>,
    last_error: Option<String>,
}

impl ProviderState {
    fn new(name: String) -> Self {
        Self {
            name,
            is_healthy: true,
            consecutive_failures: 0,
            total_checks: 0,
            successful_checks: 0,
            latency_samples: Vec::with_capacity(100),
            last_check: None,
            last_success: None,
            last_failure: None,
            last_error: None,
        }
    }

    fn record_success(&mut self, latency_ms: u64) {
        self.total_checks += 1;
        self.successful_checks += 1;
        self.consecutive_failures = 0;
        self.is_healthy = true;
        self.last_check = Some(Instant::now());
        self.last_success = Some(Instant::now());
        self.last_error = None;

        self.latency_samples.push(latency_ms);
        if self.latency_samples.len() > 100 {
            self.latency_samples.remove(0);
        }
    }

    fn record_failure(&mut self, error: String, threshold: u32) {
        self.total_checks += 1;
        self.consecutive_failures += 1;
        self.last_check = Some(Instant::now());
        self.last_failure = Some(Instant::now());
        self.last_error = Some(error);

        if self.consecutive_failures >= threshold {
            self.is_healthy = false;
        }
    }

    fn uptime_percent(&self) -> f64 {
        if self.total_checks == 0 {
            return 100.0;
        }
        (self.successful_checks as f64 / self.total_checks as f64) * 100.0
    }

    fn average_latency_ms(&self) -> Option<u64> {
        if self.latency_samples.is_empty() {
            return None;
        }
        Some(self.latency_samples.iter().sum::<u64>() / self.latency_samples.len() as u64)
    }

    fn p99_latency_ms(&self) -> Option<u64> {
        if self.latency_samples.is_empty() {
            return None;
        }
        let mut sorted = self.latency_samples.clone();
        sorted.sort();
        let idx = (sorted.len() as f64 * 0.99) as usize;
        Some(sorted[idx.min(sorted.len() - 1)])
    }
}

/// Trait for health-checkable providers
#[async_trait::async_trait]
pub trait HealthCheckable: Send + Sync {
    fn name(&self) -> &str;
    async fn health_check(&self) -> Result<(), String>;
    async fn is_cid_available(&self, cid: &str) -> Result<bool, String>;
}

/// Monitor statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HealthMonitorStats {
    pub total_checks: u64,
    pub successful_checks: u64,
    pub failed_checks: u64,
    pub alerts_generated: u64,
    pub cid_audits_completed: u64,
    pub is_running: bool,
}

/// The health monitor service
pub struct HealthMonitor {
    config: HealthMonitorConfig,
    providers: Arc<RwLock<HashMap<String, Arc<dyn HealthCheckable>>>>,
    state: Arc<RwLock<HashMap<String, ProviderState>>>,
    critical_cids: Arc<RwLock<Vec<String>>>,
    alert_sender: mpsc::Sender<HealthAlert>,
    alert_receiver: Arc<RwLock<Option<mpsc::Receiver<HealthAlert>>>>,
    shutdown_sender: broadcast::Sender<()>,
    is_running: Arc<AtomicBool>,
    stats: Arc<MonitorStatsInner>,
}

struct MonitorStatsInner {
    total_checks: AtomicU64,
    successful_checks: AtomicU64,
    failed_checks: AtomicU64,
    alerts_generated: AtomicU64,
    cid_audits_completed: AtomicU64,
}

impl Default for MonitorStatsInner {
    fn default() -> Self {
        Self {
            total_checks: AtomicU64::new(0),
            successful_checks: AtomicU64::new(0),
            failed_checks: AtomicU64::new(0),
            alerts_generated: AtomicU64::new(0),
            cid_audits_completed: AtomicU64::new(0),
        }
    }
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(config: HealthMonitorConfig) -> Self {
        let (alert_sender, alert_receiver) = mpsc::channel(config.alert_buffer_size);
        let (shutdown_sender, _) = broadcast::channel(1);

        Self {
            config,
            providers: Arc::new(RwLock::new(HashMap::new())),
            state: Arc::new(RwLock::new(HashMap::new())),
            critical_cids: Arc::new(RwLock::new(Vec::new())),
            alert_sender,
            alert_receiver: Arc::new(RwLock::new(Some(alert_receiver))),
            shutdown_sender,
            is_running: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(MonitorStatsInner::default()),
        }
    }

    /// Register a provider for monitoring
    pub async fn register_provider<P: HealthCheckable + 'static>(&self, provider: P) {
        let name = provider.name().to_string();

        let mut providers = self.providers.write().await;
        providers.insert(name.clone(), Arc::new(provider));

        let mut state = self.state.write().await;
        state.insert(name.clone(), ProviderState::new(name.clone()));

        info!(provider = %name, "Registered provider for health monitoring");
    }

    /// Add a CID to the critical monitoring list
    pub async fn add_critical_cid(&self, cid: String) {
        let mut cids = self.critical_cids.write().await;
        if !cids.contains(&cid) {
            cids.push(cid);
        }
    }

    /// Remove a CID from the critical monitoring list
    pub async fn remove_critical_cid(&self, cid: &str) {
        let mut cids = self.critical_cids.write().await;
        cids.retain(|c| c != cid);
    }

    /// Get the alert receiver (can only be called once)
    pub async fn take_alert_receiver(&self) -> Option<mpsc::Receiver<HealthAlert>> {
        let mut guard = self.alert_receiver.write().await;
        guard.take()
    }

    /// Start the health monitor
    pub async fn start(&self) {
        if self.is_running.swap(true, Ordering::SeqCst) {
            warn!("Health monitor already running");
            return;
        }

        info!(
            check_interval_secs = self.config.check_interval.as_secs(),
            "Starting health monitor"
        );

        // Spawn health check loop
        self.spawn_health_check_loop().await;

        // Spawn CID audit loop if enabled
        if self.config.enable_cid_audit {
            self.spawn_cid_audit_loop().await;
        }
    }

    async fn spawn_health_check_loop(&self) {
        let providers = self.providers.clone();
        let state = self.state.clone();
        let config = self.config.clone();
        let alert_sender = self.alert_sender.clone();
        let stats = self.stats.clone();
        let is_running = self.is_running.clone();
        let mut shutdown_rx = self.shutdown_sender.subscribe();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.check_interval);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !is_running.load(Ordering::Relaxed) {
                            break;
                        }

                        let provs = providers.read().await;
                        let mut alerts = Vec::new();
                        let mut healthy_count = 0;

                        for (name, provider) in provs.iter() {
                            let start = Instant::now();

                            let result = tokio::time::timeout(
                                config.check_timeout,
                                provider.health_check(),
                            )
                            .await;

                            let latency_ms = start.elapsed().as_millis() as u64;
                            stats.total_checks.fetch_add(1, Ordering::Relaxed);

                            let mut state_guard = state.write().await;
                            let provider_state = state_guard
                                .entry(name.clone())
                                .or_insert_with(|| ProviderState::new(name.clone()));

                            match result {
                                Ok(Ok(())) => {
                                    let was_unhealthy = !provider_state.is_healthy;
                                    provider_state.record_success(latency_ms);
                                    stats.successful_checks.fetch_add(1, Ordering::Relaxed);
                                    healthy_count += 1;

                                    // Check for high latency
                                    if latency_ms > config.latency_critical_threshold_ms {
                                        alerts.push(HealthAlert {
                                            alert_type: AlertType::HighLatency,
                                            severity: AlertSeverity::Error,
                                            provider: Some(name.clone()),
                                            message: format!(
                                                "Provider {} has critical latency: {}ms",
                                                name, latency_ms
                                            ),
                                            timestamp: chrono::Utc::now().to_rfc3339(),
                                        });
                                    } else if latency_ms > config.latency_warning_threshold_ms {
                                        alerts.push(HealthAlert {
                                            alert_type: AlertType::HighLatency,
                                            severity: AlertSeverity::Warning,
                                            provider: Some(name.clone()),
                                            message: format!(
                                                "Provider {} has high latency: {}ms",
                                                name, latency_ms
                                            ),
                                            timestamp: chrono::Utc::now().to_rfc3339(),
                                        });
                                    }

                                    // Recovery alert
                                    if was_unhealthy {
                                        info!(provider = %name, "Provider recovered");
                                        alerts.push(HealthAlert {
                                            alert_type: AlertType::ProviderDegraded,
                                            severity: AlertSeverity::Info,
                                            provider: Some(name.clone()),
                                            message: format!("Provider {} has recovered", name),
                                            timestamp: chrono::Utc::now().to_rfc3339(),
                                        });
                                    }

                                    debug!(
                                        provider = %name,
                                        latency_ms = latency_ms,
                                        "Health check passed"
                                    );
                                }
                                Ok(Err(e)) => {
                                    let was_healthy = provider_state.is_healthy;
                                    provider_state.record_failure(e.clone(), config.failure_threshold);
                                    stats.failed_checks.fetch_add(1, Ordering::Relaxed);

                                    if was_healthy && !provider_state.is_healthy {
                                        warn!(provider = %name, error = %e, "Provider marked unhealthy");
                                        alerts.push(HealthAlert {
                                            alert_type: AlertType::ProviderDown,
                                            severity: AlertSeverity::Error,
                                            provider: Some(name.clone()),
                                            message: format!(
                                                "Provider {} is down: {}",
                                                name, e
                                            ),
                                            timestamp: chrono::Utc::now().to_rfc3339(),
                                        });
                                    }
                                }
                                Err(_) => {
                                    let was_healthy = provider_state.is_healthy;
                                    provider_state.record_failure(
                                        "Health check timed out".to_string(),
                                        config.failure_threshold,
                                    );
                                    stats.failed_checks.fetch_add(1, Ordering::Relaxed);

                                    if was_healthy && !provider_state.is_healthy {
                                        warn!(provider = %name, "Provider health check timed out");
                                        alerts.push(HealthAlert {
                                            alert_type: AlertType::ProviderDown,
                                            severity: AlertSeverity::Error,
                                            provider: Some(name.clone()),
                                            message: format!(
                                                "Provider {} health check timed out",
                                                name
                                            ),
                                            timestamp: chrono::Utc::now().to_rfc3339(),
                                        });
                                    }
                                }
                            }
                        }

                        // Check if we have minimum healthy providers
                        if healthy_count < config.min_healthy_providers {
                            alerts.push(HealthAlert {
                                alert_type: AlertType::AllProvidersDown,
                                severity: AlertSeverity::Critical,
                                provider: None,
                                message: format!(
                                    "Only {} healthy providers, minimum required is {}",
                                    healthy_count, config.min_healthy_providers
                                ),
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            });
                        }

                        // Send alerts
                        for alert in alerts {
                            stats.alerts_generated.fetch_add(1, Ordering::Relaxed);
                            if alert_sender.send(alert).await.is_err() {
                                warn!("Alert channel full, dropping alert");
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Health check loop shutting down");
                        break;
                    }
                }
            }
        });
    }

    async fn spawn_cid_audit_loop(&self) {
        let providers = self.providers.clone();
        let critical_cids = self.critical_cids.clone();
        let config = self.config.clone();
        let alert_sender = self.alert_sender.clone();
        let stats = self.stats.clone();
        let is_running = self.is_running.clone();
        let mut shutdown_rx = self.shutdown_sender.subscribe();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.cid_audit_interval);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !is_running.load(Ordering::Relaxed) {
                            break;
                        }

                        let cids = critical_cids.read().await;
                        let provs = providers.read().await;

                        if cids.is_empty() || provs.is_empty() {
                            continue;
                        }

                        let cids_to_check: Vec<_> = cids
                            .iter()
                            .take(config.max_cids_per_audit)
                            .cloned()
                            .collect();

                        drop(cids);

                        debug!(count = cids_to_check.len(), "Starting CID audit");

                        for cid in cids_to_check {
                            let mut availability = HashMap::new();
                            let mut available_count = 0;

                            for (name, provider) in provs.iter() {
                                let result = tokio::time::timeout(
                                    config.check_timeout,
                                    provider.is_cid_available(&cid),
                                )
                                .await;

                                let is_available = match result {
                                    Ok(Ok(true)) => {
                                        available_count += 1;
                                        true
                                    }
                                    _ => false,
                                };

                                availability.insert(name.clone(), is_available);
                            }

                            // Generate alerts for unavailable CIDs
                            if available_count == 0 {
                                let alert = HealthAlert {
                                    alert_type: AlertType::CidUnavailable,
                                    severity: AlertSeverity::Critical,
                                    provider: None,
                                    message: format!(
                                        "Critical CID {} is not available on any provider",
                                        cid
                                    ),
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                };
                                stats.alerts_generated.fetch_add(1, Ordering::Relaxed);
                                let _ = alert_sender.send(alert).await;
                            } else if available_count < provs.len() {
                                let alert = HealthAlert {
                                    alert_type: AlertType::CidPartiallyAvailable,
                                    severity: AlertSeverity::Warning,
                                    provider: None,
                                    message: format!(
                                        "Critical CID {} is only available on {}/{} providers",
                                        cid, available_count, provs.len()
                                    ),
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                };
                                stats.alerts_generated.fetch_add(1, Ordering::Relaxed);
                                let _ = alert_sender.send(alert).await;
                            }
                        }

                        stats.cid_audits_completed.fetch_add(1, Ordering::Relaxed);
                    }
                    _ = shutdown_rx.recv() => {
                        info!("CID audit loop shutting down");
                        break;
                    }
                }
            }
        });
    }

    /// Run a single health check and get a report
    pub async fn check_now(&self) -> HealthReport {
        let providers = self.providers.read().await;
        let mut state_guard = self.state.write().await;
        let mut provider_checks = Vec::new();
        let mut alerts = Vec::new();
        let mut healthy_count = 0;

        for (name, provider) in providers.iter() {
            let start = Instant::now();

            let result = tokio::time::timeout(
                self.config.check_timeout,
                provider.health_check(),
            )
            .await;

            let latency_ms = start.elapsed().as_millis() as u64;

            let provider_state = state_guard
                .entry(name.clone())
                .or_insert_with(|| ProviderState::new(name.clone()));

            let (is_healthy, error) = match result {
                Ok(Ok(())) => {
                    provider_state.record_success(latency_ms);
                    healthy_count += 1;
                    (true, None)
                }
                Ok(Err(e)) => {
                    provider_state.record_failure(e.clone(), self.config.failure_threshold);
                    (false, Some(e))
                }
                Err(_) => {
                    provider_state.record_failure(
                        "Health check timed out".to_string(),
                        self.config.failure_threshold,
                    );
                    (false, Some("Timeout".to_string()))
                }
            };

            provider_checks.push(ProviderHealthCheck {
                provider_name: name.clone(),
                is_healthy,
                latency_ms: if is_healthy { Some(latency_ms) } else { None },
                error,
                checked_at: chrono::Utc::now().to_rfc3339(),
                consecutive_failures: provider_state.consecutive_failures,
                uptime_percent: provider_state.uptime_percent(),
            });
        }

        if healthy_count < self.config.min_healthy_providers {
            alerts.push(HealthAlert {
                alert_type: AlertType::AllProvidersDown,
                severity: AlertSeverity::Critical,
                provider: None,
                message: format!(
                    "Only {} healthy providers, minimum required is {}",
                    healthy_count, self.config.min_healthy_providers
                ),
                timestamp: chrono::Utc::now().to_rfc3339(),
            });
        }

        HealthReport {
            timestamp: chrono::Utc::now().to_rfc3339(),
            overall_healthy: healthy_count >= self.config.min_healthy_providers,
            healthy_providers: healthy_count,
            total_providers: providers.len(),
            providers: provider_checks,
            alerts,
            cid_audit: None,
        }
    }

    /// Audit critical CIDs now
    pub async fn audit_cids_now(&self) -> CidAuditReport {
        let providers = self.providers.read().await;
        let cids = self.critical_cids.read().await;

        let mut details = Vec::new();
        let mut available_everywhere = 0;
        let mut partially_available = 0;
        let mut unavailable = 0;

        for cid in cids.iter() {
            let mut availability = HashMap::new();
            let mut available_count = 0;

            for (name, provider) in providers.iter() {
                let result = tokio::time::timeout(
                    self.config.check_timeout,
                    provider.is_cid_available(cid),
                )
                .await;

                let is_available = matches!(result, Ok(Ok(true)));
                if is_available {
                    available_count += 1;
                }
                availability.insert(name.clone(), is_available);
            }

            let issue = if available_count == 0 {
                unavailable += 1;
                Some("Not available on any provider".to_string())
            } else if available_count < providers.len() {
                partially_available += 1;
                Some(format!(
                    "Only available on {}/{} providers",
                    available_count,
                    providers.len()
                ))
            } else {
                available_everywhere += 1;
                None
            };

            details.push(CidAuditDetail {
                cid: cid.clone(),
                availability,
                issue,
            });
        }

        CidAuditReport {
            total_cids_checked: cids.len(),
            available_everywhere,
            partially_available,
            unavailable,
            details,
        }
    }

    /// Get current statistics
    pub fn stats(&self) -> HealthMonitorStats {
        HealthMonitorStats {
            total_checks: self.stats.total_checks.load(Ordering::Relaxed),
            successful_checks: self.stats.successful_checks.load(Ordering::Relaxed),
            failed_checks: self.stats.failed_checks.load(Ordering::Relaxed),
            alerts_generated: self.stats.alerts_generated.load(Ordering::Relaxed),
            cid_audits_completed: self.stats.cid_audits_completed.load(Ordering::Relaxed),
            is_running: self.is_running.load(Ordering::Relaxed),
        }
    }

    /// Get provider state
    pub async fn get_provider_state(&self, name: &str) -> Option<ProviderHealthCheck> {
        let state = self.state.read().await;
        state.get(name).map(|s| ProviderHealthCheck {
            provider_name: s.name.clone(),
            is_healthy: s.is_healthy,
            latency_ms: s.average_latency_ms(),
            error: s.last_error.clone(),
            checked_at: chrono::Utc::now().to_rfc3339(),
            consecutive_failures: s.consecutive_failures,
            uptime_percent: s.uptime_percent(),
        })
    }

    /// Shutdown the health monitor
    pub async fn shutdown(&self) {
        info!("Shutting down health monitor");
        self.is_running.store(false, Ordering::SeqCst);
        let _ = self.shutdown_sender.send(());
    }

    /// Check if the monitor is running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }
}

/// Mock provider for testing
pub struct MockHealthCheckable {
    name: String,
    should_fail: bool,
    latency: Duration,
}

impl MockHealthCheckable {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            should_fail: false,
            latency: Duration::from_millis(10),
        }
    }

    pub fn failing(name: &str) -> Self {
        Self {
            name: name.to_string(),
            should_fail: true,
            latency: Duration::from_millis(10),
        }
    }
}

#[async_trait::async_trait]
impl HealthCheckable for MockHealthCheckable {
    fn name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> Result<(), String> {
        tokio::time::sleep(self.latency).await;
        if self.should_fail {
            Err("Mock health check failure".to_string())
        } else {
            Ok(())
        }
    }

    async fn is_cid_available(&self, _cid: &str) -> Result<bool, String> {
        Ok(!self.should_fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_monitor_creation() {
        let config = HealthMonitorConfig::default();
        let monitor = HealthMonitor::new(config);
        assert!(!monitor.is_running());
    }

    #[tokio::test]
    async fn test_register_provider() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register_provider(MockHealthCheckable::new("test")).await;

        let report = monitor.check_now().await;
        assert_eq!(report.total_providers, 1);
    }

    #[tokio::test]
    async fn test_check_healthy_provider() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register_provider(MockHealthCheckable::new("healthy")).await;

        let report = monitor.check_now().await;
        assert!(report.overall_healthy);
        assert_eq!(report.healthy_providers, 1);
        assert!(report.providers[0].is_healthy);
    }

    #[tokio::test]
    async fn test_check_unhealthy_provider() {
        let config = HealthMonitorConfig {
            failure_threshold: 1,
            ..Default::default()
        };
        let monitor = HealthMonitor::new(config);
        monitor.register_provider(MockHealthCheckable::failing("unhealthy")).await;

        let report = monitor.check_now().await;
        assert!(!report.overall_healthy);
        assert_eq!(report.healthy_providers, 0);
        assert!(!report.providers[0].is_healthy);
    }

    #[tokio::test]
    async fn test_critical_cid_management() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());

        monitor.add_critical_cid("QmTest123".to_string()).await;
        monitor.add_critical_cid("QmTest456".to_string()).await;

        let cids = monitor.critical_cids.read().await;
        assert_eq!(cids.len(), 2);
        drop(cids);

        monitor.remove_critical_cid("QmTest123").await;

        let cids = monitor.critical_cids.read().await;
        assert_eq!(cids.len(), 1);
        assert_eq!(cids[0], "QmTest456");
    }

    #[tokio::test]
    async fn test_cid_audit() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register_provider(MockHealthCheckable::new("provider1")).await;
        monitor.add_critical_cid("QmTestCid".to_string()).await;

        let audit = monitor.audit_cids_now().await;
        assert_eq!(audit.total_cids_checked, 1);
        assert_eq!(audit.available_everywhere, 1);
    }

    #[tokio::test]
    async fn test_stats() {
        let monitor = HealthMonitor::new(HealthMonitorConfig::default());
        monitor.register_provider(MockHealthCheckable::new("test")).await;

        let _ = monitor.check_now().await;

        let stats = monitor.stats();
        assert_eq!(stats.total_checks, 1);
        assert_eq!(stats.successful_checks, 1);
    }

    #[test]
    fn test_provider_state_uptime() {
        let mut state = ProviderState::new("test".to_string());

        state.record_success(100);
        state.record_success(100);
        state.record_failure("error".to_string(), 3);

        assert_eq!(state.total_checks, 3);
        assert_eq!(state.successful_checks, 2);
        assert!((state.uptime_percent() - 66.67).abs() < 1.0);
    }

    #[test]
    fn test_alert_severity_ordering() {
        assert!(AlertSeverity::Critical as u8 > AlertSeverity::Error as u8);
        assert!(AlertSeverity::Error as u8 > AlertSeverity::Warning as u8);
        assert!(AlertSeverity::Warning as u8 > AlertSeverity::Info as u8);
    }
}
