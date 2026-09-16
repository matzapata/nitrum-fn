// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Script, console2} from "forge-std/Script.sol";

import {NitrumOracle} from "../src/NitrumOracle.sol";

/// @notice Call `setEnclave(pcr0Hash, contentHash)` on a deployed NitrumOracle.
///
/// Env:
///   PRIVATE_KEY
///   ORACLE_ADDRESS
///   PCR0          48-byte hex → pcr0Hash = keccak256(PCR0)
///   CONTENT_HASH  0x-prefixed bytes32 (sha256 of oracle.wasm)
///                 from `nitrum-fn describe ./oracle.wasm` (`hash=` line)
///   # or pass pcr0Hash directly:
///   PCR0_HASH     0x-prefixed bytes32
///
///   forge script script/SetEnclave.s.sol:SetEnclave --rpc-url $BASE_SEPOLIA_RPC_URL --broadcast
contract SetEnclave is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        NitrumOracle oracle = NitrumOracle(vm.envAddress("ORACLE_ADDRESS"));

        bytes32 pcr0Hash = _pcr0Hash();
        bytes32 contentHash_ = vm.envBytes32("CONTENT_HASH");

        console2.log("oracle", address(oracle));
        console2.log("pcr0Hash");
        console2.logBytes32(pcr0Hash);
        console2.log("contentHash");
        console2.logBytes32(contentHash_);

        vm.startBroadcast(pk);
        oracle.setEnclave(pcr0Hash, contentHash_);
        vm.stopBroadcast();
    }

    function _pcr0Hash() internal view returns (bytes32) {
        if (vm.envExists("PCR0_HASH")) return vm.envBytes32("PCR0_HASH");
        require(vm.envExists("PCR0"), "set PCR0 (48-byte hex) or PCR0_HASH");
        bytes memory pcr0 = vm.parseBytes(vm.envString("PCR0"));
        require(pcr0.length == 48, "PCR0 must be 48 bytes");
        return keccak256(pcr0);
    }
}
