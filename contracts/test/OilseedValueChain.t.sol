// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {BaseTest} from "./Base.t.sol";
import {OilseedValueChain} from "../src/OilseedValueChain.sol";

contract OilseedValueChainTest is BaseTest {
    // ======================== ACCESS CONTROL ========================

    function test_DeployerIsAdminOnly() public view {
        assertEq(chain.getRoles(admin), chain.ROLE_ADMIN());
        assertFalse(chain.hasRole(admin, chain.ROLE_FPO()));
    }

    function test_GrantRole_RevertsForNonAdmin() public {
        uint256 role = chain.ROLE_FPO();
        vm.prank(stranger);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.grantRole(stranger, role);
    }

    function test_GrantRole_RejectsUnknownBitsAndZero() public {
        vm.startPrank(admin);
        vm.expectRevert(OilseedValueChain.InvalidRole.selector);
        chain.grantRole(stranger, 0);
        vm.expectRevert(OilseedValueChain.InvalidRole.selector);
        chain.grantRole(stranger, 1 << 9);
        vm.expectRevert(OilseedValueChain.InvalidAddress.selector);
        chain.grantRole(address(0), 1 << 2);
        vm.stopPrank();
    }

    function test_RevokeRole_CannotRevokeOwnAdmin() public {
        uint256 role = chain.ROLE_ADMIN();
        vm.prank(admin);
        vm.expectRevert(OilseedValueChain.CannotRevokeOwnAdmin.selector);
        chain.revokeRole(admin, role);

        vm.prank(admin2);
        chain.revokeRole(admin, role);
        assertFalse(chain.hasRole(admin, role));
    }

    function test_RevokedParticipantCannotWrite() public {
        _registerFarmer();
        uint256 role = chain.ROLE_FPO();
        vm.prank(admin);
        chain.revokeRole(fpo, role);
        vm.prank(fpo);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.recordProcurement(BATCH, FARMER, h("p"), CID);
    }

    // ======================== FARMER REGISTRATION ========================

    function test_RegisterFarmer_StoresCommitmentOnly() public {
        bytes32 contentHash = h("registration");
        bytes32 expected = chain.computeRecordDigest(chain.RT_FARMER_REGISTRATION(), FARMER, contentHash);

        vm.expectEmit(true, true, true, true, address(chain));
        emit OilseedValueChain.RecordAnchored(
            FARMER, chain.RT_FARMER_REGISTRATION(), expected, contentHash, fpo, bytes32(0), CID
        );
        bytes32 digest = _registerFarmer();

        assertEq(digest, expected);
        (bytes32 regDigest, address by, uint64 at) = chain.farmers(FARMER);
        assertEq(regDigest, digest);
        assertEq(by, fpo);
        assertEq(at, block.timestamp);
        assertTrue(chain.verifyRecord(digest, FARMER, chain.RT_FARMER_REGISTRATION()));
    }

    function test_RegisterFarmer_Reverts() public {
        vm.prank(warehouse);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.registerFarmer(FARMER, h("r"), CID);

        vm.startPrank(fpo);
        vm.expectRevert(OilseedValueChain.InvalidId.selector);
        chain.registerFarmer(bytes32(0), h("r"), CID);
        vm.expectRevert(OilseedValueChain.EmptyContentHash.selector);
        chain.registerFarmer(FARMER, bytes32(0), CID);
        vm.expectRevert(OilseedValueChain.InvalidCid.selector);
        chain.registerFarmer(FARMER, h("r"), "");
        vm.expectRevert(OilseedValueChain.InvalidCid.selector);
        chain.registerFarmer(FARMER, h("r"), string(new bytes(129)));
        chain.registerFarmer(FARMER, h("r"), CID);
        vm.expectRevert(OilseedValueChain.AlreadyRegistered.selector);
        chain.registerFarmer(FARMER, h("r2"), CID);
        vm.stopPrank();
    }

    // ======================== PROCUREMENT / BATCH ========================

    function test_RecordProcurement_CreatesBatch() public {
        bytes32 digest = _procure();
        OilseedValueChain.Batch memory b = chain.getBatch(BATCH);
        assertEq(b.farmerId, FARMER);
        assertEq(b.parentBatchId, bytes32(0));
        assertEq(b.custodian, fpo);
        assertEq(uint8(b.stage), uint8(OilseedValueChain.Stage.Procured));
        assertEq(b.recordCount, 1);
        assertEq(b.head, keccak256(abi.encode(bytes32(0), digest)));
        assertTrue(chain.verifyRecord(digest, BATCH, chain.RT_PROCUREMENT()));
        assertFalse(chain.verifyRecord(digest, BATCH, chain.RT_LOGISTICS()));
        assertFalse(chain.verifyRecord(digest, OIL, chain.RT_PROCUREMENT()));
    }

    function test_RecordProcurement_Reverts() public {
        vm.prank(fpo);
        vm.expectRevert(OilseedValueChain.FarmerNotRegistered.selector);
        chain.recordProcurement(BATCH, FARMER, h("p"), CID);

        _procure();
        vm.prank(fpo2);
        vm.expectRevert(OilseedValueChain.BatchAlreadyExists.selector);
        chain.recordProcurement(BATCH, FARMER, h("p2"), CID);

        vm.prank(warehouse);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.recordProcurement(keccak256("b2"), FARMER, h("p3"), CID);
    }

    // ======================== CUSTODY & STAGES ========================

    function test_TransferCustody_OnlyCustodianToEligible() public {
        _procure();
        vm.prank(fpo2);
        vm.expectRevert(OilseedValueChain.NotCustodian.selector);
        chain.transferCustody(BATCH, warehouse, h("t"), CID);

        vm.startPrank(fpo);
        vm.expectRevert(OilseedValueChain.InvalidCustodian.selector);
        chain.transferCustody(BATCH, stranger, h("t"), CID);
        vm.expectRevert(OilseedValueChain.InvalidCustodian.selector);
        chain.transferCustody(BATCH, fpo, h("t"), CID);
        vm.expectRevert(OilseedValueChain.InvalidCustodian.selector);
        chain.transferCustody(BATCH, oracle, h("t"), CID); // oracle role is not a custodian role
        chain.transferCustody(BATCH, warehouse, h("t"), CID);
        vm.stopPrank();

        assertEq(chain.custodianOf(BATCH), warehouse);
    }

    function test_WarehouseAndLogistics_RequireRoleAndCustody() public {
        _procure();
        vm.prank(warehouse);
        vm.expectRevert(OilseedValueChain.NotCustodian.selector);
        chain.recordWarehouseInspection(BATCH, h("i"), CID);

        vm.prank(fpo);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.recordWarehouseInspection(BATCH, h("i"), CID);

        vm.prank(fpo);
        chain.transferCustody(BATCH, warehouse, h("t"), CID);
        vm.prank(warehouse);
        chain.recordWarehouseInspection(BATCH, h("i"), CID);
        assertEq(uint8(chain.getBatch(BATCH).stage), uint8(OilseedValueChain.Stage.Stored));

        vm.prank(warehouse);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.recordLogistics(BATCH, h("g"), CID);
    }

    function test_DuplicateRecordIsRejected() public {
        _procure();
        vm.prank(fpo);
        chain.transferCustody(BATCH, warehouse, h("t"), CID);
        vm.startPrank(warehouse);
        chain.recordWarehouseInspection(BATCH, h("same"), CID);
        vm.expectRevert(OilseedValueChain.AlreadyAnchored.selector);
        chain.recordWarehouseInspection(BATCH, h("same"), CID);
        vm.stopPrank();
    }

    function test_HeadChainCoversFullHistory() public {
        bytes32[] memory digests = new bytes32[](6);
        _registerFarmer();
        vm.prank(fpo);
        digests[0] = chain.recordProcurement(BATCH, FARMER, h("procurement"), CID);
        vm.prank(fpo);
        digests[1] = chain.transferCustody(BATCH, warehouse, h("to-warehouse"), CID);
        vm.prank(warehouse);
        digests[2] = chain.recordWarehouseInspection(BATCH, h("inspection"), CID);
        vm.prank(warehouse);
        digests[3] = chain.transferCustody(BATCH, logistics, h("to-logistics"), CID);
        vm.prank(logistics);
        digests[4] = chain.recordLogistics(BATCH, h("gps"), CID);
        vm.prank(logistics);
        digests[5] = chain.transferCustody(BATCH, processor, h("to-processor"), CID);

        bytes32 head;
        for (uint256 i = 0; i < digests.length; i++) {
            head = keccak256(abi.encode(head, digests[i]));
        }
        OilseedValueChain.Batch memory b = chain.getBatch(BATCH);
        assertEq(b.head, head);
        assertEq(b.recordCount, 6);
        assertEq(uint8(b.stage), uint8(OilseedValueChain.Stage.InTransit));
    }

    // ======================== PROCESSING ========================

    function test_ProcessBatch_ConsumesInputAndCreatesOutputs() public {
        bytes32 digest = _process();
        OilseedValueChain.Batch memory input = chain.getBatch(BATCH);
        assertEq(uint8(input.stage), uint8(OilseedValueChain.Stage.Consumed));

        OilseedValueChain.Batch memory oil = chain.getBatch(OIL);
        assertEq(oil.parentBatchId, BATCH);
        assertEq(oil.farmerId, FARMER);
        assertEq(oil.custodian, processor);
        assertEq(oil.head, keccak256(abi.encode(bytes32(0), digest)));
        assertEq(uint8(oil.stage), uint8(OilseedValueChain.Stage.Processed));
        assertTrue(chain.verifyRecord(digest, BATCH, chain.RT_PROCESSING()));

        vm.prank(processor);
        vm.expectRevert(OilseedValueChain.InvalidStage.selector);
        chain.transferCustody(BATCH, warehouse, h("late"), CID);
    }

    function test_ProcessBatch_RejectsBadOutputs() public {
        _toProcessor();
        bytes32[] memory none = new bytes32[](0);
        vm.prank(processor);
        vm.expectRevert(OilseedValueChain.InvalidOutputs.selector);
        chain.processBatch(BATCH, none, h("x"), CID);

        bytes32[] memory dup = new bytes32[](2);
        dup[0] = OIL;
        dup[1] = OIL;
        vm.prank(processor);
        vm.expectRevert(OilseedValueChain.BatchAlreadyExists.selector);
        chain.processBatch(BATCH, dup, h("x"), CID);

        bytes32[] memory tooMany = new bytes32[](17);
        for (uint256 i = 0; i < 17; i++) {
            tooMany[i] = bytes32(i + 1);
        }
        vm.prank(processor);
        vm.expectRevert(OilseedValueChain.InvalidOutputs.selector);
        chain.processBatch(BATCH, tooMany, h("x"), CID);
    }

    // ======================== PACKAGING ========================

    function test_CreateSku_AndVerifyUnits() public {
        _process();
        bytes32 l0 = chain.unitLeaf(SKU, "unit-0");
        bytes32 l1 = chain.unitLeaf(SKU, "unit-1");
        bytes32 l2 = chain.unitLeaf(SKU, "unit-2");
        bytes32 h01 = _hashPair(l0, l1);
        bytes32 root = _hashPair(h01, l2);

        vm.prank(processor);
        chain.createSku(SKU, OIL, root, 3, h("packaging"), CID);

        (bytes32 batchId, bytes32 parent, bytes32 farmerId, bytes32 unitsRoot, uint64 packagedAt) =
            chain.verifyPackageOrigin(SKU);
        assertEq(batchId, OIL);
        assertEq(parent, BATCH);
        assertEq(farmerId, FARMER);
        assertEq(unitsRoot, root);
        assertEq(packagedAt, block.timestamp);

        bytes32[] memory proof2 = new bytes32[](1);
        proof2[0] = h01;
        assertTrue(chain.verifyUnit(SKU, "unit-2", proof2));
        bytes32[] memory proof0 = new bytes32[](2);
        proof0[0] = l1;
        proof0[1] = l2;
        assertTrue(chain.verifyUnit(SKU, "unit-0", proof0));
        assertFalse(chain.verifyUnit(SKU, "unit-9", proof0));
    }

    function test_CreateSku_Reverts() public {
        _process();
        vm.prank(processor);
        vm.expectRevert(OilseedValueChain.NotAProductBatch.selector);
        chain.createSku(SKU, BATCH, h("root"), 1, h("p"), CID);

        vm.prank(processor);
        vm.expectRevert(OilseedValueChain.InvalidUnits.selector);
        chain.createSku(SKU, OIL, bytes32(0), 1, h("p"), CID);

        vm.prank(packager);
        vm.expectRevert(OilseedValueChain.NotCustodian.selector);
        chain.createSku(SKU, OIL, h("root"), 1, h("p"), CID);

        vm.startPrank(processor);
        chain.createSku(SKU, OIL, h("root"), 1, h("p"), CID);
        vm.expectRevert(OilseedValueChain.SKUAlreadyExists.selector);
        chain.createSku(SKU, OIL, h("root"), 1, h("p2"), CID);
        vm.stopPrank();

        vm.expectRevert(OilseedValueChain.SKUNotFound.selector);
        chain.verifyPackageOrigin(keccak256("missing"));
    }

    // ======================== FRAUD ========================

    function test_Fraud_ReportAndIndependentReview() public {
        _procure();
        vm.prank(stranger);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.reportFraud(BATCH, h("evidence"), CID);

        vm.prank(retailer);
        bytes32 report = chain.reportFraud(BATCH, h("evidence"), CID);
        assertEq(uint8(chain.fraudStatus(report)), uint8(OilseedValueChain.FraudStatus.Open));

        vm.prank(fpo);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.resolveFraud(report, true, h("resolution"), CID);

        vm.prank(admin);
        bytes32 resolution = chain.resolveFraud(report, true, h("resolution"), CID);
        assertEq(uint8(chain.fraudStatus(report)), uint8(OilseedValueChain.FraudStatus.Upheld));
        assertTrue(chain.verifyRecord(resolution, BATCH, chain.RT_FRAUD_RESOLUTION()));

        vm.prank(admin2);
        vm.expectRevert(OilseedValueChain.ReportNotOpen.selector);
        chain.resolveFraud(report, false, h("again"), CID);
    }

    function test_Fraud_AdminCannotResolveOwnReport() public {
        _procure();
        vm.prank(admin);
        bytes32 report = chain.reportFraud(BATCH, h("evidence"), CID);
        vm.prank(admin);
        vm.expectRevert(OilseedValueChain.CannotResolveOwnReport.selector);
        chain.resolveFraud(report, true, h("r"), CID);
        vm.prank(admin2);
        chain.resolveFraud(report, false, h("r"), CID);
        assertEq(uint8(chain.fraudStatus(report)), uint8(OilseedValueChain.FraudStatus.Dismissed));
    }

    // ======================== AI COMMIT–REVEAL ========================

    function test_AI_CommitRevealWindow() public {
        _procure();
        bytes32 content = h("assessment");
        bytes32 salt = h("salt");
        bytes32 commit = chain.computeAICommit(BATCH, content, salt, oracle);

        vm.prank(oracle);
        chain.commitAIAssessment(BATCH, commit);

        vm.prank(oracle);
        vm.expectRevert(OilseedValueChain.RevealTooEarly.selector);
        chain.revealAIAssessment(BATCH, content, salt, CID);

        vm.warp(block.timestamp + 1 hours);
        vm.prank(oracle);
        vm.expectRevert(OilseedValueChain.InvalidReveal.selector);
        chain.revealAIAssessment(BATCH, content, h("wrong"), CID);

        vm.prank(oracle);
        bytes32 digest = chain.revealAIAssessment(BATCH, content, salt, CID);
        assertTrue(chain.verifyRecord(digest, BATCH, chain.RT_AI_ASSESSMENT()));

        vm.prank(oracle);
        vm.expectRevert(OilseedValueChain.AlreadyRevealed.selector);
        chain.revealAIAssessment(BATCH, content, salt, CID);
    }

    function test_AI_ExpiredCommitCanBeReplaced() public {
        _procure();
        bytes32 commit = chain.computeAICommit(BATCH, h("a"), h("s"), oracle);
        vm.prank(oracle);
        chain.commitAIAssessment(BATCH, commit);

        vm.prank(oracle);
        vm.expectRevert(OilseedValueChain.AlreadyCommitted.selector);
        chain.commitAIAssessment(BATCH, commit);

        vm.warp(block.timestamp + 24 hours + 1);
        vm.prank(oracle);
        vm.expectRevert(OilseedValueChain.RevealTooLate.selector);
        chain.revealAIAssessment(BATCH, h("a"), h("s"), CID);

        bytes32 commit2 = chain.computeAICommit(BATCH, h("b"), h("s2"), oracle);
        vm.prank(oracle);
        chain.commitAIAssessment(BATCH, commit2);
        (bytes32 stored,,,) = chain.aiCommits(BATCH);
        assertEq(stored, commit2);
    }

    function test_AI_OnlyCommittingOracleCanReveal() public {
        _procure();
        address oracle2 = makeAddr("oracle2");
        uint256 role = chain.ROLE_AI_ORACLE();
        vm.prank(admin);
        chain.grantRole(oracle2, role);

        bytes32 commit = chain.computeAICommit(BATCH, h("a"), h("s"), oracle);
        vm.prank(oracle);
        chain.commitAIAssessment(BATCH, commit);
        vm.warp(block.timestamp + 2 hours);
        vm.prank(oracle2);
        vm.expectRevert(OilseedValueChain.Unauthorized.selector);
        chain.revealAIAssessment(BATCH, h("a"), h("s"), CID);
    }

    // ======================== DIGESTS (fuzz) ========================

    function testFuzz_RecordDigestIsDomainSeparated(uint8 typeA, uint8 typeB, bytes32 subject, bytes32 content)
        public
        view
    {
        vm.assume(typeA != typeB);
        bytes32 a = chain.computeRecordDigest(typeA, subject, content);
        assertEq(
            a, keccak256(abi.encode(chain.RECORD_TYPEHASH(), block.chainid, address(chain), typeA, subject, content))
        );
        assertTrue(a != chain.computeRecordDigest(typeB, subject, content));
    }

    function testFuzz_DigestBoundToRegistryAndChain(bytes32 subject, bytes32 content) public {
        bytes32 a = chain.computeRecordDigest(2, subject, content);
        vm.prank(admin);
        OilseedValueChain other = new OilseedValueChain();
        assertTrue(a != other.computeRecordDigest(2, subject, content));
        vm.chainId(block.chainid + 1);
        assertTrue(a != chain.computeRecordDigest(2, subject, content));
    }

    function testFuzz_RolesBitmap(uint256 role) public {
        role = bound(role, 1, chain.ALL_ROLES());
        vm.prank(admin);
        chain.grantRole(stranger, role);
        assertEq(chain.getRoles(stranger), role);
        assertTrue(chain.hasRole(stranger, role));
        vm.prank(admin);
        chain.revokeRole(stranger, role);
        assertEq(chain.getRoles(stranger), 0);
    }
}
