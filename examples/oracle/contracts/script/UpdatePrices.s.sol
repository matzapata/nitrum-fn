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

/// @dev Exposes `_parseAttestation` for the submit scripts.
contract NitroValidatorScriptParser is NitroValidator {
    constructor() NitroValidator(ICertManager(address(1)), IP384Verifier(address(1))) {}

    function parseAttestation(bytes memory attestationTbs) external pure returns (Ptrs memory) {
        return _parseAttestation(attestationTbs);
    }
}

/// @dev Shared parse + P-384 hint helpers. Cert cache and `updatePrice` are split so
/// Alchemy's ~16M gas cap can use a high estimate multiplier on certs without
/// inflating `updatePrice` past the limit.
abstract contract AttestationSubmitBase is Script {
    using Asn1Decode for bytes;
    using CborDecode for bytes;
    using LibAsn1Ptr for Asn1Ptr;
    using LibBytes for bytes;
    using LibCborElement for CborElement;

    struct Loaded {
        NitrumOracle oracle;
        CertManager certManager;
        bytes attestation;
        bytes attestationTbs;
        bytes signature;
        NitroValidator.Ptrs ptrs;
    }

    function _load(bool needBody) internal returns (Loaded memory loaded, bytes memory body) {
        loaded.oracle = NitrumOracle(vm.envAddress("ORACLE_ADDRESS"));
        loaded.attestation = vm.envBytes("ATTESTATION_HEX");
        require(loaded.attestation.length > 0, "set ATTESTATION_HEX");
        if (needBody) {
            body = vm.envBytes("BODY_HEX");
            require(body.length > 0, "set BODY_HEX");
        }

        NitroValidatorScriptParser parser = new NitroValidatorScriptParser();
        (loaded.attestationTbs, loaded.signature) = parser.decodeAttestationTbs(loaded.attestation);
        loaded.ptrs = parser.parseAttestation(loaded.attestationTbs);

        NitroValidator nv = NitroValidator(address(loaded.oracle.validator()));
        loaded.certManager = CertManager(address(nv.certManager()));

        console2.log("oracle", address(loaded.oracle));
        console2.log("attestation bytes", loaded.attestation.length);
        if (needBody) {
            console2.log("body bytes", body.length);
        }
        console2.log("cabundle certs", loaded.ptrs.cabundle.length);
    }

    function _runColdHintedCache(Loaded memory loaded) internal returns (ICertManager.VerifiedCert memory leaf) {
        bytes memory rootCert = loaded.attestationTbs.slice(loaded.ptrs.cabundle[0]);
        bytes32 parentHash = keccak256(rootCert);
        ICertManager.VerifiedCert memory parent = loaded.certManager.loadVerified(parentHash);
        require(parent.pubKey.length > 0, "root not pinned");

        for (uint256 i = 1; i < loaded.ptrs.cabundle.length; ++i) {
            bytes memory caCert = loaded.attestationTbs.slice(loaded.ptrs.cabundle[i]);
            bytes memory hints = _certHints(caCert, parent.pubKey);
            parentHash = loaded.certManager.verifyCACertWithHints(caCert, parentHash, hints);
            parent = loaded.certManager.loadVerified(parentHash);
            require(parent.pubKey.length > 0, "CA not cached");
        }

        bytes memory clientCert = loaded.attestationTbs.slice(loaded.ptrs.cert);
        bytes memory clientHints = _certHints(clientCert, parent.pubKey);
        leaf = loaded.certManager.verifyClientCertWithHints(clientCert, parentHash, clientHints);
        require(leaf.pubKey.length > 0, "leaf not cached");
    }

    function _loadCachedLeaf(Loaded memory loaded) internal view returns (ICertManager.VerifiedCert memory leaf) {
        bytes memory clientCert = loaded.attestationTbs.slice(loaded.ptrs.cert);
        leaf = loaded.certManager.loadVerified(_certCacheKey(clientCert));
        require(leaf.pubKey.length > 0, "leaf not cached; run CacheCerts first");
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

/// @notice First leaf for this enclave: cache cabundle + client cert.
///
/// Alchemy under-estimates these txs; use `--gas-estimate-multiplier 200`.
/// Skip on a re-take when the leaf is already cached.
///
///   forge script script/UpdatePrices.s.sol:CacheCerts \
///     --rpc-url $BASE_SEPOLIA_RPC_URL --broadcast --ffi --offline \
///     --gas-estimate-multiplier 200
contract CacheCerts is AttestationSubmitBase {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        (Loaded memory loaded,) = _load(false);

        vm.startBroadcast(pk);
        _runColdHintedCache(loaded);
        console2.log("cert cold path done");
        vm.stopBroadcast();
    }
}

/// @notice Warm `updatePrice` after the leaf is cached.
///
/// Does **not** call the oracle in Foundry's EVM — local `modexp` metering exceeds
/// Alchemy's 16M cap. Writes `updatePrice.data` for `cast send --gas-limit 16000000`.
///
///   forge script script/UpdatePrices.s.sol:UpdatePrices --ffi --offline
///   cast send $ORACLE_ADDRESS $(cat updatePrice.data) \
///     --rpc-url $BASE_SEPOLIA_RPC_URL --private-key $PRIVATE_KEY --gas-limit 16000000
contract UpdatePrices is AttestationSubmitBase {
    function run() external {
        (Loaded memory loaded, bytes memory body) = _load(true);

        ICertManager.VerifiedCert memory leaf = _loadCachedLeaf(loaded);
        bytes memory hints = _attestationHints(loaded.attestation, leaf.pubKey);
        bytes memory data =
            abi.encodeCall(NitrumOracle.updatePrice, (loaded.attestationTbs, loaded.signature, hints, body));

        vm.writeFile("updatePrice.data", vm.toString(data));
        console2.log("wrote updatePrice.data, calldata bytes", data.length);
    }
}
