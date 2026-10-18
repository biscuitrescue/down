// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {OilseedValueChain} from "../src/OilseedValueChain.sol";
import {BatchProvenanceNFT} from "../src/BatchProvenanceNFT.sol";
import {FarmerCredential} from "../src/FarmerCredential.sol";

/// Deploys the registry and both NFT contracts and writes deployments/<chainId>.json.
/// Usage: PRIVATE_KEY=0x... forge script script/Deploy.s.sol --rpc-url $RPC_URL --broadcast
contract DeployScript is Script {
    function run() external {
        // Get the deployer's private key from environment variable
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");

        // Start broadcasting transactions
        vm.startBroadcast(deployerPrivateKey);

        OilseedValueChain registry = new OilseedValueChain();
        BatchProvenanceNFT provenance = new BatchProvenanceNFT(registry);
        FarmerCredential credential = new FarmerCredential(registry);

        vm.stopBroadcast();

        // Log the deployed contract addresses
        console.log("OilseedValueChain deployed at:", address(registry));
        console.log("BatchProvenanceNFT deployed at:", address(provenance));
        console.log("FarmerCredential deployed at:", address(credential));

        string memory key = "deployment";
        vm.serializeUint(key, "chainId", block.chainid);
        vm.serializeAddress(key, "deployer", vm.addr(deployerPrivateKey));
        vm.serializeAddress(key, "OilseedValueChain", address(registry));
        vm.serializeAddress(key, "BatchProvenanceNFT", address(provenance));
        string memory json = vm.serializeAddress(key, "FarmerCredential", address(credential));
        vm.writeJson(json, string.concat("./deployments/", vm.toString(block.chainid), ".json"));
    }
}
