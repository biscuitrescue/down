// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {BaseTest} from "./Base.t.sol";
import {BatchProvenanceNFT} from "../src/BatchProvenanceNFT.sol";
import {FarmerCredential} from "../src/FarmerCredential.sol";
import {IERC721Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";

contract BatchProvenanceNFTTest is BaseTest {
    function test_Mint_FromProcurementRecord() public {
        bytes32 digest = _procure();
        vm.prank(fpo);
        uint256 tokenId = provenance.mint(digest, CID);

        assertEq(tokenId, 1);
        assertEq(provenance.ownerOf(tokenId), fpo);
        assertEq(provenance.tokenURI(tokenId), string.concat("ipfs://", CID));
        assertEq(provenance.tokenOfRecord(digest), tokenId);
        assertEq(provenance.batchOfToken(tokenId), BATCH);
        assertEq(provenance.recordOfToken(tokenId), digest);
    }

    function test_Mint_PreventsDuplicates() public {
        bytes32 digest = _procure();
        vm.prank(fpo);
        provenance.mint(digest, CID);
        vm.prank(fpo);
        vm.expectRevert(abi.encodeWithSelector(BatchProvenanceNFT.AlreadyMinted.selector, 1));
        provenance.mint(digest, CID);
    }

    function test_Mint_RejectsInvalidReferences() public {
        bytes32 regDigest = _registerFarmer();
        vm.prank(fpo);
        vm.expectRevert(BatchProvenanceNFT.UnknownRecord.selector);
        provenance.mint(keccak256("arbitrary tx hash"), CID);

        vm.prank(fpo);
        vm.expectRevert(BatchProvenanceNFT.NotMintableRecord.selector);
        provenance.mint(regDigest, CID);

        vm.prank(fpo);
        bytes32 digest = chain.recordProcurement(BATCH, FARMER, h("procurement"), CID);
        vm.prank(fpo);
        vm.expectRevert(BatchProvenanceNFT.InvalidCid.selector);
        provenance.mint(digest, "");
    }

    function test_Mint_OnlySubmitterOrAdmin() public {
        bytes32 digest = _procure();
        vm.prank(fpo2);
        vm.expectRevert(BatchProvenanceNFT.Unauthorized.selector);
        provenance.mint(digest, CID);

        vm.prank(admin);
        uint256 tokenId = provenance.mint(digest, CID);
        assertEq(provenance.ownerOf(tokenId), fpo, "minted to current custodian, not to admin");
    }

    function test_Transfer_FollowsCustody() public {
        bytes32 digest = _procure();
        vm.prank(fpo);
        uint256 tokenId = provenance.mint(digest, CID);

        vm.prank(fpo);
        vm.expectRevert(abi.encodeWithSelector(BatchProvenanceNFT.NotBatchCustodian.selector, stranger));
        provenance.transferFrom(fpo, stranger, tokenId);

        vm.prank(fpo);
        chain.transferCustody(BATCH, warehouse, h("t"), CID);

        vm.prank(fpo);
        vm.expectRevert(abi.encodeWithSelector(BatchProvenanceNFT.NotBatchCustodian.selector, logistics));
        provenance.transferFrom(fpo, logistics, tokenId);

        vm.prank(fpo);
        provenance.transferFrom(fpo, warehouse, tokenId);
        assertEq(provenance.ownerOf(tokenId), warehouse);
    }

    function test_SyncToCustodian_IsPermissionless() public {
        bytes32 digest = _procure();
        vm.prank(fpo);
        uint256 tokenId = provenance.mint(digest, CID);
        vm.prank(fpo);
        chain.transferCustody(BATCH, warehouse, h("t"), CID);

        vm.prank(stranger);
        provenance.syncToCustodian(tokenId);
        assertEq(provenance.ownerOf(tokenId), warehouse);
    }

    function test_TokenURI_RevertsForMissingToken() public {
        vm.expectRevert(abi.encodeWithSelector(IERC721Errors.ERC721NonexistentToken.selector, 42));
        provenance.tokenURI(42);
    }
}

contract FarmerCredentialTest is BaseTest {
    address internal farmerWallet = makeAddr("farmerWallet");

    function test_Issue_ByRegisteringFpo() public {
        _registerFarmer();
        vm.prank(fpo);
        uint256 tokenId = credential.issue(farmerWallet, FARMER, CID);
        assertEq(credential.ownerOf(tokenId), farmerWallet);
        assertTrue(credential.locked(tokenId));
        assertEq(credential.credentialOf(FARMER), tokenId);
        assertEq(credential.tokenURI(tokenId), string.concat("ipfs://", CID));
        assertTrue(credential.supportsInterface(0xb45a3c0e));
        assertTrue(credential.supportsInterface(0x80ac58cd)); // ERC-721
    }

    function test_Issue_Reverts() public {
        vm.prank(fpo);
        vm.expectRevert(FarmerCredential.FarmerNotRegistered.selector);
        credential.issue(farmerWallet, FARMER, CID);

        _registerFarmer();
        vm.prank(fpo2);
        vm.expectRevert(FarmerCredential.Unauthorized.selector);
        credential.issue(farmerWallet, FARMER, CID);

        vm.prank(fpo);
        credential.issue(farmerWallet, FARMER, CID);
        vm.prank(admin);
        vm.expectRevert(abi.encodeWithSelector(FarmerCredential.AlreadyIssued.selector, 1));
        credential.issue(farmerWallet, FARMER, CID);
    }

    function test_Soulbound_TransfersAndApprovalsRevert() public {
        _registerFarmer();
        vm.prank(fpo);
        uint256 tokenId = credential.issue(farmerWallet, FARMER, CID);

        vm.startPrank(farmerWallet);
        vm.expectRevert(FarmerCredential.Soulbound.selector);
        credential.transferFrom(farmerWallet, stranger, tokenId);
        vm.expectRevert(FarmerCredential.Soulbound.selector);
        credential.safeTransferFrom(farmerWallet, stranger, tokenId);
        vm.expectRevert(FarmerCredential.Soulbound.selector);
        credential.approve(stranger, tokenId);
        vm.expectRevert(FarmerCredential.Soulbound.selector);
        credential.setApprovalForAll(stranger, true);
        vm.stopPrank();
    }

    function test_Revoke_ThenReissue() public {
        _registerFarmer();
        vm.prank(fpo);
        uint256 tokenId = credential.issue(farmerWallet, FARMER, CID);

        vm.prank(stranger);
        vm.expectRevert(FarmerCredential.Unauthorized.selector);
        credential.revoke(tokenId);

        vm.prank(fpo);
        credential.revoke(tokenId);
        assertEq(credential.credentialOf(FARMER), 0);
        vm.expectRevert(abi.encodeWithSelector(IERC721Errors.ERC721NonexistentToken.selector, tokenId));
        credential.ownerOf(tokenId);

        vm.prank(fpo);
        uint256 reissued = credential.issue(farmerWallet, FARMER, CID);
        assertEq(reissued, 2);
    }
}
