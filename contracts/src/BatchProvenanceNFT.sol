// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import {OilseedValueChain} from "./OilseedValueChain.sol";

/// @title BatchProvenanceNFT
/// @notice ERC-721 provenance certificate: exactly one token per anchored procurement record (a farmer's
///         yield sold to an FPO). The token links to the verified on-chain record; it does not by itself prove
///         any real-world claim beyond what that record commits to.
/// @dev Ownership follows custody: a token can only move to the current custodian of its batch, either through a
///      normal transfer by the holder or through the permissionless `syncToCustodian`.
contract BatchProvenanceNFT is ERC721 {
    OilseedValueChain public immutable registry;

    uint256 public constant MAX_CID_LENGTH = 128;

    uint256 public totalMinted;
    mapping(bytes32 => uint256) public tokenOfRecord; // procurement recordDigest => tokenId
    mapping(uint256 => bytes32) public recordOfToken;
    mapping(uint256 => bytes32) public batchOfToken;
    mapping(uint256 => string) private _metadataCids;

    event ProvenanceMinted(
        uint256 indexed tokenId,
        bytes32 indexed batchId,
        bytes32 indexed sourceRecordDigest,
        address to,
        string metadataCid
    );

    error UnknownRecord();
    error NotMintableRecord();
    error Unauthorized();
    error AlreadyMinted(uint256 tokenId);
    error InvalidCid();
    error NotBatchCustodian(address to);

    constructor(OilseedValueChain registry_) ERC721("OilsChain Batch Provenance", "OILPROV") {
        registry = registry_;
    }

    /// @notice Mint the provenance NFT for a procurement record. Only the FPO that anchored the record (or an
    ///         admin) can mint; the token goes to the batch's current custodian.
    function mint(bytes32 sourceRecordDigest, string calldata cid) external returns (uint256 tokenId) {
        (bytes32 batchId, address submitter, uint64 anchoredAt, uint8 recordType) = registry.anchors(sourceRecordDigest);
        if (anchoredAt == 0) revert UnknownRecord();
        if (recordType != registry.RT_PROCUREMENT()) revert NotMintableRecord();
        if (msg.sender != submitter && !registry.hasRole(msg.sender, registry.ROLE_ADMIN())) revert Unauthorized();
        uint256 existing = tokenOfRecord[sourceRecordDigest];
        if (existing != 0) revert AlreadyMinted(existing);
        uint256 cidLength = bytes(cid).length;
        if (cidLength == 0 || cidLength > MAX_CID_LENGTH) revert InvalidCid();

        tokenId = ++totalMinted;
        tokenOfRecord[sourceRecordDigest] = tokenId;
        recordOfToken[tokenId] = sourceRecordDigest;
        batchOfToken[tokenId] = batchId;
        _metadataCids[tokenId] = cid;

        address to = registry.custodianOf(batchId);
        // _mint (not _safeMint): no receiver callback, so no reentrancy surface.
        _mint(to, tokenId);
        emit ProvenanceMinted(tokenId, batchId, sourceRecordDigest, to, cid);
    }

    /// @notice Move a token to its batch's current custodian. Callable by anyone; it can only align ownership
    ///         with on-chain custody.
    function syncToCustodian(uint256 tokenId) external {
        _requireOwned(tokenId);
        address custodian = registry.custodianOf(batchOfToken[tokenId]);
        if (ownerOf(tokenId) != custodian) {
            _update(custodian, tokenId, address(0));
        }
    }

    function tokenURI(uint256 tokenId) public view override returns (string memory) {
        _requireOwned(tokenId);
        return string.concat("ipfs://", _metadataCids[tokenId]);
    }

    function metadataCid(uint256 tokenId) external view returns (string memory) {
        _requireOwned(tokenId);
        return _metadataCids[tokenId];
    }

    function _update(address to, uint256 tokenId, address auth) internal override returns (address) {
        address from = _ownerOf(tokenId);
        if (from != address(0) && to != address(0)) {
            if (to != registry.custodianOf(batchOfToken[tokenId])) revert NotBatchCustodian(to);
        }
        return super._update(to, tokenId, auth);
    }
}
