// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import {OilseedValueChain} from "./OilseedValueChain.sol";

/// @title FarmerCredential
/// @notice Soulbound (ERC-5192, permanently locked) credential attesting a farmer's verified production record.
///         It is issued by the FPO that registered the farmer and points to evidence (anchored record digests) in
///         its IPFS metadata. It is an attestation by that FPO, not a credit score.
/// @dev Transfers and approvals always revert. Issuer or admin can revoke (burn); a farmer holds at most one.
contract FarmerCredential is ERC721 {
    OilseedValueChain public immutable registry;

    uint256 public constant MAX_CID_LENGTH = 128;
    bytes4 private constant ERC5192_INTERFACE_ID = 0xb45a3c0e;

    uint256 public totalIssued;
    mapping(bytes32 => uint256) public credentialOf; // farmerId => tokenId (0 = none)
    mapping(uint256 => bytes32) public farmerOfToken;
    mapping(uint256 => string) private _metadataCids;

    /// @notice ERC-5192
    event Locked(uint256 tokenId);
    event CredentialIssued(uint256 indexed tokenId, bytes32 indexed farmerId, address indexed to, string metadataCid);
    event CredentialRevoked(uint256 indexed tokenId, bytes32 indexed farmerId, address indexed by);

    error Soulbound();
    error FarmerNotRegistered();
    error Unauthorized();
    error AlreadyIssued(uint256 tokenId);
    error InvalidCid();
    error InvalidAddress();

    constructor(OilseedValueChain registry_) ERC721("OilsChain Farmer Credential", "OILCRED") {
        registry = registry_;
    }

    function issue(address to, bytes32 farmerId, string calldata cid) external returns (uint256 tokenId) {
        if (to == address(0)) revert InvalidAddress();
        (, address registeredBy, uint64 registeredAt) = registry.farmers(farmerId);
        if (registeredAt == 0) revert FarmerNotRegistered();
        if (msg.sender != registeredBy && !registry.hasRole(msg.sender, registry.ROLE_ADMIN())) revert Unauthorized();
        uint256 existing = credentialOf[farmerId];
        if (existing != 0) revert AlreadyIssued(existing);
        uint256 cidLength = bytes(cid).length;
        if (cidLength == 0 || cidLength > MAX_CID_LENGTH) revert InvalidCid();

        tokenId = ++totalIssued;
        credentialOf[farmerId] = tokenId;
        farmerOfToken[tokenId] = farmerId;
        _metadataCids[tokenId] = cid;
        _mint(to, tokenId);

        emit Locked(tokenId);
        emit CredentialIssued(tokenId, farmerId, to, cid);
    }

    function revoke(uint256 tokenId) external {
        _requireOwned(tokenId);
        bytes32 farmerId = farmerOfToken[tokenId];
        (, address registeredBy,) = registry.farmers(farmerId);
        if (msg.sender != registeredBy && !registry.hasRole(msg.sender, registry.ROLE_ADMIN())) revert Unauthorized();

        delete credentialOf[farmerId];
        delete farmerOfToken[tokenId];
        delete _metadataCids[tokenId];
        _burn(tokenId);
        emit CredentialRevoked(tokenId, farmerId, msg.sender);
    }

    /// @notice ERC-5192: credentials are always locked.
    function locked(uint256 tokenId) external view returns (bool) {
        _requireOwned(tokenId);
        return true;
    }

    function tokenURI(uint256 tokenId) public view override returns (string memory) {
        _requireOwned(tokenId);
        return string.concat("ipfs://", _metadataCids[tokenId]);
    }

    function supportsInterface(bytes4 interfaceId) public view override returns (bool) {
        return interfaceId == ERC5192_INTERFACE_ID || super.supportsInterface(interfaceId);
    }

    function approve(address, uint256) public pure override {
        revert Soulbound();
    }

    function setApprovalForAll(address, bool) public pure override {
        revert Soulbound();
    }

    function _update(address to, uint256 tokenId, address auth) internal override returns (address) {
        address from = _ownerOf(tokenId);
        if (from != address(0) && to != address(0)) revert Soulbound();
        return super._update(to, tokenId, auth);
    }
}
