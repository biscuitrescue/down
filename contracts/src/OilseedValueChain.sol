// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "@openzeppelin/contracts/utils/cryptography/MerkleProof.sol";

/// @title OilseedValueChain
/// @notice Batch-level traceability registry for the oilseed value chain.
/// @dev Storage holds only identifiers, commitments, custody and lifecycle state. Full records live off-chain
///      (IPFS); each one is committed here as a domain-separated `recordDigest` over its `contentHash`
///      (keccak256 of the record's canonical JSON, see docs/INTEGRITY.md). The IPFS CID is emitted in the
///      `RecordAnchored` event as a locator only and is never written to storage.
///
///      Every batch keeps a hash chain `head = keccak256(abi.encode(head, recordDigest))` over its records,
///      so a verifier can prove that an off-chain timeline is complete and correctly ordered.
contract OilseedValueChain {
    // ======================== ACCESS CONTROL (bitmap) ========================
    // Each bit is a role. One storage read per check; several roles can be granted in one write.

    uint256 public constant ROLE_ADMIN = 1 << 0;
    uint256 public constant ROLE_FARMER = 1 << 1; // reserved: farmers are onboarded by FPOs and need no wallet
    uint256 public constant ROLE_FPO = 1 << 2;
    uint256 public constant ROLE_WAREHOUSE = 1 << 3;
    uint256 public constant ROLE_LOGISTICS = 1 << 4;
    uint256 public constant ROLE_PROCESSOR = 1 << 5;
    uint256 public constant ROLE_PACKAGER = 1 << 6;
    uint256 public constant ROLE_AI_ORACLE = 1 << 7;
    uint256 public constant ROLE_RETAILER = 1 << 8;

    uint256 public constant ALL_ROLES = (1 << 9) - 1;
    /// @notice Roles allowed to hold custody of a batch.
    uint256 public constant CUSTODIAN_ROLES =
        ROLE_FPO | ROLE_WAREHOUSE | ROLE_LOGISTICS | ROLE_PROCESSOR | ROLE_PACKAGER | ROLE_RETAILER;

    mapping(address => uint256) private roles;

    event RoleGranted(address indexed account, uint256 role, address indexed sender);
    event RoleRevoked(address indexed account, uint256 role, address indexed sender);

    // ======================== RECORD TYPES / LIFECYCLE ========================

    uint8 public constant RT_FARMER_REGISTRATION = 1;
    uint8 public constant RT_PROCUREMENT = 2;
    uint8 public constant RT_CUSTODY_TRANSFER = 3;
    uint8 public constant RT_WAREHOUSE_INSPECTION = 4;
    uint8 public constant RT_LOGISTICS = 5;
    uint8 public constant RT_PROCESSING = 6;
    uint8 public constant RT_PACKAGING = 7;
    uint8 public constant RT_FRAUD_REPORT = 8;
    uint8 public constant RT_FRAUD_RESOLUTION = 9;
    uint8 public constant RT_AI_ASSESSMENT = 10;

    /// @dev Stage reflects the latest lifecycle event of a batch. `Consumed` is terminal (input of processing).
    enum Stage {
        None,
        Procured,
        Stored,
        InTransit,
        Processed,
        Consumed,
        Packaged
    }

    enum FraudStatus {
        None,
        Open,
        Upheld,
        Dismissed
    }

    bytes32 public constant RECORD_TYPEHASH = keccak256(
        "OilsChainRecord(uint256 chainId,address registry,uint8 recordType,bytes32 subjectId,bytes32 contentHash)"
    );
    bytes32 public constant AI_COMMIT_TYPEHASH =
        keccak256("OilsChainAICommit(bytes32 batchId,bytes32 contentHash,bytes32 salt,address oracle)");

    uint256 public constant MAX_CID_LENGTH = 128;
    uint256 public constant MAX_PROCESSING_OUTPUTS = 16;

    // Time-locked reveal window for AI assessments (1 hour to 24 hours after commit)
    uint64 public constant MIN_REVEAL_DELAY = 1 hours;
    uint64 public constant MAX_REVEAL_DELAY = 24 hours;

    // ======================== STORAGE ========================

    struct Farmer {
        bytes32 registrationDigest; // recordDigest of the (encrypted) registration record
        address registeredBy; // FPO that onboarded the farmer
        uint64 registeredAt;
    }

    struct Batch {
        bytes32 farmerId;
        bytes32 parentBatchId; // non-zero for processing outputs
        bytes32 head; // hash chain over this batch's record digests
        address custodian;
        uint64 createdAt;
        Stage stage;
        uint16 recordCount;
    }

    struct Sku {
        bytes32 batchId;
        bytes32 unitsRoot; // Merkle root over unit leaves, see unitLeaf()
        uint64 packagedAt;
        uint32 unitCount;
    }

    struct Anchor {
        bytes32 subjectId; // batchId, or farmerId for registrations
        address submitter;
        uint64 anchoredAt;
        uint8 recordType;
    }

    struct AICommit {
        bytes32 commitHash;
        address oracle;
        uint64 committedAt;
        bool revealed;
    }

    mapping(bytes32 => Farmer) public farmers;
    mapping(bytes32 => Batch) private batches;
    mapping(bytes32 => Sku) public skus;
    mapping(bytes32 => Anchor) public anchors;
    mapping(bytes32 => AICommit) public aiCommits;
    mapping(bytes32 => FraudStatus) public fraudStatus;

    // ======================== EVENTS ========================

    /// @notice Emitted for every anchored record. `head` is the subject batch's hash-chain head after this
    ///         record (zero for non-batch subjects). `cid` locates the full record on IPFS.
    event RecordAnchored(
        bytes32 indexed subjectId,
        uint8 indexed recordType,
        bytes32 indexed recordDigest,
        bytes32 contentHash,
        address submitter,
        bytes32 head,
        string cid
    );
    event FarmerRegistered(bytes32 indexed farmerId, address indexed registeredBy, bytes32 registrationDigest);
    event BatchCreated(
        bytes32 indexed batchId, bytes32 indexed farmerId, bytes32 indexed parentBatchId, address custodian
    );
    event CustodyTransferred(bytes32 indexed batchId, address indexed from, address indexed to);
    event StageChanged(bytes32 indexed batchId, Stage stage);
    event SkuCreated(bytes32 indexed skuId, bytes32 indexed batchId, bytes32 unitsRoot, uint32 unitCount);
    event FraudReported(bytes32 indexed batchId, bytes32 indexed reportDigest, address indexed reporter);
    event FraudResolved(bytes32 indexed reportDigest, bool upheld, address indexed reviewer);
    event AIAssessmentCommitted(bytes32 indexed batchId, address indexed oracle, bytes32 commitHash);

    // ======================== CUSTOM ERRORS ========================

    error Unauthorized();
    error InvalidRole();
    error InvalidAddress();
    error CannotRevokeOwnAdmin();
    error InvalidId();
    error EmptyContentHash();
    error InvalidCid();
    error AlreadyAnchored();
    error AlreadyRegistered();
    error FarmerNotRegistered();
    error BatchAlreadyExists();
    error BatchNotFound();
    error NotCustodian();
    error InvalidCustodian();
    error InvalidStage();
    error NotAProductBatch();
    error InvalidOutputs();
    error SKUAlreadyExists();
    error SKUNotFound();
    error InvalidUnits();
    error ReportNotOpen();
    error CannotResolveOwnReport();
    error AlreadyCommitted();
    error NotCommitted();
    error AlreadyRevealed();
    error InvalidReveal();
    error RevealTooEarly();
    error RevealTooLate();

    constructor() {
        // Least privilege: the deployer is admin only and must grant operational roles explicitly.
        roles[msg.sender] = ROLE_ADMIN;
        emit RoleGranted(msg.sender, ROLE_ADMIN, msg.sender);
    }

    // ======================== ACCESS CONTROL FUNCTIONS ========================

    modifier onlyRole(uint256 role) {
        if (roles[msg.sender] & role == 0) revert Unauthorized();
        _;
    }

    function grantRole(address account, uint256 role) external onlyRole(ROLE_ADMIN) {
        if (account == address(0)) revert InvalidAddress();
        if (role == 0 || role & ~ALL_ROLES != 0) revert InvalidRole();
        roles[account] |= role; // Set bit(s)
        emit RoleGranted(account, role, msg.sender);
    }

    function revokeRole(address account, uint256 role) external onlyRole(ROLE_ADMIN) {
        if (role == 0 || role & ~ALL_ROLES != 0) revert InvalidRole();
        // Prevent an admin from locking itself out; another admin must revoke it.
        if (account == msg.sender && role & ROLE_ADMIN != 0) revert CannotRevokeOwnAdmin();
        roles[account] &= ~role; // Clear bit(s)
        emit RoleRevoked(account, role, msg.sender);
    }

    /// @notice True if `account` holds ANY of the bits in `role`.
    function hasRole(address account, uint256 role) external view returns (bool) {
        return roles[account] & role != 0;
    }

    function getRoles(address account) external view returns (uint256) {
        return roles[account];
    }

    function isCustodianEligible(address account) public view returns (bool) {
        return roles[account] & CUSTODIAN_ROLES != 0;
    }

    // ======================== DIGESTS ========================

    /// @notice Domain-separated commitment binding a record's content to its type, subject, chain and registry.
    function computeRecordDigest(uint8 recordType, bytes32 subjectId, bytes32 contentHash)
        public
        view
        returns (bytes32)
    {
        return keccak256(abi.encode(RECORD_TYPEHASH, block.chainid, address(this), recordType, subjectId, contentHash));
    }

    function computeAICommit(bytes32 batchId, bytes32 contentHash, bytes32 salt, address oracle)
        public
        pure
        returns (bytes32)
    {
        return keccak256(abi.encode(AI_COMMIT_TYPEHASH, batchId, contentHash, salt, oracle));
    }

    /// @notice Merkle leaf for one retail unit of a SKU (OpenZeppelin StandardMerkleTree convention).
    function unitLeaf(bytes32 skuId, string calldata unitId) public pure returns (bytes32) {
        return keccak256(bytes.concat(keccak256(abi.encode(skuId, keccak256(bytes(unitId))))));
    }

    // ======================== STAGE 1: FARMER REGISTRATION ========================
    // On-chain: random farmerId + commitment to the encrypted registration record.
    // Off-chain: profile (name, contact, land, location) encrypted on IPFS; key never on-chain.

    function registerFarmer(bytes32 farmerId, bytes32 contentHash, string calldata cid)
        external
        onlyRole(ROLE_FPO)
        returns (bytes32 digest)
    {
        if (farmerId == bytes32(0)) revert InvalidId();
        if (farmers[farmerId].registeredAt != 0) revert AlreadyRegistered();

        digest = _anchor(RT_FARMER_REGISTRATION, farmerId, contentHash, cid);
        farmers[farmerId] =
            Farmer({registrationDigest: digest, registeredBy: msg.sender, registeredAt: uint64(block.timestamp)});

        emit FarmerRegistered(farmerId, msg.sender, digest);
        emit RecordAnchored(farmerId, RT_FARMER_REGISTRATION, digest, contentHash, msg.sender, bytes32(0), cid);
    }

    // ======================== STAGE 2: FPO PROCUREMENT (batch creation) ========================

    function recordProcurement(bytes32 batchId, bytes32 farmerId, bytes32 contentHash, string calldata cid)
        external
        onlyRole(ROLE_FPO)
        returns (bytes32 digest)
    {
        if (batchId == bytes32(0)) revert InvalidId();
        if (farmers[farmerId].registeredAt == 0) revert FarmerNotRegistered();
        Batch storage b = batches[batchId];
        if (b.createdAt != 0) revert BatchAlreadyExists();

        b.farmerId = farmerId;
        b.custodian = msg.sender;
        b.createdAt = uint64(block.timestamp);
        b.stage = Stage.Procured;

        emit BatchCreated(batchId, farmerId, bytes32(0), msg.sender);
        emit StageChanged(batchId, Stage.Procured);
        digest = _anchorBatchRecord(b, batchId, RT_PROCUREMENT, contentHash, cid);
    }

    // ======================== CUSTODY ========================

    function transferCustody(bytes32 batchId, address to, bytes32 contentHash, string calldata cid)
        external
        returns (bytes32 digest)
    {
        Batch storage b = _custodiedBatch(batchId);
        if (b.stage == Stage.Consumed) revert InvalidStage();
        if (to == msg.sender || !isCustodianEligible(to)) revert InvalidCustodian();

        b.custodian = to;
        emit CustodyTransferred(batchId, msg.sender, to);
        digest = _anchorBatchRecord(b, batchId, RT_CUSTODY_TRANSFER, contentHash, cid);
    }

    // ======================== STAGE 3: WAREHOUSE STORAGE / QUALITY INSPECTION ========================

    function recordWarehouseInspection(bytes32 batchId, bytes32 contentHash, string calldata cid)
        external
        onlyRole(ROLE_WAREHOUSE)
        returns (bytes32 digest)
    {
        Batch storage b = _custodiedBatch(batchId);
        if (b.stage == Stage.Consumed) revert InvalidStage();
        _setStage(b, batchId, Stage.Stored);
        digest = _anchorBatchRecord(b, batchId, RT_WAREHOUSE_INSPECTION, contentHash, cid);
    }

    // ======================== STAGE 4: LOGISTICS ========================

    function recordLogistics(bytes32 batchId, bytes32 contentHash, string calldata cid)
        external
        onlyRole(ROLE_LOGISTICS)
        returns (bytes32 digest)
    {
        Batch storage b = _custodiedBatch(batchId);
        if (b.stage == Stage.Consumed) revert InvalidStage();
        _setStage(b, batchId, Stage.InTransit);
        digest = _anchorBatchRecord(b, batchId, RT_LOGISTICS, contentHash, cid);
    }

    // ======================== STAGE 5: PROCESSING ========================
    // The input batch is consumed; each output batch inherits the farmer and links back via parentBatchId.
    // Output heads start from the processing record so their history is bound to the input's.

    function processBatch(
        bytes32 inputBatchId,
        bytes32[] calldata outputBatchIds,
        bytes32 contentHash,
        string calldata cid
    ) external onlyRole(ROLE_PROCESSOR) returns (bytes32 digest) {
        Batch storage input = _custodiedBatch(inputBatchId);
        if (input.stage == Stage.Consumed) revert InvalidStage();
        uint256 n = outputBatchIds.length;
        if (n == 0 || n > MAX_PROCESSING_OUTPUTS) revert InvalidOutputs();

        digest = _anchorBatchRecord(input, inputBatchId, RT_PROCESSING, contentHash, cid);
        _setStage(input, inputBatchId, Stage.Consumed);

        bytes32 farmerId = input.farmerId;
        bytes32 outputHead = keccak256(abi.encode(bytes32(0), digest));
        uint64 timestamp = uint64(block.timestamp);
        for (uint256 i = 0; i < n;) {
            bytes32 outId = outputBatchIds[i];
            if (outId == bytes32(0)) revert InvalidId();
            Batch storage out = batches[outId];
            if (out.createdAt != 0) revert BatchAlreadyExists();

            out.farmerId = farmerId;
            out.parentBatchId = inputBatchId;
            out.head = outputHead;
            out.custodian = msg.sender;
            out.createdAt = timestamp;
            out.stage = Stage.Processed;
            out.recordCount = 1;

            emit BatchCreated(outId, farmerId, inputBatchId, msg.sender);
            emit StageChanged(outId, Stage.Processed);
            unchecked {
                ++i;
            }
        }
    }

    // ======================== STAGE 6: PACKAGING ========================

    function createSku(
        bytes32 skuId,
        bytes32 batchId,
        bytes32 unitsRoot,
        uint32 unitCount,
        bytes32 contentHash,
        string calldata cid
    ) external onlyRole(ROLE_PACKAGER) returns (bytes32 digest) {
        Batch storage b = _custodiedBatch(batchId);
        if (b.parentBatchId == bytes32(0)) revert NotAProductBatch();
        if (b.stage == Stage.Consumed) revert InvalidStage();
        if (skuId == bytes32(0)) revert InvalidId();
        if (skus[skuId].packagedAt != 0) revert SKUAlreadyExists();
        if (unitsRoot == bytes32(0) || unitCount == 0) revert InvalidUnits();

        skus[skuId] =
            Sku({batchId: batchId, unitsRoot: unitsRoot, packagedAt: uint64(block.timestamp), unitCount: unitCount});
        _setStage(b, batchId, Stage.Packaged);
        emit SkuCreated(skuId, batchId, unitsRoot, unitCount);
        digest = _anchorBatchRecord(b, batchId, RT_PACKAGING, contentHash, cid);
    }

    // ======================== STAGE 7: FRAUD REPORTING (with review) ========================
    // Reports become part of the batch history; only an admin other than the reporter can resolve them.

    function reportFraud(bytes32 batchId, bytes32 contentHash, string calldata cid)
        external
        onlyRole(ALL_ROLES)
        returns (bytes32 digest)
    {
        Batch storage b = _existingBatch(batchId);
        digest = _anchorBatchRecord(b, batchId, RT_FRAUD_REPORT, contentHash, cid);
        fraudStatus[digest] = FraudStatus.Open;
        emit FraudReported(batchId, digest, msg.sender);
    }

    function resolveFraud(bytes32 reportDigest, bool upheld, bytes32 contentHash, string calldata cid)
        external
        onlyRole(ROLE_ADMIN)
        returns (bytes32 digest)
    {
        if (fraudStatus[reportDigest] != FraudStatus.Open) revert ReportNotOpen();
        Anchor storage report = anchors[reportDigest];
        if (report.submitter == msg.sender) revert CannotResolveOwnReport();

        fraudStatus[reportDigest] = upheld ? FraudStatus.Upheld : FraudStatus.Dismissed;
        emit FraudResolved(reportDigest, upheld, msg.sender);
        bytes32 batchId = report.subjectId;
        digest = _anchorBatchRecord(batches[batchId], batchId, RT_FRAUD_RESOLUTION, contentHash, cid);
    }

    // ======================== STAGE 8: AI ASSESSMENT (COMMIT–REVEAL) ========================
    // The oracle commits to an assessment before it is public and must reveal it within the window.
    // An expired, unrevealed commit can be replaced, so a batch is never permanently blocked.

    function commitAIAssessment(bytes32 batchId, bytes32 commitHash) external onlyRole(ROLE_AI_ORACLE) {
        _existingBatch(batchId);
        if (commitHash == bytes32(0)) revert InvalidReveal();
        AICommit storage c = aiCommits[batchId];
        if (c.committedAt != 0 && !c.revealed && block.timestamp <= c.committedAt + MAX_REVEAL_DELAY) {
            revert AlreadyCommitted();
        }

        aiCommits[batchId] = AICommit({
            commitHash: commitHash, oracle: msg.sender, committedAt: uint64(block.timestamp), revealed: false
        });
        emit AIAssessmentCommitted(batchId, msg.sender, commitHash);
    }

    function revealAIAssessment(bytes32 batchId, bytes32 contentHash, bytes32 salt, string calldata cid)
        external
        onlyRole(ROLE_AI_ORACLE)
        returns (bytes32 digest)
    {
        AICommit storage c = aiCommits[batchId];
        if (c.committedAt == 0) revert NotCommitted();
        if (c.revealed) revert AlreadyRevealed();
        if (c.oracle != msg.sender) revert Unauthorized();

        // Enforce time-locked reveal period
        if (block.timestamp < c.committedAt + MIN_REVEAL_DELAY) revert RevealTooEarly();
        if (block.timestamp > c.committedAt + MAX_REVEAL_DELAY) revert RevealTooLate();

        // Verify commit matches reveal
        if (computeAICommit(batchId, contentHash, salt, msg.sender) != c.commitHash) revert InvalidReveal();

        c.revealed = true;
        digest = _anchorBatchRecord(batches[batchId], batchId, RT_AI_ASSESSMENT, contentHash, cid);
    }

    // ======================== VERIFICATION / VIEW FUNCTIONS ========================

    function getBatch(bytes32 batchId) external view returns (Batch memory) {
        return batches[batchId];
    }

    function custodianOf(bytes32 batchId) external view returns (address) {
        return batches[batchId].custodian;
    }

    /// @notice True if `recordDigest` was anchored for `subjectId` with `recordType`.
    function verifyRecord(bytes32 recordDigest, bytes32 subjectId, uint8 recordType) external view returns (bool) {
        Anchor storage a = anchors[recordDigest];
        return a.anchoredAt != 0 && a.subjectId == subjectId && a.recordType == recordType;
    }

    function verifyPackageOrigin(bytes32 skuId)
        external
        view
        returns (bytes32 batchId, bytes32 parentBatchId, bytes32 farmerId, bytes32 unitsRoot, uint64 packagedAt)
    {
        Sku memory s = skus[skuId];
        if (s.packagedAt == 0) revert SKUNotFound();
        Batch storage b = batches[s.batchId];
        return (s.batchId, b.parentBatchId, b.farmerId, s.unitsRoot, s.packagedAt);
    }

    /// @notice Verify that `unitId` belongs to SKU `skuId` using a Merkle proof.
    function verifyUnit(bytes32 skuId, string calldata unitId, bytes32[] calldata proof) external view returns (bool) {
        Sku storage s = skus[skuId];
        if (s.packagedAt == 0) revert SKUNotFound();
        return MerkleProof.verifyCalldata(proof, s.unitsRoot, unitLeaf(skuId, unitId));
    }

    // ======================== INTERNAL ========================

    function _existingBatch(bytes32 batchId) internal view returns (Batch storage b) {
        b = batches[batchId];
        if (b.createdAt == 0) revert BatchNotFound();
    }

    function _custodiedBatch(bytes32 batchId) internal view returns (Batch storage b) {
        b = _existingBatch(batchId);
        if (b.custodian != msg.sender) revert NotCustodian();
    }

    function _setStage(Batch storage b, bytes32 batchId, Stage stage) internal {
        if (b.stage != stage) {
            b.stage = stage;
            emit StageChanged(batchId, stage);
        }
    }

    function _anchor(uint8 recordType, bytes32 subjectId, bytes32 contentHash, string calldata cid)
        internal
        returns (bytes32 digest)
    {
        if (contentHash == bytes32(0)) revert EmptyContentHash();
        uint256 cidLength = bytes(cid).length;
        if (cidLength == 0 || cidLength > MAX_CID_LENGTH) revert InvalidCid();

        digest = computeRecordDigest(recordType, subjectId, contentHash);
        Anchor storage a = anchors[digest];
        if (a.anchoredAt != 0) revert AlreadyAnchored();
        a.subjectId = subjectId;
        a.submitter = msg.sender;
        a.anchoredAt = uint64(block.timestamp);
        a.recordType = recordType;
    }

    function _anchorBatchRecord(
        Batch storage b,
        bytes32 batchId,
        uint8 recordType,
        bytes32 contentHash,
        string calldata cid
    ) internal returns (bytes32 digest) {
        digest = _anchor(recordType, batchId, contentHash, cid);
        bytes32 head = keccak256(abi.encode(b.head, digest));
        b.head = head;
        b.recordCount += 1;
        emit RecordAnchored(batchId, recordType, digest, contentHash, msg.sender, head, cid);
    }
}
