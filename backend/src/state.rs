use crate::chain::ChainClient;
use crate::config::Config;
use crate::db::Db;
use crate::ipfs::IpfsClient;
use anyhow::Result;
use std::ops::Deref;
use std::sync::Arc;

/// Shared clients and configuration.
pub struct AppStateInner {
    pub config: Config,
    pub db: Db,
    pub chain_client: ChainClient,
    pub ipfs_client: IpfsClient,
    /// Backup pinning targets (empty = single-homed; see pinning.rs).
    pub backups: Vec<crate::pinning::BackupTarget>,
    /// Plain HTTP client for the optional local advisory model.
    pub http: reqwest::Client,
}

/// Unified application state containing all shared clients and configuration
#[derive(Clone)]
pub struct AppState(Arc<AppStateInner>);

impl Deref for AppState {
    type Target = AppStateInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AppState {
    /// Connect to the chain (validating chain id and deployments), the IPFS API and the database.
    pub async fn initialize(config: Config) -> Result<Self> {
        tracing::info!("Initializing application state");

        let chain_client = ChainClient::connect(&config.chain).await?;
        tracing::info!(
            chain_id = chain_client.chain_id,
            registry = %chain_client.registry_address,
            admin = %chain_client.admin_address(),
            "Chain client initialized"
        );

        let ipfs_client = IpfsClient::new(config.ipfs.clone())?;
        match ipfs_client.version().await {
            Ok(v) => tracing::info!(version = %v, "IPFS API reachable"),
            // Not fatal: reads and verification report IPFS unavailability explicitly per record.
            Err(e) => tracing::warn!(error = %e, "IPFS API not reachable at startup"),
        }

        let db = Db::open(&config.database_path)?;
        tracing::info!(path = %config.database_path.display(), "Database ready");

        Ok(Self::from_parts(config, db, chain_client, ipfs_client))
    }

    pub fn from_parts(
        config: Config,
        db: Db,
        chain_client: ChainClient,
        ipfs_client: IpfsClient,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(config.advisory.timeout)
            .build()
            .expect("HTTP client builds");
        let backups = crate::pinning::build_targets(&config.ipfs_backup)
            .expect("backup pinning clients build");
        for t in &backups {
            tracing::info!(target = t.name(), "backup pinning target");
        }
        Self(Arc::new(AppStateInner {
            config,
            db,
            chain_client,
            ipfs_client,
            backups,
            http,
        }))
    }

    pub fn domain(&self) -> crate::canonical::Domain {
        crate::canonical::Domain {
            chain_id: self.chain_client.chain_id,
            registry: self.chain_client.registry_address,
        }
    }
}
