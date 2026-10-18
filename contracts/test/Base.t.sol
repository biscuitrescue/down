// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {OilseedValueChain} from "../src/OilseedValueChain.sol";
import {BatchProvenanceNFT} from "../src/BatchProvenanceNFT.sol";
import {FarmerCredential} from "../src/FarmerCredential.sol";

abstract contract BaseTest is Test {
    OilseedValueChain internal chain;
    BatchProvenanceNFT internal provenance;
    FarmerCredential internal credential;

    address internal admin = makeAddr("admin");
    address internal admin2 = makeAddr("admin2");
    address internal fpo = makeAddr("fpo");
    address internal fpo2 = makeAddr("fpo2");
    address internal warehouse = makeAddr("warehouse");
    address internal logistics = makeAddr("logistics");
    address internal processor = makeAddr("processor");
    address internal packager = makeAddr("packager");
    address internal oracle = makeAddr("oracle");
    address internal retailer = makeAddr("retailer");
    address internal stranger = makeAddr("stranger");

    bytes32 internal constant FARMER = keccak256("farmer-1");
    bytes32 internal constant BATCH = keccak256("batch-1");
    bytes32 internal constant OIL = keccak256("batch-1/oil");
    bytes32 internal constant CAKE = keccak256("batch-1/cake");
    bytes32 internal constant SKU = keccak256("sku-1");
    string internal constant CID = "bafkreigh2akiscaildcqabsyg3dfr6chu3fgpregiymsck7e7aqa4s52zy";

    function setUp() public virtual {
        vm.startPrank(admin);
        chain = new OilseedValueChain();
        provenance = new BatchProvenanceNFT(chain);
        credential = new FarmerCredential(chain);
        chain.grantRole(admin2, chain.ROLE_ADMIN());
        chain.grantRole(fpo, chain.ROLE_FPO());
        chain.grantRole(fpo2, chain.ROLE_FPO());
        chain.grantRole(warehouse, chain.ROLE_WAREHOUSE());
        chain.grantRole(logistics, chain.ROLE_LOGISTICS());
        chain.grantRole(processor, chain.ROLE_PROCESSOR() | chain.ROLE_PACKAGER());
        chain.grantRole(packager, chain.ROLE_PACKAGER());
        chain.grantRole(oracle, chain.ROLE_AI_ORACLE());
        chain.grantRole(retailer, chain.ROLE_RETAILER());
        vm.stopPrank();
    }

    function h(string memory s) internal pure returns (bytes32) {
        return keccak256(bytes(s));
    }

    function _registerFarmer() internal returns (bytes32 digest) {
        vm.prank(fpo);
        digest = chain.registerFarmer(FARMER, h("registration"), CID);
    }

    function _procure() internal returns (bytes32 digest) {
        _registerFarmer();
        vm.prank(fpo);
        digest = chain.recordProcurement(BATCH, FARMER, h("procurement"), CID);
    }

    function _toProcessor() internal {
        _procure();
        vm.prank(fpo);
        chain.transferCustody(BATCH, warehouse, h("to-warehouse"), CID);
        vm.prank(warehouse);
        chain.recordWarehouseInspection(BATCH, h("inspection"), CID);
        vm.prank(warehouse);
        chain.transferCustody(BATCH, logistics, h("to-logistics"), CID);
        vm.prank(logistics);
        chain.recordLogistics(BATCH, h("gps"), CID);
        vm.prank(logistics);
        chain.transferCustody(BATCH, processor, h("to-processor"), CID);
    }

    function _process() internal returns (bytes32 digest) {
        _toProcessor();
        bytes32[] memory outs = new bytes32[](2);
        outs[0] = OIL;
        outs[1] = CAKE;
        vm.prank(processor);
        digest = chain.processBatch(BATCH, outs, h("lab-report"), CID);
    }

    function _hashPair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a < b ? keccak256(abi.encode(a, b)) : keccak256(abi.encode(b, a));
    }
}
