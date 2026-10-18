//! Environment configuration. Every variable is documented in `backend/.env.example`.

use alloy::primitives::{Address, U256};
use anyhow::{bail, Context, Result};
use std::{env, path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
    Test,
}

impl Environment {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "production" | "prod" => Environment::Production,
            "test" => Environment::Test,
            _ => Environment::Development,
        }
    }

    pub fn is_production(&self) -> bool {
        matches!(self, Environment::Production)
    }
}

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub host: String,
    pub environment: Environment,
    pub database_path: PathBuf,
    pub frontend_dir: Option<PathBuf>,
    pub cors_allowed_origins: Vec<String>,
    pub chain: ChainConfig,
    pub ipfs: IpfsConfig,
    pub ipfs_backup: IpfsBackupConfig,
    pub secrets: SecretsConfig,
    pub reconcile_interval: Duration,
    pub ai_auto_reveal: bool,
    pub advisory: AdvisoryConfig,
    pub ranking_prior_strength: f64,
}

#[derive(Clone)]
pub struct ChainConfig {
    pub rpc_url: String,
    pub chain_id: u64,
    pub registry: Address,
    pub provenance_nft: Address,
    pub credential_nft: Address,
    pub deploy_block: u64,
    pub admin_private_key: String,
    pub participant_funding_wei: U256,
    pub confirm_timeout: Duration,
    pub poll_interval: Duration,
    pub drop_after: Duration,
}

#[derive(Debug, Clone)]
pub struct IpfsConfig {
    pub api_url: String,
    pub gateway_url: String,
    /// Full `Authorization` header value (e.g. `Bearer ...`) for hosted Kubo-compatible APIs.
    pub auth_header: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
}

/// Replication of every stored block to additional pinning targets (see `pinning.rs`).
#[derive(Debug, Clone, Default)]
pub struct IpfsBackupConfig {
    /// Extra Kubo-compatible nodes; blocks are pushed to them and read back to verify.
    pub kubo: Vec<IpfsConfig>,
    /// IPFS Pinning Service API endpoint (e.g. Pinata `https://api.pinata.cloud/psa`, Filebase, 4EVERLAND).
    pub pinning_service: Option<PinningServiceConfig>,
    /// Attempts per (cid, target) before the job is marked `failed`.
    pub max_attempts: u32,
}

#[derive(Clone)]
pub struct PinningServiceConfig {
    pub url: String,
    pub token: String,
    pub timeout: Duration,
}

impl std::fmt::Debug for PinningServiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinningServiceConfig")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct SecretsConfig {
    pub admin_api_token: String,
    /// Encrypts custodial participant wallet keys at rest (SQLite).
    pub key_encryption_key: [u8; 32],
    /// Encrypts personal / confidential records before they are uploaded to IPFS.
    pub pii_encryption_key: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct AdvisoryConfig {
    /// Base URL of a local Ollama-compatible server, e.g. `http://127.0.0.1:11434`. Unset = disabled.
    pub base_url: Option<String>,
    pub model: String,
    pub languages: Vec<String>,
    pub timeout: Duration,
}

fn var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn required(name: &str) -> Result<String> {
    var(name).with_context(|| format!("{name} environment variable is required"))
}

fn parse_or<T: std::str::FromStr>(name: &str, default: T) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    match var(name) {
        Some(v) => v
            .parse::<T>()
            .map_err(|e| anyhow::anyhow!("{name} is invalid: {e}")),
        None => Ok(default),
    }
}

fn parse_address(name: &str, value: &str) -> Result<Address> {
    let addr: Address = value
        .parse()
        .with_context(|| format!("{name} is not a valid address"))?;
    if addr == Address::ZERO {
        bail!("{name} must not be the zero address");
    }
    Ok(addr)
}

pub fn parse_key32(name: &str, value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value.trim_start_matches("0x"))
        .with_context(|| format!("{name} must be 32 bytes of hex"))?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("{name} must be exactly 32 bytes (64 hex chars)"))
}

/// Addresses written by `contracts/script/Deploy.s.sol`.
#[derive(serde::Deserialize)]
struct DeploymentFile {
    #[serde(rename = "chainId")]
    chain_id: u64,
    #[serde(rename = "OilseedValueChain")]
    registry: Address,
    #[serde(rename = "BatchProvenanceNFT")]
    provenance: Address,
    #[serde(rename = "FarmerCredential")]
    credential: Address,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let port = parse_or("PORT", 3000u16)?;
        let host = var("HOST").unwrap_or_else(|| "127.0.0.1".to_string());
        let environment = var("ENVIRONMENT")
            .map(|e| Environment::from_str(&e))
            .unwrap_or(Environment::Development);

        let chain_id: u64 = required("CHAIN_ID")?
            .parse()
            .context("CHAIN_ID must be a valid u64")?;

        // Addresses: explicit env vars win; otherwise read the Foundry deployment file.
        let deployment = match var("DEPLOYMENT_FILE") {
            Some(path) => {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("cannot read DEPLOYMENT_FILE {path}"))?;
                let d: DeploymentFile = serde_json::from_str(&text)
                    .with_context(|| format!("DEPLOYMENT_FILE {path} is malformed"))?;
                if d.chain_id != chain_id {
                    bail!(
                        "DEPLOYMENT_FILE is for chain {} but CHAIN_ID is {chain_id}",
                        d.chain_id
                    );
                }
                Some(d)
            }
            None => None,
        };
        let pick =
            |name: &str, alias: Option<&str>, from_file: Option<Address>| -> Result<Address> {
                if let Some(v) = var(name).or_else(|| alias.and_then(var)) {
                    return parse_address(name, &v);
                }
                from_file.with_context(|| format!("{name} (or DEPLOYMENT_FILE) is required"))
            };
        let registry = pick(
            "REGISTRY_ADDRESS",
            Some("CONTRACT_ADDRESS"),
            deployment.as_ref().map(|d| d.registry),
        )?;
        let provenance_nft = pick(
            "PROVENANCE_NFT_ADDRESS",
            None,
            deployment.as_ref().map(|d| d.provenance),
        )?;
        let credential_nft = pick(
            "CREDENTIAL_NFT_ADDRESS",
            None,
            deployment.as_ref().map(|d| d.credential),
        )?;

        let admin_private_key = var("ADMIN_PRIVATE_KEY")
            .or_else(|| var("PRIVATE_KEY"))
            .context("ADMIN_PRIVATE_KEY environment variable is required")?;

        let chain = ChainConfig {
            rpc_url: required("RPC_URL")?,
            chain_id,
            registry,
            provenance_nft,
            credential_nft,
            deploy_block: parse_or("DEPLOY_BLOCK", 0u64)?,
            admin_private_key,
            participant_funding_wei: parse_or("PARTICIPANT_FUNDING_WEI", U256::ZERO)?,
            confirm_timeout: Duration::from_secs(parse_or("TX_CONFIRM_TIMEOUT_SECS", 30u64)?),
            poll_interval: Duration::from_millis(parse_or("TX_POLL_INTERVAL_MS", 500u64)?),
            drop_after: Duration::from_secs(parse_or("TX_DROP_AFTER_SECS", 600u64)?),
        };

        // Accept the legacy Infura-style project id/secret as HTTP basic auth.
        let auth_header = var("IPFS_AUTH_HEADER").or_else(|| {
            match (var("IPFS_PROJECT_ID"), var("IPFS_PROJECT_SECRET")) {
                (Some(id), Some(secret)) => {
                    use base64::Engine;
                    Some(format!(
                        "Basic {}",
                        base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"))
                    ))
                }
                _ => None,
            }
        });
        let ipfs = IpfsConfig {
            api_url: required("IPFS_API_URL")?.trim_end_matches('/').to_string(),
            gateway_url: var("IPFS_GATEWAY_URL")
                .unwrap_or_else(|| "https://ipfs.io/ipfs/".to_string()),
            auth_header,
            timeout: Duration::from_secs(parse_or("IPFS_TIMEOUT_SECS", 15u64)?),
            max_retries: parse_or("IPFS_MAX_RETRIES", 3u32)?,
        };

        let backup_auth = var("IPFS_BACKUP_KUBO_AUTH_HEADER");
        let ipfs_backup = IpfsBackupConfig {
            kubo: var("IPFS_BACKUP_KUBO_URLS")
                .map(|v| {
                    v.split(',')
                        .map(|u| u.trim().trim_end_matches('/').to_string())
                        .filter(|u| !u.is_empty())
                        .map(|api_url| IpfsConfig {
                            api_url,
                            gateway_url: ipfs.gateway_url.clone(),
                            auth_header: backup_auth.clone(),
                            timeout: ipfs.timeout,
                            max_retries: 1,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            pinning_service: match (
                var("IPFS_PINNING_SERVICE_URL"),
                var("IPFS_PINNING_SERVICE_TOKEN"),
            ) {
                (Some(url), Some(token)) => Some(PinningServiceConfig {
                    url: url.trim_end_matches('/').to_string(),
                    token,
                    timeout: ipfs.timeout,
                }),
                (None, None) => None,
                _ => bail!(
                    "set both IPFS_PINNING_SERVICE_URL and IPFS_PINNING_SERVICE_TOKEN, or neither"
                ),
            },
            max_attempts: parse_or("IPFS_BACKUP_MAX_ATTEMPTS", 8u32)?,
        };

        let admin_api_token = required("ADMIN_API_TOKEN")?;
        if admin_api_token.len() < 32 {
            bail!("ADMIN_API_TOKEN must be at least 32 characters");
        }
        let secrets = SecretsConfig {
            admin_api_token,
            key_encryption_key: parse_key32(
                "KEY_ENCRYPTION_KEY",
                &required("KEY_ENCRYPTION_KEY")?,
            )?,
            pii_encryption_key: parse_key32(
                "PII_ENCRYPTION_KEY",
                &required("PII_ENCRYPTION_KEY")?,
            )?,
        };
        if secrets.key_encryption_key == secrets.pii_encryption_key {
            bail!("KEY_ENCRYPTION_KEY and PII_ENCRYPTION_KEY must differ");
        }

        let advisory = AdvisoryConfig {
            base_url: var("ADVISORY_BASE_URL").map(|u| u.trim_end_matches('/').to_string()),
            model: var("ADVISORY_MODEL").unwrap_or_else(|| "gemma3n:e2b".to_string()),
            languages: var("ADVISORY_LANGUAGES")
                .unwrap_or_else(|| "en".to_string())
                .split(',')
                .map(|l| l.trim().to_lowercase())
                .filter(|l| !l.is_empty())
                .collect(),
            timeout: Duration::from_secs(parse_or("ADVISORY_TIMEOUT_SECS", 120u64)?),
        };

        Ok(Config {
            port,
            host,
            environment,
            database_path: var("DATABASE_PATH")
                .unwrap_or_else(|| "oilschain.db".to_string())
                .into(),
            frontend_dir: var("FRONTEND_DIR").map(PathBuf::from),
            cors_allowed_origins: var("CORS_ALLOWED_ORIGINS")
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            chain,
            ipfs,
            ipfs_backup,
            secrets,
            reconcile_interval: Duration::from_secs(parse_or("RECONCILE_INTERVAL_SECS", 15u64)?),
            ai_auto_reveal: parse_or("AI_AUTO_REVEAL", true)?,
            advisory,
            ranking_prior_strength: parse_or("RANKING_PRIOR_STRENGTH", 4.0f64)?,
        })
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}
