pub mod chain;
pub mod config;
pub mod error;
pub mod indexer;
pub mod ipfs;
pub mod ipfs_client;
pub mod ipfs_providers;
pub mod routes;
pub mod state;
pub mod supply_chain_handlers;
pub mod workflows;

pub mod ai;
pub mod privacy;
pub mod schema;
pub mod services;
pub mod utils;

pub use schema::{SchemaRegistry, SchemaValidator, SupplyChainStage, ValidationResult};
pub use privacy::{PrivacyProcessor, PrivacyLevel};
pub use ipfs_providers::{IpfsProvider, IpfsRouter, Cid};
pub use services::{HealthMonitor, PinningWorker};
pub use ai::{AiAnalysisResult, IntegrityVerifier, ModelRegistry};
pub use utils::{compress_data, decompress_data};
