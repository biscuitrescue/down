use crate::chain::ChainClient;
use crate::ipfs::IpfsClient;
use anyhow::Result;
use std::sync::Arc;

/// Unified application state containing all shared clients and configuration
#[derive(Clone)]
pub struct AppState {
    pub chain_client: Arc<ChainClient>,
    pub ipfs_client: Arc<IpfsClient>,
}

impl AppState {
    /// Create new AppState from environment variables
    pub async fn from_env() -> Result<Self> {
        tracing::info!("Initializing application state from environment");

        let chain_client = ChainClient::from_env().await?;
        tracing::info!("Chain client initialized successfully");

        let ipfs_client = IpfsClient::from_env()?;
        tracing::info!("IPFS client initialized successfully");

        Ok(Self {
            chain_client: Arc::new(chain_client),
            ipfs_client: Arc::new(ipfs_client),
        })
    }
}
