// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {OilseedValueChain} from "../src/OilseedValueChain.sol";

/// Drives random custody/inspection/logistics sequences on one batch and tracks the digests it anchored.
contract BatchHandler is Test {
    OilseedValueChain public chain;
    bytes32 public constant BATCH = keccak256("inv-batch");
    address[3] public actors;
    bytes32[] public digests;
    uint256 private nonce;

    constructor(OilseedValueChain chain_, address fpo, address warehouse, address logistics) {
        chain = chain_;
        actors = [fpo, warehouse, logistics];
    }

    function digestCount() external view returns (uint256) {
        return digests.length;
    }

    function pushDigest(bytes32 d) external {
        digests.push(d);
    }

    function transfer(uint256 toSeed) external {
        address custodian = chain.custodianOf(BATCH);
        address to = actors[toSeed % 3];
        if (to == custodian) return;
        vm.prank(custodian);
        digests.push(chain.transferCustody(BATCH, to, keccak256(abi.encode("t", ++nonce)), "bafkreicid"));
    }

    function inspect() external {
        address custodian = chain.custodianOf(BATCH);
        if (custodian != actors[1]) return;
        vm.prank(custodian);
        digests.push(chain.recordWarehouseInspection(BATCH, keccak256(abi.encode("i", ++nonce)), "bafkreicid"));
    }

    function move() external {
        address custodian = chain.custodianOf(BATCH);
        if (custodian != actors[2]) return;
        vm.prank(custodian);
        digests.push(chain.recordLogistics(BATCH, keccak256(abi.encode("l", ++nonce)), "bafkreicid"));
    }
}

contract HeadChainInvariantTest is Test {
    OilseedValueChain internal chain;
    BatchHandler internal handler;

    function setUp() public {
        address fpo = makeAddr("fpo");
        address warehouse = makeAddr("warehouse");
        address logistics = makeAddr("logistics");
        chain = new OilseedValueChain();
        chain.grantRole(fpo, chain.ROLE_FPO());
        chain.grantRole(warehouse, chain.ROLE_WAREHOUSE());
        chain.grantRole(logistics, chain.ROLE_LOGISTICS());

        handler = new BatchHandler(chain, fpo, warehouse, logistics);
        vm.startPrank(fpo);
        chain.registerFarmer(keccak256("f"), keccak256("reg"), "bafkreicid");
        bytes32 d = chain.recordProcurement(handler.BATCH(), keccak256("f"), keccak256("proc"), "bafkreicid");
        vm.stopPrank();
        handler.pushDigest(d);

        targetContract(address(handler));
        bytes4[] memory selectors = new bytes4[](3);
        selectors[0] = BatchHandler.transfer.selector;
        selectors[1] = BatchHandler.inspect.selector;
        selectors[2] = BatchHandler.move.selector;
        targetSelector(FuzzSelector({addr: address(handler), selectors: selectors}));
    }

    /// forge-config: default.invariant.runs = 64
    /// forge-config: default.invariant.depth = 64
    /// The stored head always equals the hash chain over every anchored digest, in order.
    function invariant_HeadMatchesAnchoredHistory() public view {
        uint256 n = handler.digestCount();
        bytes32 head;
        for (uint256 i = 0; i < n; i++) {
            bytes32 d = handler.digests(i);
            head = keccak256(abi.encode(head, d));
            (bytes32 subject,,,) = chain.anchors(d);
            assertEq(subject, handler.BATCH());
        }
        OilseedValueChain.Batch memory b = chain.getBatch(handler.BATCH());
        assertEq(b.head, head);
        assertEq(b.recordCount, n);
    }
}
