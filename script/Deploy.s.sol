// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

import "forge-std/Script.sol";
import "../src/OilseedValueChain.sol";

contract DeployScript is Script {
    function run() external {
        // Get the deployer's private key from environment variable
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");

        // Start broadcasting transactions
        vm.startBroadcast(deployerPrivateKey);

        // Deploy the contract
        OilseedValueChain contract_ = new OilseedValueChain();

        // Log the deployed contract address
        console.log("OilseedValueChain deployed at:", address(contract_));

        vm.stopBroadcast();
    }
}
