// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Script, console2} from "forge-std/Script.sol";
import {Asn1Decode, Asn1Ptr, LibAsn1Ptr} from "@nitro-validator/Asn1Decode.sol";
import {CborDecode, CborElement, LibCborElement} from "@nitro-validator/CborDecode.sol";
import {CertManager} from "@nitro-validator/CertManager.sol";
import {ICertManager} from "@nitro-validator/ICertManager.sol";
import {IP384Verifier} from "@nitro-validator/IP384Verifier.sol";
import {LibBytes} from "@nitro-validator/LibBytes.sol";
import {NitroValidator} from "@nitro-validator/NitroValidator.sol";

import {NitrumOracle} from "../src/NitrumOracle.sol";

/// @dev Exposes `_parseAttestation` for the submit script.
contract NitroValidatorScriptParser is NitroValidator {
    constructor() NitroValidator(ICertManager(address(1)), IP384Verifier(address(1))) {}

    function parseAttestation(bytes memory attestationTbs) external pure returns (Ptrs memory) {
        return _parseAttestation(attestationTbs);
    }
}

/// @notice After a staging invoke: cold-cache certs (optional) and call `updatePrice`.
///
/// Requires `--ffi` (runs `lib/nitro-validator/tools/p384_hints.js`).
///
/// Env:
///   PRIVATE_KEY
///   ORACLE_ADDRESS
///   ATTESTATION_HEX   0x-prefixed COSE Sign1 (or omit and use capture/attestation.hex)
///   BODY_HEX          0x-prefixed canonical JSON body (or omit and use capture/body.hex)
///   CACHE_CERTS       optional bool, default true
///
///   forge script script/UpdatePrices.s.sol:UpdatePrices \
///     --rpc-url $BASE_SEPOLIA_RPC_URL --broadcast --ffi
contract UpdatePrices is Script {
    using Asn1Decode for bytes;
    using CborDecode for bytes;
    using LibAsn1Ptr for Asn1Ptr;
    using LibBytes for bytes;
    using LibCborElement for CborElement;

    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        NitrumOracle oracle = NitrumOracle(vm.envAddress("ORACLE_ADDRESS"));
        bytes memory attestation = _loadBytes("ATTESTATION_HEX", "capture/attestation.hex");
        bytes memory body = _loadBytes("BODY_HEX", "capture/body.hex");
        bool cacheCerts = vm.envOr("CACHE_CERTS", true);

        NitroValidatorScriptParser parser = new NitroValidatorScriptParser();
        (bytes memory attestationTbs, bytes memory signature) = parser.decodeAttestationTbs(attestation);
        NitroValidator.Ptrs memory ptrs = parser.parseAttestation(attestationTbs);

        NitroValidator nv = NitroValidator(address(oracle.validator()));
        CertManager certManager = CertManager(address(nv.certManager()));

        console2.log("oracle", address(oracle));
        console2.log("attestation bytes", attestation.length);
        console2.log("body bytes", body.length);
        console2.log("cabundle certs", ptrs.cabundle.length);

        vm.startBroadcast(pk);

        ICertManager.VerifiedCert memory leaf;
        if (cacheCerts) {
            leaf = _runColdHintedCache(certManager, attestationTbs, ptrs);
            console2.log("cert cold path done");
        } else {
            bytes memory clientCert = attestationTbs.slice(ptrs.cert);
            leaf = certManager.loadVerified(_certCacheKey(clientCert));
            require(leaf.pubKey.length > 0, "leaf not cached; set CACHE_CERTS=true");
        }

        bytes memory hints = _attestationHints(attestation, leaf.pubKey);
        oracle.updatePrice(attestationTbs, signature, hints, body);
        console2.log("updatePrice submitted");

        vm.stopBroadcast();
    }

    function _loadBytes(string memory envKey, string memory fallbackPath) internal view returns (bytes memory) {
        try vm.envBytes(envKey) returns (bytes memory fromEnv) {
            if (fromEnv.length > 0) return fromEnv;
        } catch {}
        string memory raw = vm.readFile(fallbackPath);
        return vm.parseBytes(raw);
    }

    function _runColdHintedCache(
        CertManager certManager,
        bytes memory attestationTbs,
        NitroValidator.Ptrs memory ptrs
    ) internal returns (ICertManager.VerifiedCert memory leaf) {
        bytes memory rootCert = attestationTbs.slice(ptrs.cabundle[0]);
        bytes32 parentHash = keccak256(rootCert);
        ICertManager.VerifiedCert memory parent = certManager.loadVerified(parentHash);
        require(parent.pubKey.length > 0, "root not pinned");

        for (uint256 i = 1; i < ptrs.cabundle.length; ++i) {
            bytes memory caCert = attestationTbs.slice(ptrs.cabundle[i]);
            bytes memory hints = _certHints(caCert, parent.pubKey);
            parentHash = certManager.verifyCACertWithHints(caCert, parentHash, hints);
            parent = certManager.loadVerified(parentHash);
            require(parent.pubKey.length > 0, "CA not cached");
        }

        bytes memory clientCert = attestationTbs.slice(ptrs.cert);
        bytes memory clientHints = _certHints(clientCert, parent.pubKey);
        leaf = certManager.verifyClientCertWithHints(clientCert, parentHash, clientHints);
        require(leaf.pubKey.length > 0, "leaf not cached");
    }

    function _certCacheKey(bytes memory certificate) internal pure returns (bytes32) {
        Asn1Ptr root = certificate.root();
        Asn1Ptr tbsCertPtr = certificate.firstChildOf(root);
        return certificate.keccak(tbsCertPtr.header(), tbsCertPtr.totalLength());
    }

    function _certHints(bytes memory cert, bytes memory parentPubKey) internal returns (bytes memory) {
        string[] memory command = new string[](7);
        command[0] = "node";
        command[1] = string.concat(vm.projectRoot(), "/lib/nitro-validator/tools/p384_hints.js");
        command[2] = "cert";
        command[3] = "--cert";
        command[4] = vm.toString(cert);
        command[5] = "--pubkey";
        command[6] = vm.toString(parentPubKey);
        return vm.ffi(command);
    }

    function _attestationHints(bytes memory attestation, bytes memory leafPubKey) internal returns (bytes memory) {
        string[] memory command = new string[](7);
        command[0] = "node";
        command[1] = string.concat(vm.projectRoot(), "/lib/nitro-validator/tools/p384_hints.js");
        command[2] = "attestation";
        command[3] = "--attestation";
        command[4] = vm.toString(attestation);
        command[5] = "--pubkey";
        command[6] = vm.toString(leafPubKey);
        return vm.ffi(command);
    }
}
