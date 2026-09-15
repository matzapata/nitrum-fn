// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Script, console2} from "forge-std/Script.sol";

import {NitrumOracle} from "../src/NitrumOracle.sol";

/// @notice Call `setEnclave(pcr0Hash, contentHash)` on a deployed NitrumOracle.
///
/// Env:
///   PRIVATE_KEY
///   ORACLE_ADDRESS
///   PCR0         48-byte hex → pcr0Hash = keccak256(PCR0)
///   WASM_PATH    path to oracle.wasm → contentHash = sha256(file)
///   # or pass hashes directly:
///   PCR0_HASH / CONTENT_HASH  (0x-prefixed bytes32)
///
///   forge script script/SetEnclave.s.sol:SetEnclave --rpc-url $BASE_SEPOLIA_RPC_URL --broadcast
contract SetEnclave is Script {
    string internal constant DEFAULT_WASM_PATH =
        "../enclave/target/wasm32-unknown-unknown/release/oracle.wasm";

    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        NitrumOracle oracle = NitrumOracle(vm.envAddress("ORACLE_ADDRESS"));

        bytes32 pcr0Hash = _pcr0Hash();
        bytes32 contentHash_ = _contentHash();

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

    function _contentHash() internal view returns (bytes32) {
        if (vm.envExists("CONTENT_HASH")) return vm.envBytes32("CONTENT_HASH");
        string memory wasmPath = vm.envOr("WASM_PATH", DEFAULT_WASM_PATH);
        bytes memory wasm = vm.readFileBinary(wasmPath);
        require(wasm.length > 0, "empty wasm; build enclave or set CONTENT_HASH / WASM_PATH");
        return sha256(wasm);
    }
}
