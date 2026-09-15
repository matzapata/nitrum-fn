// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Script, console2} from "forge-std/Script.sol";
import {CertManager} from "@nitro-validator/CertManager.sol";
import {NitroValidator} from "@nitro-validator/NitroValidator.sol";
import {P384Verifier} from "@nitro-validator/P384Verifier.sol";

import {INitroAttestationValidator} from "../src/INitroAttestationValidator.sol";
import {NitrumOracle} from "../src/NitrumOracle.sol";

/// @notice Deploy to Base Sepolia: P384Verifier → CertManager → NitroValidator → NitrumOracle.
///
/// Env:
///   PRIVATE_KEY
///   EXPECTED_PCR0_HASH   keccak256 of the 48-byte PCR0
///   EXPECTED_WASM_HASH   sha256 of the published oracle.wasm
///   MAX_AGE_SECONDS      optional, default 3600
///   CERT_OWNER / CERT_REVOKER  optional, default broadcaster
///   EXISTING_VALIDATOR   optional — reuse a deployed NitroValidator
///
///   forge script script/Deploy.s.sol:Deploy --rpc-url $BASE_SEPOLIA_RPC_URL --broadcast
contract Deploy is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        address broadcaster = vm.addr(pk);

        bytes32 pcr0Hash = vm.envBytes32("EXPECTED_PCR0_HASH");
        bytes32 wasmHash = vm.envBytes32("EXPECTED_WASM_HASH");
        uint256 maxAge = vm.envOr("MAX_AGE_SECONDS", uint256(3600));

        vm.startBroadcast(pk);

        INitroAttestationValidator validator;
        address existing = vm.envOr("EXISTING_VALIDATOR", address(0));
        if (existing != address(0)) {
            validator = INitroAttestationValidator(existing);
            console2.log("using existing NitroValidator", existing);
        } else {
            address owner = vm.envOr("CERT_OWNER", broadcaster);
            address revoker = vm.envOr("CERT_REVOKER", broadcaster);

            P384Verifier p384 = new P384Verifier();
            CertManager certManager = new CertManager(p384, owner, revoker);
            NitroValidator nv = new NitroValidator(certManager, p384);
            validator = INitroAttestationValidator(address(nv));

            console2.log("P384Verifier", address(p384));
            console2.log("CertManager", address(certManager));
            console2.log("NitroValidator", address(nv));
        }

        NitrumOracle oracle = new NitrumOracle(validator, pcr0Hash, wasmHash, maxAge);
        console2.log("NitrumOracle", address(oracle));
        console2.logBytes32(oracle.expectedPcr0Hash());
        console2.logBytes32(oracle.expectedWasmHash());

        vm.stopBroadcast();
    }
}
