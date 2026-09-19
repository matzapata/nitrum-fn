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

        console2.log("parsing attestation");
        NitroValidatorScriptParser parser = new NitroValidatorScriptParser();
        (loaded.attestationTbs, loaded.signature) = parser.decodeAttestationTbs(loaded.attestation);
        loaded.ptrs = parser.parseAttestation(loaded.attestationTbs);

        console2.log("reading CertManager");
        NitroValidator nv = NitroValidator(address(loaded.oracle.validator()));
        loaded.certManager = CertManager(address(nv.certManager()));

        console2.log("oracle", address(loaded.oracle));
        console2.log("attestation bytes", loaded.attestation.length);
        if (needBody) {
            console2.log("body bytes", body.length);
        }
        console2.log("cabundle certs", loaded.ptrs.cabundle.length);
    }

    function _cached(CertManager certManager, bytes memory cert)
        internal
        view
        returns (ICertManager.VerifiedCert memory)
    {
        return certManager.loadVerified(_certCacheKey(cert));
    }

    function _writeCacheTx(uint256 i, bytes memory data) internal {
        vm.writeFile(string.concat("cacheCerts.", vm.toString(i), ".data"), vm.toString(data));
    }

    /// @dev Writes `cacheCerts.N.data` for uncached cabundle + leaf. Parent keys for the next
    ///      cert come from `loadVerified` (already cached) or the cert SPKI — never from simulating
    ///      `verify*WithHints` on the fork (Foundry MODEXP hangs for minutes). Does not broadcast:
    ///      cached-CA txs are cheap execution + huge calldata, so Foundry's gas limit falls below
    ///      the EIP-7623 floor.
    function _dumpColdCache(Loaded memory loaded) internal returns (uint256 n) {
        vm.writeFile("cacheCerts.to", vm.toString(address(loaded.certManager)));

        bytes memory clientCert = loaded.attestationTbs.slice(loaded.ptrs.cert);
        if (_cached(loaded.certManager, clientCert).pubKey.length > 0) {
            console2.log("leaf already cached");
            return 0;
        }

        bytes memory rootCert = loaded.attestationTbs.slice(loaded.ptrs.cabundle[0]);
        bytes32 parentHash = keccak256(rootCert);
        bytes memory parentPubKey = loaded.certManager.loadVerified(parentHash).pubKey;
        require(parentPubKey.length > 0, "root not pinned");

        for (uint256 i = 1; i < loaded.ptrs.cabundle.length; ++i) {
            bytes memory caCert = loaded.attestationTbs.slice(loaded.ptrs.cabundle[i]);
            ICertManager.VerifiedCert memory cached = _cached(loaded.certManager, caCert);
            if (cached.pubKey.length > 0) {
                parentHash = _certCacheKey(caCert);
                parentPubKey = cached.pubKey;
                console2.log("skip cached CA", i);
                continue;
            }
            console2.log("P-384 hints for CA", i);
            bytes memory hints = _certHints(caCert, parentPubKey);
            _writeCacheTx(n++, abi.encodeCall(CertManager.verifyCACertWithHints, (caCert, parentHash, hints)));
            parentHash = _certCacheKey(caCert);
            parentPubKey = _certSubjectPubKey(caCert);
        }

        console2.log("P-384 hints for leaf");
        bytes memory clientHints = _certHints(clientCert, parentPubKey);
        _writeCacheTx(
            n++, abi.encodeCall(CertManager.verifyClientCertWithHints, (clientCert, parentHash, clientHints))
        );
    }

    /// @dev Uncompressed P-384 subject key (96 bytes) from X.509 SPKI. Same layout as
    ///      CertManager._parsePubKey — ASN.1 only, no signature verify.
    function _certSubjectPubKey(bytes memory certificate) internal pure returns (bytes memory) {
        Asn1Ptr ptr = certificate.firstChildOf(certificate.root()); // TBS
        ptr = certificate.firstChildOf(ptr); // version
        ptr = certificate.nextSiblingOf(ptr); // serial
        ptr = certificate.nextSiblingOf(ptr); // sigAlgo
        ptr = certificate.nextSiblingOf(ptr); // issuer
        ptr = certificate.nextSiblingOf(ptr); // validity
        ptr = certificate.nextSiblingOf(ptr); // subject
        ptr = certificate.nextSiblingOf(ptr); // SPKI
        ptr = certificate.nextSiblingOf(certificate.firstChildOf(ptr)); // BIT STRING
        ptr = certificate.bitstring(ptr);
        uint256 start = ptr.content();
        require(ptr.length() == 97 && certificate[start] == 0x04, "bad cert pubkey");
        return certificate.slice(start + 1, 96);
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

/// @notice First leaf for this enclave: dump cabundle + client cert calldata (no local P-384).
///
/// Do **not** `forge script --broadcast` these. A cached CA is a no-op with ~28KB calldata;
/// local gas then sits under Base Sepolia's EIP-7623 floor (`intrinsic gas too low`).
/// Send each `cacheCerts.N.data` with `cast send --gas-limit 16000000`, same as `updatePrice`.
///
///   forge script script/UpdatePrices.s.sol:CacheCerts --ffi --rpc-url $BASE_SEPOLIA_RPC_URL
///   cast send $(cat cacheCerts.to) $(cat cacheCerts.0.data) \
///     --rpc-url $BASE_SEPOLIA_RPC_URL --private-key $PRIVATE_KEY --gas-limit 16000000
contract CacheCerts is AttestationSubmitBase {
    function run() external {
        (Loaded memory loaded,) = _load(false);
        uint256 n = _dumpColdCache(loaded);
        console2.log("wrote cacheCerts.*.data count", n);
    }
}

/// @notice Warm `updatePrice` after the leaf is cached.
///
/// Does **not** broadcast `updatePrice` — local `modexp` metering exceeds Alchemy's
/// 16M cap. Still needs `--rpc-url` to read `oracle.validator()` and the cached leaf.
/// Writes `updatePrice.data` for `cast send --gas-limit 16000000`.
///
///   forge script script/UpdatePrices.s.sol:UpdatePrices --ffi \
///     --rpc-url $BASE_SEPOLIA_RPC_URL
///   cast send $ORACLE_ADDRESS $(cat updatePrice.data) \
///     --rpc-url $BASE_SEPOLIA_RPC_URL --private-key $PRIVATE_KEY --gas-limit 16000000
contract UpdatePrices is AttestationSubmitBase {
    function run() external {
        (Loaded memory loaded, bytes memory body) = _load(true);

        ICertManager.VerifiedCert memory leaf = _loadCachedLeaf(loaded);
        console2.log("P-384 hints for attestation");
        bytes memory hints = _attestationHints(loaded.attestation, leaf.pubKey);
        bytes memory data =
            abi.encodeCall(NitrumOracle.updatePrice, (loaded.attestationTbs, loaded.signature, hints, body));

        vm.writeFile("updatePrice.data", vm.toString(data));
        console2.log("wrote updatePrice.data, calldata bytes", data.length);
    }
}
