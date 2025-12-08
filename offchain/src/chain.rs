use alloy::{
    network::EthereumWallet,
    primitives::{Address, FixedBytes},
    providers::ProviderBuilder,
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
    sol,
    transports::http::{Client, Http},
};
use anyhow::{Context, Result};
use std::env;

// Type alias for the provider with all recommended fillers + wallet
type AppProvider = alloy::providers::fillers::FillProvider<
    alloy::providers::fillers::JoinFill<
        alloy::providers::fillers::JoinFill<
            alloy::providers::Identity,
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::GasFiller,
                alloy::providers::fillers::JoinFill<
                    alloy::providers::fillers::BlobGasFiller,
                    alloy::providers::fillers::JoinFill<
                        alloy::providers::fillers::NonceFiller,
                        alloy::providers::fillers::ChainIdFiller,
                    >,
                >,
            >,
        >,
        alloy::providers::fillers::WalletFiller<EthereumWallet>,
    >,
    alloy::providers::RootProvider<Http<Client>>,
    Http<Client>,
    alloy::network::Ethereum,
>;

// Contract definition matching nyx.sol
sol! {
    #[sol(rpc)]
    contract OilseedValueChain {
        event RoleGranted(address indexed account, uint256 role);
        event RoleRevoked(address indexed account, uint256 role);

        event FarmerRegistered(
            bytes32 indexed farmerDID,
            bytes32 indexed cropIDHash,
            uint64 timestamp,
            string metadataCID
        );

        event OwnershipTransfer(
            bytes32 indexed batchHash,
            bytes32 indexed fromDID,
            address indexed toAddress,
            uint64 timestamp,
            uint8 transferType,
            string metadataCID
        );

        event WarehouseStateUpdated(
            bytes32 indexed warehouseId,
            bytes32 stateHash,
            uint64 timestamp,
            string metadataCID
        );

        event LogisticsMilestone(
            bytes32 indexed shipmentId,
            bytes32 locationHash,
            uint64 timestamp,
            bool isDelivered,
            string metadataCID
        );

        event BatchProcessed(
            bytes32 indexed inputBatchHash,
            bytes32 transformHash,
            bytes32[] outputBatchHashes,
            uint64 timestamp,
            string metadataCID
        );

        event SKUPackaged(
            bytes32 indexed skuId,
            bytes32 indexed parentBatchHash,
            bytes32 merkleRoot,
            uint64 timestamp,
            string metadataCID
        );

        event FraudDetected(
            bytes32 indexed skuId,
            address indexed reporter,
            bytes32 evidenceHash,
            uint64 timestamp,
            string evidenceCID
        );

        event AIScoreCommitted(
            bytes32 indexed batchHash,
            bytes32 commitHash,
            uint64 timestamp
        );

        event AIScoreRevealed(
            bytes32 indexed batchHash,
            bytes32 revealHash,
            uint64 timestamp,
            string metadataCID
        );

        struct FarmerRecord {
            bytes32 cropIDHash;
            uint64 registeredAt;
        }

        struct WarehouseState {
            bytes32 stateHash;
            uint64 lastUpdated;
        }

        struct PackageRecord {
            bytes32 parentBatchHash;
            bytes32 merkleRoot;
            uint64 packagedAt;
        }

        struct AIScore {
            bytes32 commitHash;
            bytes32 revealHash;
            uint64 committedAt;
            uint64 revealedAt;
        }

        function grantRole(address account, uint256 role) external;
        function revokeRole(address account, uint256 role) external;
        function hasRole(address account, uint256 role) external view returns (bool);
        function getRoles(address account) external view returns (uint256);

        // Stage 1: Farmer Registration
        function registerFarmer(bytes32 farmerDID, bytes32 cropIDHash, string calldata metadataCID) external;
        function farmers(bytes32 farmerDID) external view returns (FarmerRecord memory);

        // Stage 2: FPO Verification
        function fpoPurchase(bytes32 batchHash, bytes32 farmerDID, string calldata metadataCID) external;

        // Stage 3: Warehouse Storage
        function updateWarehouseState(bytes32 warehouseId, bytes32 stateHash, string calldata metadataCID) external;
        function warehouseStates(bytes32 warehouseId) external view returns (WarehouseState memory);

        // Stage 4: Logistics
        function recordLogistics(bytes32 shipmentId, bytes32 locationHash, bool isDelivered, string calldata metadataCID) external;

        // Stage 5: Processing
        function processBatch(
            bytes32 inputBatchHash,
            bytes32 transformHash,
            bytes32[] calldata outputBatchHashes,
            string calldata metadataCID
        ) external;

        // Stage 6: Packaging
        function createSKU(bytes32 skuId, bytes32 parentBatchHash, bytes32 merkleRoot, string calldata metadataCID) external;
        function packages(bytes32 skuId) external view returns (PackageRecord memory);

        // Stage 7: Retail / Fraud Detection
        function reportFraud(bytes32 skuId, bytes32 evidenceHash, string calldata evidenceCID) external;

        // Stage 8: AI Scoring
        function commitAIScore(bytes32 batchHash, bytes32 commitHash) external;
        function revealAIScore(bytes32 batchHash, bytes32 revealHash, bytes32 nonce, string calldata metadataCID) external;
        function aiScores(bytes32 batchHash) external view returns (AIScore memory);
        function computeAIScoreCommit(bytes32 revealHash, bytes32 nonce) external pure returns (bytes32);

        // Batch Operations
        function batchUpdateWarehouse(
            bytes32[] calldata warehouseIds,
            bytes32[] calldata stateHashes
        ) external;

        function batchRecordLogistics(
            bytes32[] calldata shipmentIds,
            bytes32[] calldata locationHashes,
            bool[] calldata deliveryStatuses
        ) external;

        // Verification Functions
        function verifyPackageOrigin(bytes32 skuId) external view
            returns (bytes32 parentBatchHash, bytes32 merkleRoot, uint64 packagedAt);

        function verifyFarmer(bytes32 farmerDID) external view
            returns (bool exists, bytes32 cropIDHash, uint64 registeredAt);

        function getWarehouseState(bytes32 warehouseId) external view
            returns (bytes32 stateHash, uint64 lastUpdated);

        function getAIScore(bytes32 batchHash) external view
            returns (bytes32 commitHash, bytes32 revealHash, uint64 committedAt, uint64 revealedAt);
    }
}

pub struct ChainConfig {
    pub rpc_url: String,
    pub private_key: String,
    pub contract_address: String,
    pub chain_id: u64,
}

impl ChainConfig {
    pub fn from_env() -> Result<Self> {
        let rpc_url = env::var("RPC_URL").context("RPC_URL environment variable is required")?;
        let private_key =
            env::var("PRIVATE_KEY").context("PRIVATE_KEY environment variable is required")?;
        let contract_address = env::var("CONTRACT_ADDRESS")
            .context("CONTRACT_ADDRESS environment variable is required")?;
        let chain_id = env::var("CHAIN_ID")
            .unwrap_or_else(|_| "1".to_string())
            .parse::<u64>()
            .context("CHAIN_ID must be a valid u64")?;

        Ok(Self {
            rpc_url,
            private_key,
            contract_address,
            chain_id,
        })
    }
}

#[derive(Clone)]
pub struct ChainClient {
    contract: OilseedValueChain::OilseedValueChainInstance<Http<Client>, AppProvider>,
}

impl ChainClient {
    pub async fn new(config: ChainConfig) -> Result<Self> {
        let signer = config
            .private_key
            .parse::<PrivateKeySigner>()
            .context("Failed to parse private key")?;

        tracing::info!(
            address = ?signer.address(),
            "Initialized signer"
        );

        let wallet = EthereumWallet::from(signer);

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_http(config.rpc_url.parse().context("Invalid RPC URL")?);

        let contract_address: Address = config
            .contract_address
            .parse()
            .context("Failed to parse contract address")?;

        tracing::info!(
            contract_address = ?contract_address,
            rpc_url = %config.rpc_url,
            chain_id = config.chain_id,
            "Initialized ChainClient"
        );

        let contract = OilseedValueChain::new(contract_address, provider);

        Ok(Self { contract })
    }

    pub async fn from_env() -> Result<Self> {
        let config = ChainConfig::from_env()?;
        Self::new(config).await
    }

    pub async fn register_farmer(
        &self,
        farmer_did: FixedBytes<32>,
        crop_id_hash: FixedBytes<32>,
        metadata_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(?farmer_did, ?crop_id_hash, cid = %metadata_cid, "Registering farmer");

        let tx = self
            .contract
            .registerFarmer(farmer_did, crop_id_hash, metadata_cid)
            .send()
            .await
            .context("Failed to send registerFarmer transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "Farmer registered successfully"
        );

        Ok(receipt)
    }

    pub async fn verify_farmer(
        &self,
        farmer_did: FixedBytes<32>,
    ) -> Result<(bool, FixedBytes<32>, u64)> {
        let result = self
            .contract
            .verifyFarmer(farmer_did)
            .call()
            .await
            .context("Failed to call verifyFarmer")?;

        Ok((result.exists, result.cropIDHash, result.registeredAt))
    }

    pub async fn fpo_purchase(
        &self,
        batch_hash: FixedBytes<32>,
        farmer_did: FixedBytes<32>,
        metadata_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(?batch_hash, ?farmer_did, cid = %metadata_cid, "Recording FPO purchase");

        let tx = self
            .contract
            .fpoPurchase(batch_hash, farmer_did, metadata_cid)
            .send()
            .await
            .context("Failed to send fpoPurchase transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "FPO purchase recorded successfully"
        );

        Ok(receipt)
    }

    pub async fn update_warehouse_state(
        &self,
        warehouse_id: FixedBytes<32>,
        state_hash: FixedBytes<32>,
        metadata_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(?warehouse_id, ?state_hash, cid = %metadata_cid, "Updating warehouse state");

        let tx = self
            .contract
            .updateWarehouseState(warehouse_id, state_hash, metadata_cid)
            .send()
            .await
            .context("Failed to send updateWarehouseState transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "Warehouse state updated successfully"
        );

        Ok(receipt)
    }

    pub async fn batch_update_warehouse(
        &self,
        warehouse_ids: Vec<FixedBytes<32>>,
        state_hashes: Vec<FixedBytes<32>>,
    ) -> Result<TransactionReceipt> {
        tracing::info!(
            count = warehouse_ids.len(),
            "Batch updating warehouse states"
        );

        let tx = self
            .contract
            .batchUpdateWarehouse(warehouse_ids, state_hashes)
            .send()
            .await
            .context("Failed to send batchUpdateWarehouse transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "Warehouse states batch updated successfully"
        );

        Ok(receipt)
    }

    pub async fn record_logistics(
        &self,
        shipment_id: FixedBytes<32>,
        location_hash: FixedBytes<32>,
        is_delivered: bool,
        metadata_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(
            ?shipment_id,
            ?location_hash,
            is_delivered,
            cid = %metadata_cid,
            "Recording logistics milestone"
        );

        let tx = self
            .contract
            .recordLogistics(shipment_id, location_hash, is_delivered, metadata_cid)
            .send()
            .await
            .context("Failed to send recordLogistics transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "Logistics milestone recorded successfully"
        );

        Ok(receipt)
    }

    pub async fn process_batch(
        &self,
        input_batch_hash: FixedBytes<32>,
        transform_hash: FixedBytes<32>,
        output_batch_hashes: Vec<FixedBytes<32>>,
        metadata_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(
            ?input_batch_hash,
            ?transform_hash,
            output_count = output_batch_hashes.len(),
            cid = %metadata_cid,
            "Processing batch"
        );

        let tx = self
            .contract
            .processBatch(
                input_batch_hash,
                transform_hash,
                output_batch_hashes,
                metadata_cid,
            )
            .send()
            .await
            .context("Failed to send processBatch transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "Batch processed successfully"
        );

        Ok(receipt)
    }

    pub async fn create_sku(
        &self,
        sku_id: FixedBytes<32>,
        parent_batch_hash: FixedBytes<32>,
        merkle_root: FixedBytes<32>,
        metadata_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(?sku_id, ?parent_batch_hash, ?merkle_root, cid = %metadata_cid, "Creating SKU");

        let tx = self
            .contract
            .createSKU(sku_id, parent_batch_hash, merkle_root, metadata_cid)
            .send()
            .await
            .context("Failed to send createSKU transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "SKU created successfully"
        );

        Ok(receipt)
    }

    pub async fn verify_package_origin(
        &self,
        sku_id: FixedBytes<32>,
    ) -> Result<(FixedBytes<32>, FixedBytes<32>, u64)> {
        let result = self
            .contract
            .verifyPackageOrigin(sku_id)
            .call()
            .await
            .context("Failed to call verifyPackageOrigin")?;

        Ok((result.parentBatchHash, result.merkleRoot, result.packagedAt))
    }

    pub async fn report_fraud(
        &self,
        sku_id: FixedBytes<32>,
        evidence_hash: FixedBytes<32>,
        evidence_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(?sku_id, ?evidence_hash, cid = %evidence_cid, "Reporting fraud");

        let tx = self
            .contract
            .reportFraud(sku_id, evidence_hash, evidence_cid)
            .send()
            .await
            .context("Failed to send reportFraud transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "Fraud reported successfully"
        );

        Ok(receipt)
    }

    pub async fn commit_ai_score(
        &self,
        batch_hash: FixedBytes<32>,
        commit_hash: FixedBytes<32>,
    ) -> Result<TransactionReceipt> {
        tracing::info!(?batch_hash, ?commit_hash, "Committing AI score");

        let tx = self
            .contract
            .commitAIScore(batch_hash, commit_hash)
            .send()
            .await
            .context("Failed to send commitAIScore transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "AI score committed successfully"
        );

        Ok(receipt)
    }

    pub async fn reveal_ai_score(
        &self,
        batch_hash: FixedBytes<32>,
        reveal_hash: FixedBytes<32>,
        nonce: FixedBytes<32>,
        metadata_cid: String,
    ) -> Result<TransactionReceipt> {
        tracing::info!(?batch_hash, ?reveal_hash, cid = %metadata_cid, "Revealing AI score");

        let tx = self
            .contract
            .revealAIScore(batch_hash, reveal_hash, nonce, metadata_cid)
            .send()
            .await
            .context("Failed to send revealAIScore transaction")?;

        let receipt = tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        tracing::info!(
            tx_hash = ?receipt.transaction_hash,
            "AI score revealed successfully"
        );

        Ok(receipt)
    }
}

use alloy::primitives::keccak256;

pub fn hash_string(data: &str) -> FixedBytes<32> {
    keccak256(data.as_bytes())
}

pub fn hash_bytes(data: &[u8]) -> FixedBytes<32> {
    keccak256(data)
}

pub fn generate_commit_hash(reveal_hash: FixedBytes<32>, nonce: FixedBytes<32>) -> FixedBytes<32> {
    keccak256([reveal_hash.as_slice(), nonce.as_slice()].concat())
}
