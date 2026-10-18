// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

// Gas-optimized on-chain RBAC using bitmap approach + IPFS-friendly events

contract OilseedValueChain {
    // ======================== ACCESS CONTROL (Gas Optimized) ========================
    // Using bitmap for roles: each bit represents a role.
    // Using uint256 instead of uint8 for future-proofing (more roles, no extra storage cost).

    uint256 constant ROLE_ADMIN = 1 << 0; // 000...0001
    uint256 constant ROLE_FARMER = 1 << 1; // 000...0010
    uint256 constant ROLE_FPO = 1 << 2; // 000...0100
    uint256 constant ROLE_WAREHOUSE = 1 << 3; // 000...1000
    uint256 constant ROLE_LOGISTICS = 1 << 4; // 000..1_0000
    uint256 constant ROLE_PROCESSOR = 1 << 5; // ...
    uint256 constant ROLE_PACKAGER = 1 << 6;
    uint256 constant ROLE_AI_ORACLE = 1 << 7;

    mapping(address => uint256) private roles;

    event RoleGranted(address indexed account, uint256 role);
    event RoleRevoked(address indexed account, uint256 role);

    // ======================== CUSTOM ERRORS (Gas Efficient) ========================
    error Unauthorized();
    error AlreadyRegistered();
    error FarmerNotRegistered();
    error SKUAlreadyExists();
    error SKUNotFound();
    error AlreadyCommitted();
    error NotCommitted();
    error AlreadyRevealed();
    error InvalidReveal();
    error LengthMismatch();
    error RevealTooEarly();
    error RevealTooLate();

    constructor() {
        roles[msg.sender] = ROLE_ADMIN;
        emit RoleGranted(msg.sender, ROLE_ADMIN);
    }

    // ======================== ACCESS CONTROL FUNCTIONS ========================

    modifier onlyRole(uint256 role) {
        if ((roles[msg.sender] & role) == 0) revert Unauthorized();
        _;
    }

    function grantRole(
        address account,
        uint256 role
    ) external onlyRole(ROLE_ADMIN) {
        roles[account] |= role; // Set bit(s)
        emit RoleGranted(account, role);
    }

    function revokeRole(
        address account,
        uint256 role
    ) external onlyRole(ROLE_ADMIN) {
        roles[account] &= ~role; // Clear bit(s)
        emit RoleRevoked(account, role);
    }

    function hasRole(
        address account,
        uint256 role
    ) external view returns (bool) {
        return roles[account] & role != 0;
    }

    function getRoles(address account) external view returns (uint256) {
        return roles[account];
    }

    // ======================== STAGE 1: FARMER REGISTER ========================
    // On-chain: Minimal farmer DID, crop ID hash (+ optional IPFS CID in event)
    // Off-chain: Full profile, land docs, satellite info on IPFS

    struct FarmerRecord {
        bytes32 cropIDHash;
        uint64 registeredAt;
    }

    mapping(bytes32 => FarmerRecord) public farmers;

    // metadataCID: IPFS CID for farmer profile JSON (optional, can be empty string)
    event FarmerRegistered(
        bytes32 indexed farmerDID,
        bytes32 indexed cropIDHash,
        uint64 timestamp,
        bytes32 metadataHash, 
        string metadataCID
    );

    function registerFarmer(
        bytes32 farmerDID,
        bytes32 cropIDHash,
        bytes32 metadataHash, 
        string calldata metadataCID
    ) external onlyRole(ROLE_FPO) {
        if (farmers[farmerDID].registeredAt != 0) revert AlreadyRegistered();

        uint64 timestamp = uint64(block.timestamp);

        farmers[farmerDID] = FarmerRecord({
            cropIDHash: cropIDHash,
            registeredAt: timestamp
        });

        emit FarmerRegistered(farmerDID, cropIDHash, timestamp, metadataCID);
    }

    // ======================== STAGE 2: FPO VERIFICATION ========================
    // On-chain: Batch transfer event + optional IPFS metadata CID
    // Off-chain: Quality report, weight slips, photos on IPFS

    uint8 constant TRANSFER_FPO_PURCHASE = 1;
    uint8 constant TRANSFER_WAREHOUSE = 2;
    uint8 constant TRANSFER_PROCESSOR = 3;
    uint8 constant TRANSFER_RETAIL = 4;

    // metadataCID: IPFS CID for docs like receipts, quality reports, etc.
    event OwnershipTransfer(
        bytes32 indexed batchHash,
        bytes32 indexed fromDID,
        address indexed toAddress,
        uint64 timestamp,
        uint8 transferType,
        string metadataCID
    );

    function fpoPurchase(
        bytes32 batchHash,
        bytes32 farmerDID,
        string calldata metadataCID
    ) external onlyRole(ROLE_FPO) {
        if (farmers[farmerDID].registeredAt == 0) revert FarmerNotRegistered();

        emit OwnershipTransfer(
            batchHash,
            farmerDID,
            msg.sender,
            uint64(block.timestamp),
            TRANSFER_FPO_PURCHASE,
            metadataCID
        );
    }

    // ======================== STAGE 3: WAREHOUSE STORAGE ========================
    // On-chain: Timed anchor digest: warehouse state hash + optional CID
    // Off-chain: Continuous IoT logs on IPFS

    struct WarehouseState {
        bytes32 stateHash;
        uint64 lastUpdated;
    }

    mapping(bytes32 => WarehouseState) public warehouseStates;

    // metadataCID: IPFS CID pointing to IoT rollup JSON, audits, etc.
    event WarehouseStateUpdated(
        bytes32 indexed warehouseId,
        bytes32 stateHash,
        uint64 timestamp,
        string metadataCID
    );

    function updateWarehouseState(
        bytes32 warehouseId,
        bytes32 stateHash,
        string calldata metadataCID
    ) external onlyRole(ROLE_WAREHOUSE) {
        uint64 timestamp = uint64(block.timestamp);

        warehouseStates[warehouseId] = WarehouseState({
            stateHash: stateHash,
            lastUpdated: timestamp
        });

        emit WarehouseStateUpdated(
            warehouseId,
            stateHash,
            timestamp,
            metadataCID
        );
    }

    // ======================== STAGE 4: LOGISTICS TRACKING ========================
    // On-chain: Location milestone hashes + optional IPFS CID
    // Off-chain: Full GPS history on IPFS

    // metadataCID: IPFS CID for signed GPS trace / documents
    event LogisticsMilestone(
        bytes32 indexed shipmentId,
        bytes32 locationHash,
        uint64 timestamp,
        bool isDelivered,
        string metadataCID
    );

    function recordLogistics(
        bytes32 shipmentId,
        bytes32 locationHash,
        bool isDelivered,
        string calldata metadataCID
    ) external onlyRole(ROLE_LOGISTICS) {
        emit LogisticsMilestone(
            shipmentId,
            locationHash,
            uint64(block.timestamp),
            isDelivered,
            metadataCID
        );
    }

    // ======================== STAGE 5: PROCESSING ========================
    // On-chain: Processing transform hash + optional CID
    // Off-chain: Yield %, lab results on IPFS

    // metadataCID: IPFS CID for lab reports, yield calculations, etc.
    event BatchProcessed(
        bytes32 indexed inputBatchHash,
        bytes32 transformHash,
        bytes32[] outputBatchHashes,
        uint64 timestamp,
        string metadataCID
    );

    function processBatch(
        bytes32 inputBatchHash,
        bytes32 transformHash,
        bytes32[] calldata outputBatchHashes,
        string calldata metadataCID
    ) external onlyRole(ROLE_PROCESSOR) {
        emit BatchProcessed(
            inputBatchHash,
            transformHash,
            outputBatchHashes,
            uint64(block.timestamp),
            metadataCID
        );
    }

    // ======================== STAGE 6: PACKAGING ========================
    // On-chain: SKU ID → Parent batch commitment + Merkle root + optional CID
    // Off-chain: Packaging metadata JSON on IPFS

    struct PackageRecord {
        bytes32 parentBatchHash;
        bytes32 merkleRoot;
        uint64 packagedAt;
    }

    mapping(bytes32 => PackageRecord) public packages;

    // metadataCID: IPFS CID for packaging manifest (unit list, labels, etc.)
    event SKUPackaged(
        bytes32 indexed skuId,
        bytes32 indexed parentBatchHash,
        bytes32 merkleRoot,
        uint64 timestamp,
        string metadataCID
    );

    function createSKU(
        bytes32 skuId,
        bytes32 parentBatchHash,
        bytes32 merkleRoot,
        string calldata metadataCID
    ) external onlyRole(ROLE_PACKAGER) {
        if (packages[skuId].packagedAt != 0) revert SKUAlreadyExists();

        uint64 timestamp = uint64(block.timestamp);

        packages[skuId] = PackageRecord({
            parentBatchHash: parentBatchHash,
            merkleRoot: merkleRoot,
            packagedAt: timestamp
        });

        emit SKUPackaged(
            skuId,
            parentBatchHash,
            merkleRoot,
            timestamp,
            metadataCID
        );
    }

    // ======================== STAGE 7: RETAIL / FRAUD REPORTING ========================
    // Only fraud-trigger events recorded on-chain.

    // evidenceCID: IPFS CID for photos, lab reports, complaints, etc.
    event FraudDetected(
        bytes32 indexed skuId,
        address indexed reporter,
        bytes32 evidenceHash,
        uint64 timestamp,
        string evidenceCID
    );

    function reportFraud(
        bytes32 skuId,
        bytes32 evidenceHash,
        string calldata evidenceCID
    ) external {
        if (packages[skuId].packagedAt == 0) revert SKUNotFound();

        emit FraudDetected(
            skuId,
            msg.sender,
            evidenceHash,
            uint64(block.timestamp),
            evidenceCID
        );
    }

    // ======================== STAGE 8: AI SCORING (COMMIT–REVEAL) ========================
    // On-chain: commit & reveal hashes + optional IPFS CID for model run
    // Off-chain: Full AI model inputs/outputs on IPFS

    struct AIScore {
        bytes32 commitHash;
        bytes32 revealHash;
        uint64 committedAt;
        uint64 revealedAt;
    }

    mapping(bytes32 => AIScore) public aiScores;

    // Time-locked reveal window (e.g., 1 hour to 24 hours after commit)
    uint64 public constant MIN_REVEAL_DELAY = 1 hours;
    uint64 public constant MAX_REVEAL_DELAY = 24 hours;

    event AIScoreCommitted(
        bytes32 indexed batchHash,
        bytes32 commitHash,
        uint64 timestamp
    );

    // metadataCID: IPFS CID for full scoring report / model explainability JSON
    event AIScoreRevealed(
        bytes32 indexed batchHash,
        bytes32 revealHash,
        uint64 timestamp,
        string metadataCID
    );

    // Helper to compute the commit off-chain and on-chain consistently
    function computeAIScoreCommit(
        bytes32 revealHash,
        bytes32 nonce
    ) public pure returns (bytes32) {
        return keccak256(abi.encodePacked(revealHash, nonce));
    }

    function commitAIScore(
        bytes32 batchHash,
        bytes32 commitHash
    ) external onlyRole(ROLE_AI_ORACLE) {
        if (aiScores[batchHash].committedAt != 0) revert AlreadyCommitted();

        uint64 timestamp = uint64(block.timestamp);

        aiScores[batchHash].commitHash = commitHash;
        aiScores[batchHash].committedAt = timestamp;

        emit AIScoreCommitted(batchHash, commitHash, timestamp);
    }

    function revealAIScore(
        bytes32 batchHash,
        bytes32 revealHash,
        bytes32 nonce,
        string calldata metadataCID
    ) external onlyRole(ROLE_AI_ORACLE) {
        AIScore storage score = aiScores[batchHash];

        if (score.committedAt == 0) revert NotCommitted();
        if (score.revealedAt != 0) revert AlreadyRevealed();

        uint64 timestamp = uint64(block.timestamp);

        // Enforce time-locked reveal period
        if (timestamp < score.committedAt + MIN_REVEAL_DELAY)
            revert RevealTooEarly();
        if (timestamp > score.committedAt + MAX_REVEAL_DELAY)
            revert RevealTooLate();

        // Verify commit matches reveal
        bytes32 expectedCommit = computeAIScoreCommit(revealHash, nonce);
        if (score.commitHash != expectedCommit) revert InvalidReveal();

        score.revealHash = revealHash;
        score.revealedAt = timestamp;

        emit AIScoreRevealed(batchHash, revealHash, timestamp, metadataCID);
    }

    // ======================== BATCH OPERATIONS FOR GAS OPTIMIZATION ========================
    // Note: these keep the same minimal interface (no CIDs here to avoid heavy arrays of strings)

    function batchUpdateWarehouse(
        bytes32[] calldata warehouseIds,
        bytes32[] calldata stateHashes
    ) external onlyRole(ROLE_WAREHOUSE) {
        uint256 length = warehouseIds.length;
        if (length != stateHashes.length) revert LengthMismatch();

        uint64 timestamp = uint64(block.timestamp);

        for (uint256 i = 0; i < length; ) {
            warehouseStates[warehouseIds[i]] = WarehouseState({
                stateHash: stateHashes[i],
                lastUpdated: timestamp
            });

            // No CID in batch mode to avoid string arrays; off-chain can still correlate by timestamp
            emit WarehouseStateUpdated(
                warehouseIds[i],
                stateHashes[i],
                timestamp,
                ""
            );

            unchecked {
                ++i;
            }
        }
    }

    function batchRecordLogistics(
        bytes32[] calldata shipmentIds,
        bytes32[] calldata locationHashes,
        bool[] calldata deliveryStatuses
    ) external onlyRole(ROLE_LOGISTICS) {
        uint256 length = shipmentIds.length;
        if (
            length != locationHashes.length || length != deliveryStatuses.length
        ) revert LengthMismatch();

        uint64 timestamp = uint64(block.timestamp);

        for (uint256 i = 0; i < length; ) {
            emit LogisticsMilestone(
                shipmentIds[i],
                locationHashes[i],
                timestamp,
                deliveryStatuses[i],
                ""
            );

            unchecked {
                ++i;
            }
        }
    }

    // ======================== VERIFICATION / VIEW FUNCTIONS ========================

    function verifyPackageOrigin(
        bytes32 skuId
    )
        external
        view
        returns (bytes32 parentBatchHash, bytes32 merkleRoot, uint64 packagedAt)
    {
        PackageRecord memory pkg = packages[skuId];
        if (pkg.packagedAt == 0) revert SKUNotFound();

        return (pkg.parentBatchHash, pkg.merkleRoot, pkg.packagedAt);
    }

    function verifyFarmer(
        bytes32 farmerDID
    )
        external
        view
        returns (bool exists, bytes32 cropIDHash, uint64 registeredAt)
    {
        FarmerRecord memory farmer = farmers[farmerDID];
        return (
            farmer.registeredAt != 0,
            farmer.cropIDHash,
            farmer.registeredAt
        );
    }

    function getWarehouseState(
        bytes32 warehouseId
    ) external view returns (bytes32 stateHash, uint64 lastUpdated) {
        WarehouseState memory state = warehouseStates[warehouseId];
        return (state.stateHash, state.lastUpdated);
    }

    function getAIScore(
        bytes32 batchHash
    )
        external
        view
        returns (
            bytes32 commitHash,
            bytes32 revealHash,
            uint64 committedAt,
            uint64 revealedAt
        )
    {
        AIScore memory score = aiScores[batchHash];
        return (
            score.commitHash,
            score.revealHash,
            score.committedAt,
            score.revealedAt
        );
    }
}
