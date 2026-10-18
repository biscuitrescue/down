//! EVM client for the OilsChain contracts. Bindings are generated from `backend/abi/*.json`, which
//! `scripts/export-artifacts.sh` exports from the Foundry build, so the ABI cannot drift from the contracts.
//!
//! A submitted transaction is not a confirmed one: `send` returns only the hash, callers persist it, and
//! confirmation is observed separately (`receipt`, `wait_for_receipt`).

use crate::config::ChainConfig;
use alloy::{
    eips::BlockNumberOrTag,
    network::{EthereumWallet, TransactionBuilder},
    primitives::{Address, B256, U256},
    providers::{DynProvider, Provider, ProviderBuilder},
    rpc::types::{Filter, Log, TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolEvent,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

sol!(
    #[sol(rpc, all_derives)]
    OilseedValueChain,
    "abi/OilseedValueChain.json"
);
sol!(
    #[sol(rpc, all_derives)]
    BatchProvenanceNFT,
    "abi/BatchProvenanceNFT.json"
);
sol!(
    #[sol(rpc, all_derives)]
    FarmerCredential,
    "abi/FarmerCredential.json"
);

/// Role bits; must equal the contract constants (checked at startup and in tests).
pub mod roles {
    pub const ADMIN: u64 = 1 << 0;
    pub const FARMER: u64 = 1 << 1;
    pub const FPO: u64 = 1 << 2;
    pub const WAREHOUSE: u64 = 1 << 3;
    pub const LOGISTICS: u64 = 1 << 4;
    pub const PROCESSOR: u64 = 1 << 5;
    pub const PACKAGER: u64 = 1 << 6;
    pub const AI_ORACLE: u64 = 1 << 7;
    pub const RETAILER: u64 = 1 << 8;
    pub const ALL: u64 = (1 << 9) - 1;
    pub const CUSTODIAN: u64 = FPO | WAREHOUSE | LOGISTICS | PROCESSOR | PACKAGER | RETAILER;

    pub const NAMES: [(&str, u64); 9] = [
        ("admin", ADMIN),
        ("farmer", FARMER),
        ("fpo", FPO),
        ("warehouse", WAREHOUSE),
        ("logistics", LOGISTICS),
        ("processor", PROCESSOR),
        ("packager", PACKAGER),
        ("ai_oracle", AI_ORACLE),
        ("retailer", RETAILER),
    ];

    pub fn parse(names: &[String]) -> Result<u64, String> {
        let mut mask = 0;
        for n in names {
            let bit = NAMES
                .iter()
                .find(|(name, _)| *name == n.as_str())
                .map(|(_, b)| *b)
                .ok_or_else(|| format!("unknown role `{n}`"))?;
            mask |= bit;
        }
        Ok(mask)
    }

    pub fn names(mask: u64) -> Vec<&'static str> {
        NAMES
            .iter()
            .filter(|(_, b)| mask & b != 0)
            .map(|(n, _)| *n)
            .collect()
    }
}

/// Lifecycle stage names, indexed by the contract's `Stage` enum.
pub const STAGES: [&str; 7] = [
    "none",
    "procured",
    "stored",
    "in_transit",
    "processed",
    "consumed",
    "packaged",
];

/// A contract write, persisted in the operation journal so it can be resent after a crash or a dropped tx.
/// Hex strings keep the journal format independent of library serialization details.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "fn", rename_all = "snake_case")]
pub enum ChainAction {
    GrantRole {
        account: String,
        role: u64,
    },
    Fund {
        to: String,
        wei: String,
    },
    RegisterFarmer {
        farmer_id: String,
        content_hash: String,
        cid: String,
    },
    RecordProcurement {
        batch_id: String,
        farmer_id: String,
        content_hash: String,
        cid: String,
    },
    TransferCustody {
        batch_id: String,
        to: String,
        content_hash: String,
        cid: String,
    },
    RecordWarehouseInspection {
        batch_id: String,
        content_hash: String,
        cid: String,
    },
    RecordLogistics {
        batch_id: String,
        content_hash: String,
        cid: String,
    },
    ProcessBatch {
        input_batch_id: String,
        output_batch_ids: Vec<String>,
        content_hash: String,
        cid: String,
    },
    CreateSku {
        sku_id: String,
        batch_id: String,
        units_root: String,
        unit_count: u32,
        content_hash: String,
        cid: String,
    },
    ReportFraud {
        batch_id: String,
        content_hash: String,
        cid: String,
    },
    ResolveFraud {
        report_digest: String,
        upheld: bool,
        content_hash: String,
        cid: String,
    },
    CommitAi {
        batch_id: String,
        commit_hash: String,
    },
    RevealAi {
        batch_id: String,
        content_hash: String,
        salt: String,
        cid: String,
    },
    MintProvenance {
        record_digest: String,
        metadata_cid: String,
    },
    IssueCredential {
        to: String,
        farmer_id: String,
        metadata_cid: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    /// The contract (or gas estimation) reverted with a known custom error. Not retryable as-is.
    #[error("{0}")]
    Rejected(String),
    /// RPC/transport problem; safe to retry with the same idempotency key.
    #[error("{0}")]
    Transport(String),
    #[error("invalid action argument: {0}")]
    Invalid(String),
}

pub fn parse_b256(s: &str) -> Result<B256, SendError> {
    s.parse()
        .map_err(|_| SendError::Invalid(format!("not a 32-byte hex value: {s}")))
}

pub fn parse_addr(s: &str) -> Result<Address, SendError> {
    s.parse()
        .map_err(|_| SendError::Invalid(format!("not an address: {s}")))
}

pub fn hex32(v: B256) -> String {
    format!("{v:#x}")
}

pub fn hexaddr(a: Address) -> String {
    format!("{a:#x}")
}

/// Decoded `RecordAnchored` event with its location.
#[derive(Debug, Clone, Serialize)]
pub struct AnchoredLog {
    pub subject_id: B256,
    pub record_type: u8,
    pub record_digest: B256,
    pub content_hash: B256,
    pub submitter: Address,
    pub head: B256,
    pub cid: String,
    pub tx_hash: Option<B256>,
    pub block_number: Option<u64>,
    pub log_index: Option<u64>,
}

fn decode_anchored(log: &Log) -> Option<AnchoredLog> {
    let decoded = log.log_decode::<OilseedValueChain::RecordAnchored>().ok()?;
    let e = decoded.inner.data;
    Some(AnchoredLog {
        subject_id: e.subjectId,
        record_type: e.recordType,
        record_digest: e.recordDigest,
        content_hash: e.contentHash,
        submitter: e.submitter,
        head: e.head,
        cid: e.cid,
        tx_hash: log.transaction_hash,
        block_number: log.block_number,
        log_index: log.log_index,
    })
}

pub struct ChainClient {
    read: DynProvider,
    rpc_url: reqwest::Url,
    pub chain_id: u64,
    pub registry_address: Address,
    pub provenance_address: Address,
    pub credential_address: Address,
    pub deploy_block: u64,
    admin_signer: PrivateKeySigner,
    pub funding_wei: U256,
    pub confirm_timeout: Duration,
    pub poll_interval: Duration,
    pub drop_after: Duration,
    /// Serializes transaction submission so nonces are assigned in order.
    send_lock: tokio::sync::Mutex<()>,
}

impl ChainClient {
    pub async fn connect(config: &ChainConfig) -> Result<Self> {
        let rpc_url: reqwest::Url = config.rpc_url.parse().context("Invalid RPC URL")?;
        let read = ProviderBuilder::new()
            .connect_http(rpc_url.clone())
            .erased();

        let remote_chain_id = read
            .get_chain_id()
            .await
            .context("RPC unreachable (eth_chainId)")?;
        if remote_chain_id != config.chain_id {
            bail!(
                "RPC chain id {remote_chain_id} does not match CHAIN_ID {}",
                config.chain_id
            );
        }
        for (name, addr) in [
            ("REGISTRY_ADDRESS", config.registry),
            ("PROVENANCE_NFT_ADDRESS", config.provenance_nft),
            ("CREDENTIAL_NFT_ADDRESS", config.credential_nft),
        ] {
            let code = read.get_code_at(addr).await.context("eth_getCode failed")?;
            if code.is_empty() {
                bail!("{name} {addr} has no contract code on chain {remote_chain_id}");
            }
        }

        let admin_signer: PrivateKeySigner = config
            .admin_private_key
            .parse()
            .context("ADMIN_PRIVATE_KEY is not a valid private key")?;

        let client = Self {
            read,
            rpc_url,
            chain_id: config.chain_id,
            registry_address: config.registry,
            provenance_address: config.provenance_nft,
            credential_address: config.credential_nft,
            deploy_block: config.deploy_block,
            admin_signer,
            funding_wei: config.participant_funding_wei,
            confirm_timeout: config.confirm_timeout,
            poll_interval: config.poll_interval,
            drop_after: config.drop_after,
            send_lock: tokio::sync::Mutex::new(()),
        };
        client.check_deployment().await?;
        Ok(client)
    }

    /// Validate that the NFTs point at the registry, role bits match, and the admin key really is admin.
    async fn check_deployment(&self) -> Result<()> {
        let reg = self.registry();
        let prov = BatchProvenanceNFT::new(self.provenance_address, self.read.clone());
        let cred = FarmerCredential::new(self.credential_address, self.read.clone());
        if prov.registry().call().await? != self.registry_address {
            bail!("PROVENANCE_NFT_ADDRESS is bound to a different registry");
        }
        if cred.registry().call().await? != self.registry_address {
            bail!("CREDENTIAL_NFT_ADDRESS is bound to a different registry");
        }
        if reg.ALL_ROLES().call().await? != U256::from(roles::ALL)
            || reg.CUSTODIAN_ROLES().call().await? != U256::from(roles::CUSTODIAN)
        {
            bail!("contract role bits differ from the backend's; redeploy or update chain::roles");
        }
        let admin_roles = reg.getRoles(self.admin_address()).call().await?;
        if admin_roles & U256::from(roles::ADMIN) == U256::ZERO {
            bail!(
                "ADMIN_PRIVATE_KEY ({}) does not hold ROLE_ADMIN",
                self.admin_address()
            );
        }
        Ok(())
    }

    pub fn admin_address(&self) -> Address {
        self.admin_signer.address()
    }

    pub fn admin_signer(&self) -> PrivateKeySigner {
        self.admin_signer.clone()
    }

    pub fn registry(&self) -> OilseedValueChain::OilseedValueChainInstance<DynProvider> {
        OilseedValueChain::new(self.registry_address, self.read.clone())
    }

    pub fn provenance(&self) -> BatchProvenanceNFT::BatchProvenanceNFTInstance<DynProvider> {
        BatchProvenanceNFT::new(self.provenance_address, self.read.clone())
    }

    pub fn credential(&self) -> FarmerCredential::FarmerCredentialInstance<DynProvider> {
        FarmerCredential::new(self.credential_address, self.read.clone())
    }

    pub fn provider(&self) -> &DynProvider {
        &self.read
    }

    fn signing_provider(&self, signer: PrivateKeySigner) -> DynProvider {
        ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .connect_http(self.rpc_url.clone())
            .erased()
    }

    /// Submit `action` signed by `signer`. Returns the transaction hash as soon as the node accepts it.
    pub async fn send(
        &self,
        signer: PrivateKeySigner,
        action: &ChainAction,
    ) -> Result<B256, SendError> {
        let _guard = self.send_lock.lock().await;
        let p = self.signing_provider(signer);
        let reg = OilseedValueChain::new(self.registry_address, p.clone());
        let prov = BatchProvenanceNFT::new(self.provenance_address, p.clone());
        let cred = FarmerCredential::new(self.credential_address, p.clone());

        macro_rules! submit {
            ($call:expr) => {
                $call
                    .send()
                    .await
                    .map(|pending| *pending.tx_hash())
                    .map_err(map_contract_error)
            };
        }

        match action {
            ChainAction::GrantRole { account, role } => {
                submit!(reg.grantRole(parse_addr(account)?, U256::from(*role)))
            }
            ChainAction::Fund { to, wei } => {
                let value: U256 = wei.parse().map_err(|_| SendError::Invalid("wei".into()))?;
                let tx = TransactionRequest::default()
                    .with_to(parse_addr(to)?)
                    .with_value(value);
                p.send_transaction(tx)
                    .await
                    .map(|pending| *pending.tx_hash())
                    .map_err(|e| SendError::Transport(e.to_string()))
            }
            ChainAction::RegisterFarmer {
                farmer_id,
                content_hash,
                cid,
            } => submit!(reg.registerFarmer(
                parse_b256(farmer_id)?,
                parse_b256(content_hash)?,
                cid.clone()
            )),
            ChainAction::RecordProcurement {
                batch_id,
                farmer_id,
                content_hash,
                cid,
            } => {
                submit!(reg.recordProcurement(
                    parse_b256(batch_id)?,
                    parse_b256(farmer_id)?,
                    parse_b256(content_hash)?,
                    cid.clone()
                ))
            }
            ChainAction::TransferCustody {
                batch_id,
                to,
                content_hash,
                cid,
            } => submit!(reg.transferCustody(
                parse_b256(batch_id)?,
                parse_addr(to)?,
                parse_b256(content_hash)?,
                cid.clone()
            )),
            ChainAction::RecordWarehouseInspection {
                batch_id,
                content_hash,
                cid,
            } => submit!(reg.recordWarehouseInspection(
                parse_b256(batch_id)?,
                parse_b256(content_hash)?,
                cid.clone()
            )),
            ChainAction::RecordLogistics {
                batch_id,
                content_hash,
                cid,
            } => {
                submit!(reg.recordLogistics(
                    parse_b256(batch_id)?,
                    parse_b256(content_hash)?,
                    cid.clone()
                ))
            }
            ChainAction::ProcessBatch {
                input_batch_id,
                output_batch_ids,
                content_hash,
                cid,
            } => {
                let outs = output_batch_ids
                    .iter()
                    .map(|o| parse_b256(o))
                    .collect::<Result<Vec<_>, _>>()?;
                submit!(reg.processBatch(
                    parse_b256(input_batch_id)?,
                    outs,
                    parse_b256(content_hash)?,
                    cid.clone()
                ))
            }
            ChainAction::CreateSku {
                sku_id,
                batch_id,
                units_root,
                unit_count,
                content_hash,
                cid,
            } => {
                submit!(reg.createSku(
                    parse_b256(sku_id)?,
                    parse_b256(batch_id)?,
                    parse_b256(units_root)?,
                    *unit_count,
                    parse_b256(content_hash)?,
                    cid.clone()
                ))
            }
            ChainAction::ReportFraud {
                batch_id,
                content_hash,
                cid,
            } => {
                submit!(reg.reportFraud(
                    parse_b256(batch_id)?,
                    parse_b256(content_hash)?,
                    cid.clone()
                ))
            }
            ChainAction::ResolveFraud {
                report_digest,
                upheld,
                content_hash,
                cid,
            } => submit!(reg.resolveFraud(
                parse_b256(report_digest)?,
                *upheld,
                parse_b256(content_hash)?,
                cid.clone()
            )),
            ChainAction::CommitAi {
                batch_id,
                commit_hash,
            } => {
                submit!(reg.commitAIAssessment(parse_b256(batch_id)?, parse_b256(commit_hash)?))
            }
            ChainAction::RevealAi {
                batch_id,
                content_hash,
                salt,
                cid,
            } => submit!(reg.revealAIAssessment(
                parse_b256(batch_id)?,
                parse_b256(content_hash)?,
                parse_b256(salt)?,
                cid.clone()
            )),
            ChainAction::MintProvenance {
                record_digest,
                metadata_cid,
            } => {
                submit!(prov.mint(parse_b256(record_digest)?, metadata_cid.clone()))
            }
            ChainAction::IssueCredential {
                to,
                farmer_id,
                metadata_cid,
            } => {
                submit!(cred.issue(
                    parse_addr(to)?,
                    parse_b256(farmer_id)?,
                    metadata_cid.clone()
                ))
            }
        }
    }

    pub async fn receipt(&self, tx_hash: B256) -> Result<Option<TransactionReceipt>> {
        Ok(self.read.get_transaction_receipt(tx_hash).await?)
    }

    /// Poll for a receipt until `timeout`. `Ok(None)` means still pending, not failed.
    pub async fn wait_for_receipt(
        &self,
        tx_hash: B256,
        timeout: Duration,
    ) -> Result<Option<TransactionReceipt>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(r) = self.receipt(tx_hash).await? {
                return Ok(Some(r));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    /// Whether the node still knows the transaction (pending or mined).
    pub async fn transaction_known(&self, tx_hash: B256) -> Result<bool> {
        Ok(self.read.get_transaction_by_hash(tx_hash).await?.is_some())
    }

    pub async fn latest_timestamp(&self) -> Result<u64> {
        let block = self
            .read
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await?
            .context("latest block missing")?;
        Ok(block.header.timestamp)
    }

    pub async fn block_timestamp(&self, number: u64) -> Result<Option<u64>> {
        Ok(self
            .read
            .get_block_by_number(BlockNumberOrTag::Number(number))
            .await?
            .map(|b| b.header.timestamp))
    }

    /// All `RecordAnchored` events for a subject, in chain order. This is the source of truth for timelines.
    pub async fn record_logs(&self, subject: B256) -> Result<Vec<AnchoredLog>> {
        let filter = Filter::new()
            .address(self.registry_address)
            .event_signature(OilseedValueChain::RecordAnchored::SIGNATURE_HASH)
            .topic1(subject)
            .from_block(self.deploy_block);
        let mut logs: Vec<AnchoredLog> = self
            .read
            .get_logs(&filter)
            .await?
            .iter()
            .filter_map(decode_anchored)
            .collect();
        logs.sort_by_key(|l| (l.block_number, l.log_index));
        Ok(logs)
    }

    /// Locate the `RecordAnchored` event for a digest (used to recover a lost transaction hash).
    pub async fn anchor_log(&self, digest: B256) -> Result<Option<AnchoredLog>> {
        let filter = Filter::new()
            .address(self.registry_address)
            .event_signature(OilseedValueChain::RecordAnchored::SIGNATURE_HASH)
            .topic3(digest)
            .from_block(self.deploy_block);
        Ok(self
            .read
            .get_logs(&filter)
            .await?
            .iter()
            .find_map(decode_anchored))
    }

    pub fn anchored_in_receipt(&self, receipt: &TransactionReceipt) -> Vec<AnchoredLog> {
        receipt
            .inner
            .logs()
            .iter()
            .filter(|l| l.address() == self.registry_address)
            .filter_map(decode_anchored)
            .collect()
    }

    pub fn minted_provenance_in_receipt(
        &self,
        receipt: &TransactionReceipt,
    ) -> Option<(u64, Address)> {
        receipt.inner.logs().iter().find_map(|l| {
            if l.address() != self.provenance_address {
                return None;
            }
            let e = l
                .log_decode::<BatchProvenanceNFT::ProvenanceMinted>()
                .ok()?
                .inner
                .data;
            Some((e.tokenId.try_into().ok()?, e.to))
        })
    }

    pub fn issued_credential_in_receipt(
        &self,
        receipt: &TransactionReceipt,
    ) -> Option<(u64, Address)> {
        receipt.inner.logs().iter().find_map(|l| {
            if l.address() != self.credential_address {
                return None;
            }
            let e = l
                .log_decode::<FarmerCredential::CredentialIssued>()
                .ok()?
                .inner
                .data;
            Some((e.tokenId.try_into().ok()?, e.to))
        })
    }
}

/// Turn a contract call error into `Rejected(<custom error name>)` when the revert data decodes.
fn map_contract_error(e: alloy::contract::Error) -> SendError {
    fn name<T: std::fmt::Debug>(v: T) -> String {
        let s = format!("{v:?}");
        s.split(['(', ' ', '{']).next().unwrap_or("").to_string()
    }
    if let Some(err) = e.as_decoded_interface_error::<OilseedValueChain::OilseedValueChainErrors>()
    {
        return SendError::Rejected(name(err));
    }
    if let Some(err) =
        e.as_decoded_interface_error::<BatchProvenanceNFT::BatchProvenanceNFTErrors>()
    {
        return SendError::Rejected(name(err));
    }
    if let Some(err) = e.as_decoded_interface_error::<FarmerCredential::FarmerCredentialErrors>() {
        return SendError::Rejected(name(err));
    }
    if e.as_revert_data().is_some() {
        return SendError::Rejected("execution reverted".into());
    }
    SendError::Transport(e.to_string())
}

pub fn generate_id() -> B256 {
    B256::from(rand::random::<[u8; 32]>())
}
