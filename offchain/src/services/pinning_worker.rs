//! Background Pinning Worker
//!
//! Manages asynchronous content pinning across multiple IPFS providers.
//! Features:
//! - Priority-based job queue
//! - Retry with exponential backoff
//! - Dead letter queue for failed jobs
//! - Persistence support for job recovery
//! - Metrics and monitoring

use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, error, info, warn};

/// Pinning job priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PinPriority {
    /// Critical content that must be pinned immediately
    Critical = 3,
    /// High priority (farmer registrations, fraud reports)
    High = 2,
    /// Normal priority (regular supply chain data)
    Normal = 1,
    /// Low priority (historical data, backfills)
    Low = 0,
}

impl Default for PinPriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Status of a pinning job
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinningJobStatus {
    /// Job is queued and waiting
    Pending,
    /// Job is currently being processed
    InProgress,
    /// Job completed successfully
    Completed,
    /// Job failed but may be retried
    Failed,
    /// Job failed permanently (max retries exceeded)
    DeadLettered,
    /// Job was cancelled
    Cancelled,
}

/// A pinning job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinningJob {
    /// Unique job ID
    pub id: String,
    /// CID to pin
    pub cid: String,
    /// Target provider name (or "all" for all providers)
    pub provider: String,
    /// Job priority
    pub priority: PinPriority,
    /// Current status
    pub status: PinningJobStatus,
    /// Number of retry attempts
    pub retry_count: u32,
    /// Maximum retries allowed
    pub max_retries: u32,
    /// When the job was created
    pub created_at: String,
    /// When the job was last updated
    pub updated_at: String,
    /// Error message if failed
    pub last_error: Option<String>,
    /// Metadata for tracking (e.g., stage, entity_id)
    pub metadata: HashMap<String, String>,
}

impl PinningJob {
    pub fn new(cid: String, provider: String) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            cid,
            provider,
            priority: PinPriority::Normal,
            status: PinningJobStatus::Pending,
            retry_count: 0,
            max_retries: 3,
            created_at: now.clone(),
            updated_at: now,
            last_error: None,
            metadata: HashMap::new(),
        }
    }

    pub fn with_priority(mut self, priority: PinPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }

    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }

    pub fn mark_in_progress(&mut self) {
        self.status = PinningJobStatus::InProgress;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn mark_completed(&mut self) {
        self.status = PinningJobStatus::Completed;
        self.updated_at = chrono::Utc::now().to_rfc3339();
        self.last_error = None;
    }

    pub fn mark_failed(&mut self, error: &str) {
        self.retry_count += 1;
        self.last_error = Some(error.to_string());
        self.updated_at = chrono::Utc::now().to_rfc3339();

        if self.can_retry() {
            self.status = PinningJobStatus::Failed;
        } else {
            self.status = PinningJobStatus::DeadLettered;
        }
    }

    pub fn mark_cancelled(&mut self) {
        self.status = PinningJobStatus::Cancelled;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

// Wrapper for priority queue ordering
#[derive(Debug, Clone)]
struct PrioritizedJob {
    job: PinningJob,
    scheduled_at: Instant,
}

impl PartialEq for PrioritizedJob {
    fn eq(&self, other: &Self) -> bool {
        self.job.id == other.job.id
    }
}

impl Eq for PrioritizedJob {}

impl PartialOrd for PrioritizedJob {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedJob {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first, then earlier scheduled time
        match self.job.priority.cmp(&other.job.priority) {
            std::cmp::Ordering::Equal => other.scheduled_at.cmp(&self.scheduled_at),
            other => other,
        }
    }
}

/// Configuration for the pinning worker
#[derive(Debug, Clone)]
pub struct PinningWorkerConfig {
    /// Number of concurrent workers
    pub worker_count: usize,
    /// Maximum queue size
    pub max_queue_size: usize,
    /// Initial retry backoff
    pub initial_backoff: Duration,
    /// Maximum retry backoff
    pub max_backoff: Duration,
    /// Backoff multiplier
    pub backoff_multiplier: f64,
    /// Job timeout
    pub job_timeout: Duration,
    /// Dead letter queue size
    pub dlq_size: usize,
    /// How often to persist queue state (if persistence enabled)
    pub persistence_interval: Duration,
    /// Enable job persistence
    pub enable_persistence: bool,
}

impl Default for PinningWorkerConfig {
    fn default() -> Self {
        Self {
            worker_count: 4,
            max_queue_size: 10000,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(300),
            backoff_multiplier: 2.0,
            job_timeout: Duration::from_secs(60),
            dlq_size: 1000,
            persistence_interval: Duration::from_secs(30),
            enable_persistence: false,
        }
    }
}

impl PinningWorkerConfig {
    pub fn backoff_for_retry(&self, retry_count: u32) -> Duration {
        let backoff_ms = self.initial_backoff.as_millis() as f64
            * self.backoff_multiplier.powi(retry_count as i32);
        let capped = backoff_ms.min(self.max_backoff.as_millis() as f64);
        Duration::from_millis(capped as u64)
    }
}

/// Pinning worker statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PinningWorkerStats {
    pub jobs_queued: u64,
    pub jobs_completed: u64,
    pub jobs_failed: u64,
    pub jobs_dead_lettered: u64,
    pub jobs_in_progress: u64,
    pub total_retries: u64,
    pub queue_size: usize,
    pub dlq_size: usize,
    pub average_processing_time_ms: u64,
}

/// Trait for IPFS pinning operations
#[async_trait::async_trait]
pub trait PinningProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn pin(&self, cid: &str) -> Result<(), String>;
    async fn is_pinned(&self, cid: &str) -> Result<bool, String>;
}

/// The main pinning worker
pub struct PinningWorker {
    config: PinningWorkerConfig,
    queue: Arc<RwLock<BinaryHeap<PrioritizedJob>>>,
    in_progress: Arc<RwLock<HashMap<String, PinningJob>>>,
    completed: Arc<RwLock<VecDeque<PinningJob>>>,
    dead_letter_queue: Arc<RwLock<VecDeque<PinningJob>>>,
    providers: Arc<RwLock<HashMap<String, Arc<dyn PinningProvider>>>>,
    job_sender: mpsc::Sender<PinningJob>,
    job_receiver: Arc<RwLock<Option<mpsc::Receiver<PinningJob>>>>,
    shutdown: Arc<AtomicBool>,
    shutdown_sender: broadcast::Sender<()>,
    stats: Arc<WorkerStatsInner>,
    is_running: Arc<AtomicBool>,
}

struct WorkerStatsInner {
    jobs_queued: AtomicU64,
    jobs_completed: AtomicU64,
    jobs_failed: AtomicU64,
    jobs_dead_lettered: AtomicU64,
    total_retries: AtomicU64,
    total_processing_time_ms: AtomicU64,
}

impl Default for WorkerStatsInner {
    fn default() -> Self {
        Self {
            jobs_queued: AtomicU64::new(0),
            jobs_completed: AtomicU64::new(0),
            jobs_failed: AtomicU64::new(0),
            jobs_dead_lettered: AtomicU64::new(0),
            total_retries: AtomicU64::new(0),
            total_processing_time_ms: AtomicU64::new(0),
        }
    }
}

impl PinningWorker {
    /// Create a new pinning worker
    pub fn new(config: PinningWorkerConfig) -> Self {
        let (job_sender, job_receiver) = mpsc::channel(config.max_queue_size);
        let (shutdown_sender, _) = broadcast::channel(1);

        Self {
            config,
            queue: Arc::new(RwLock::new(BinaryHeap::new())),
            in_progress: Arc::new(RwLock::new(HashMap::new())),
            completed: Arc::new(RwLock::new(VecDeque::with_capacity(1000))),
            dead_letter_queue: Arc::new(RwLock::new(VecDeque::new())),
            providers: Arc::new(RwLock::new(HashMap::new())),
            job_sender,
            job_receiver: Arc::new(RwLock::new(Some(job_receiver))),
            shutdown: Arc::new(AtomicBool::new(false)),
            shutdown_sender,
            stats: Arc::new(WorkerStatsInner::default()),
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Register a pinning provider
    pub async fn register_provider<P: PinningProvider + 'static>(&self, provider: P) {
        let name = provider.name().to_string();
        let mut providers = self.providers.write().await;
        providers.insert(name.clone(), Arc::new(provider));
        info!(provider = %name, "Registered pinning provider");
    }

    /// Submit a job to the queue
    pub async fn submit(&self, job: PinningJob) -> Result<String, String> {
        let queue_size = self.queue.read().await.len();
        if queue_size >= self.config.max_queue_size {
            return Err("Queue is full".to_string());
        }

        let job_id = job.id.clone();

        if let Err(e) = self.job_sender.send(job.clone()).await {
            return Err(format!("Failed to submit job: {}", e));
        }

        self.stats.jobs_queued.fetch_add(1, Ordering::Relaxed);
        debug!(job_id = %job_id, cid = %job.cid, "Job submitted to queue");

        Ok(job_id)
    }

    /// Submit a job directly to the priority queue (for internal use/retries)
    async fn queue_job(&self, job: PinningJob) {
        let mut queue = self.queue.write().await;
        queue.push(PrioritizedJob {
            job,
            scheduled_at: Instant::now(),
        });
    }

    /// Get job status
    pub async fn get_job_status(&self, job_id: &str) -> Option<PinningJob> {
        // Check in-progress
        {
            let in_progress = self.in_progress.read().await;
            if let Some(job) = in_progress.get(job_id) {
                return Some(job.clone());
            }
        }

        // Check queue
        {
            let queue = self.queue.read().await;
            for pj in queue.iter() {
                if pj.job.id == job_id {
                    return Some(pj.job.clone());
                }
            }
        }

        // Check completed
        {
            let completed = self.completed.read().await;
            for job in completed.iter() {
                if job.id == job_id {
                    return Some(job.clone());
                }
            }
        }

        // Check DLQ
        {
            let dlq = self.dead_letter_queue.read().await;
            for job in dlq.iter() {
                if job.id == job_id {
                    return Some(job.clone());
                }
            }
        }

        None
    }

    /// Get current statistics
    pub async fn stats(&self) -> PinningWorkerStats {
        let queue_size = self.queue.read().await.len();
        let dlq_size = self.dead_letter_queue.read().await.len();
        let in_progress = self.in_progress.read().await.len();

        let completed = self.stats.jobs_completed.load(Ordering::Relaxed);
        let total_time = self.stats.total_processing_time_ms.load(Ordering::Relaxed);
        let avg_time = if completed > 0 {
            total_time / completed
        } else {
            0
        };

        PinningWorkerStats {
            jobs_queued: self.stats.jobs_queued.load(Ordering::Relaxed),
            jobs_completed: completed,
            jobs_failed: self.stats.jobs_failed.load(Ordering::Relaxed),
            jobs_dead_lettered: self.stats.jobs_dead_lettered.load(Ordering::Relaxed),
            jobs_in_progress: in_progress as u64,
            total_retries: self.stats.total_retries.load(Ordering::Relaxed),
            queue_size,
            dlq_size,
            average_processing_time_ms: avg_time,
        }
    }

    /// Get dead-lettered jobs
    pub async fn get_dead_letter_queue(&self) -> Vec<PinningJob> {
        let dlq = self.dead_letter_queue.read().await;
        dlq.iter().cloned().collect()
    }

    /// Retry a dead-lettered job
    pub async fn retry_dead_letter(&self, job_id: &str) -> Result<(), String> {
        let mut dlq = self.dead_letter_queue.write().await;

        let position = dlq.iter().position(|j| j.id == job_id);
        if let Some(pos) = position {
            let mut job = dlq.remove(pos).unwrap();
            job.retry_count = 0;
            job.status = PinningJobStatus::Pending;
            job.updated_at = chrono::Utc::now().to_rfc3339();

            drop(dlq);

            self.job_sender.send(job).await.map_err(|e| e.to_string())?;
            info!(job_id = %job_id, "Dead-lettered job requeued for retry");
            Ok(())
        } else {
            Err("Job not found in dead letter queue".to_string())
        }
    }

    /// Start the worker
    pub async fn start(&self) {
        if self.is_running.swap(true, Ordering::SeqCst) {
            warn!("Pinning worker already running");
            return;
        }

        info!(
            workers = self.config.worker_count,
            "Starting pinning worker"
        );

        // Take ownership of the receiver
        let receiver = {
            let mut guard = self.job_receiver.write().await;
            guard.take()
        };

        let Some(mut receiver) = receiver else {
            error!("Pinning worker receiver already taken");
            return;
        };

        // Spawn receiver task to move jobs to priority queue
        let queue = self.queue.clone();
        let shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            while !shutdown.load(Ordering::Relaxed) {
                match receiver.recv().await {
                    Some(job) => {
                        let mut q = queue.write().await;
                        q.push(PrioritizedJob {
                            job,
                            scheduled_at: Instant::now(),
                        });
                    }
                    None => break,
                }
            }
        });

        // Spawn worker tasks
        for worker_id in 0..self.config.worker_count {
            let queue = self.queue.clone();
            let in_progress = self.in_progress.clone();
            let completed = self.completed.clone();
            let dlq = self.dead_letter_queue.clone();
            let providers = self.providers.clone();
            let config = self.config.clone();
            let stats = self.stats.clone();
            let shutdown = self.shutdown.clone();
            let mut shutdown_rx = self.shutdown_sender.subscribe();
            let job_sender = self.job_sender.clone();

            tokio::spawn(async move {
                debug!(worker_id = worker_id, "Worker started");

                loop {
                    // Check for shutdown
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    // Try to get a job from the queue
                    let job = {
                        let mut q = queue.write().await;
                        q.pop().map(|pj| pj.job)
                    };

                    let Some(mut job) = job else {
                        // No jobs available, wait a bit
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_millis(100)) => continue,
                            _ = shutdown_rx.recv() => break,
                        }
                    };

                    // Mark job as in progress
                    job.mark_in_progress();
                    {
                        let mut ip = in_progress.write().await;
                        ip.insert(job.id.clone(), job.clone());
                    }

                    let start_time = Instant::now();
                    let job_id = job.id.clone();
                    let cid = job.cid.clone();

                    debug!(
                        worker_id = worker_id,
                        job_id = %job_id,
                        cid = %cid,
                        provider = %job.provider,
                        "Processing pinning job"
                    );

                    // Get the provider
                    let provider = {
                        let provs = providers.read().await;
                        if job.provider == "all" {
                            // Pin to all providers - get first one for now
                            provs.values().next().cloned()
                        } else {
                            provs.get(&job.provider).cloned()
                        }
                    };

                    let result = match provider {
                        Some(p) => {
                            // Execute with timeout
                            tokio::time::timeout(config.job_timeout, p.pin(&cid)).await
                        }
                        None => {
                            // No provider found - create a timeout error using a zero-duration timeout
                            tokio::time::timeout(Duration::ZERO, async {
                                Err::<(), String>("Provider not found".to_string())
                            })
                            .await
                        }
                    };

                    let processing_time = start_time.elapsed();
                    stats
                        .total_processing_time_ms
                        .fetch_add(processing_time.as_millis() as u64, Ordering::Relaxed);

                    // Remove from in-progress
                    {
                        let mut ip = in_progress.write().await;
                        ip.remove(&job_id);
                    }

                    match result {
                        Ok(Ok(())) => {
                            job.mark_completed();
                            stats.jobs_completed.fetch_add(1, Ordering::Relaxed);

                            // Add to completed queue (bounded)
                            {
                                let mut comp = completed.write().await;
                                if comp.len() >= 1000 {
                                    comp.pop_front();
                                }
                                comp.push_back(job);
                            }

                            info!(
                                worker_id = worker_id,
                                job_id = %job_id,
                                cid = %cid,
                                duration_ms = processing_time.as_millis() as u64,
                                "Pinning job completed"
                            );
                        }
                        Ok(Err(e)) => {
                            job.mark_failed(&e);
                            stats.jobs_failed.fetch_add(1, Ordering::Relaxed);

                            if job.status == PinningJobStatus::DeadLettered {
                                stats.jobs_dead_lettered.fetch_add(1, Ordering::Relaxed);

                                let mut dlq_guard = dlq.write().await;
                                if dlq_guard.len() >= config.dlq_size {
                                    dlq_guard.pop_front();
                                }
                                dlq_guard.push_back(job);

                                warn!(
                                    worker_id = worker_id,
                                    job_id = %job_id,
                                    cid = %cid,
                                    error = %e,
                                    "Pinning job dead-lettered"
                                );
                            } else {
                                stats.total_retries.fetch_add(1, Ordering::Relaxed);

                                // Schedule retry with backoff
                                let backoff = config.backoff_for_retry(job.retry_count);

                                let job_for_retry = job.clone();
                                let sender = job_sender.clone();

                                tokio::spawn(async move {
                                    tokio::time::sleep(backoff).await;
                                    let _ = sender.send(job_for_retry).await;
                                });

                                warn!(
                                    worker_id = worker_id,
                                    job_id = %job_id,
                                    cid = %cid,
                                    retry = job.retry_count,
                                    backoff_ms = backoff.as_millis() as u64,
                                    error = %e,
                                    "Pinning job failed, scheduling retry"
                                );
                            }
                        }
                        Err(_) => {
                            job.mark_failed("Operation timed out");
                            stats.jobs_failed.fetch_add(1, Ordering::Relaxed);

                            if job.can_retry() {
                                stats.total_retries.fetch_add(1, Ordering::Relaxed);
                                let backoff = config.backoff_for_retry(job.retry_count);

                                let job_for_retry = job.clone();
                                let sender = job_sender.clone();

                                tokio::spawn(async move {
                                    tokio::time::sleep(backoff).await;
                                    let _ = sender.send(job_for_retry).await;
                                });
                            } else {
                                stats.jobs_dead_lettered.fetch_add(1, Ordering::Relaxed);

                                let mut dlq_guard = dlq.write().await;
                                if dlq_guard.len() >= config.dlq_size {
                                    dlq_guard.pop_front();
                                }
                                dlq_guard.push_back(job);
                            }

                            warn!(
                                worker_id = worker_id,
                                job_id = %job_id,
                                cid = %cid,
                                "Pinning job timed out"
                            );
                        }
                    }
                }

                debug!(worker_id = worker_id, "Worker stopped");
            });
        }
    }

    /// Shutdown the worker gracefully
    pub async fn shutdown(&self) {
        info!("Shutting down pinning worker");
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = self.shutdown_sender.send(());
        self.is_running.store(false, Ordering::SeqCst);
    }

    /// Check if the worker is running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }
}

/// Simple in-memory pinning provider for testing
pub struct MockPinningProvider {
    name: String,
    should_fail: bool,
    latency: Duration,
}

impl MockPinningProvider {
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

    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = latency;
        self
    }
}

#[async_trait::async_trait]
impl PinningProvider for MockPinningProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn pin(&self, cid: &str) -> Result<(), String> {
        tokio::time::sleep(self.latency).await;

        if self.should_fail {
            Err(format!("Mock pin failure for {}", cid))
        } else {
            Ok(())
        }
    }

    async fn is_pinned(&self, _cid: &str) -> Result<bool, String> {
        Ok(!self.should_fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pinning_job_creation() {
        let job = PinningJob::new("QmTestCid".to_string(), "provider1".to_string());

        assert_eq!(job.cid, "QmTestCid");
        assert_eq!(job.provider, "provider1");
        assert_eq!(job.status, PinningJobStatus::Pending);
        assert_eq!(job.retry_count, 0);
    }

    #[test]
    fn test_pinning_job_priority() {
        let job = PinningJob::new("QmTestCid".to_string(), "provider1".to_string())
            .with_priority(PinPriority::Critical);

        assert_eq!(job.priority, PinPriority::Critical);
    }

    #[test]
    fn test_pinning_job_retry() {
        let mut job =
            PinningJob::new("QmTestCid".to_string(), "provider1".to_string()).with_max_retries(2);

        assert!(job.can_retry());

        job.mark_failed("Error 1");
        assert_eq!(job.retry_count, 1);
        assert!(job.can_retry());

        job.mark_failed("Error 2");
        assert_eq!(job.retry_count, 2);
        assert!(!job.can_retry());
        assert_eq!(job.status, PinningJobStatus::DeadLettered);
    }

    #[test]
    fn test_config_backoff() {
        let config = PinningWorkerConfig {
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            backoff_multiplier: 2.0,
            ..Default::default()
        };

        assert_eq!(config.backoff_for_retry(0), Duration::from_secs(1));
        assert_eq!(config.backoff_for_retry(1), Duration::from_secs(2));
        assert_eq!(config.backoff_for_retry(2), Duration::from_secs(4));
        assert_eq!(config.backoff_for_retry(10), Duration::from_secs(60)); // Capped at max
    }

    #[test]
    fn test_priority_ordering() {
        let critical = PinningJob::new("cid1".to_string(), "p".to_string())
            .with_priority(PinPriority::Critical);
        let normal =
            PinningJob::new("cid2".to_string(), "p".to_string()).with_priority(PinPriority::Normal);
        let low =
            PinningJob::new("cid3".to_string(), "p".to_string()).with_priority(PinPriority::Low);

        let now = Instant::now();
        let pj1 = PrioritizedJob {
            job: critical,
            scheduled_at: now,
        };
        let pj2 = PrioritizedJob {
            job: normal,
            scheduled_at: now,
        };
        let pj3 = PrioritizedJob {
            job: low,
            scheduled_at: now,
        };

        // Critical > Normal > Low
        assert!(pj1 > pj2);
        assert!(pj2 > pj3);
        assert!(pj1 > pj3);
    }

    #[tokio::test]
    async fn test_worker_submit_and_stats() {
        let config = PinningWorkerConfig {
            worker_count: 1,
            max_queue_size: 100,
            ..Default::default()
        };

        let worker = PinningWorker::new(config);
        worker
            .register_provider(MockPinningProvider::new("test"))
            .await;

        let job = PinningJob::new("QmTestCid".to_string(), "test".to_string());
        let result = worker.submit(job).await;

        assert!(result.is_ok());

        let stats = worker.stats().await;
        assert_eq!(stats.jobs_queued, 1);
    }

    #[tokio::test]
    async fn test_worker_get_job_status() {
        let worker = PinningWorker::new(PinningWorkerConfig::default());

        let job = PinningJob::new("QmTestCid".to_string(), "test".to_string());
        let job_id = job.id.clone();

        let _ = worker.submit(job).await;

        let status = worker.get_job_status(&job_id).await;
        assert!(status.is_some());
    }
}
