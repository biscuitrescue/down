//! Services Module
//!
//! Background services for production IPFS infrastructure:
//! - Pinning worker for async content pinning
//! - Health monitor for provider health checks
//! - CID auditor for content availability verification

pub mod health_monitor;
pub mod pinning_worker;

pub use health_monitor::{HealthMonitor, HealthMonitorConfig, HealthReport};
pub use pinning_worker::{PinningWorker, PinningWorkerConfig, PinningJobStatus};
