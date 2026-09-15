// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {CborDecode, CborElement, LibCborElement} from "@nitro-validator/CborDecode.sol";
import {NitroValidator} from "@nitro-validator/NitroValidator.sol";

import {CanonicalPriceJson} from "./CanonicalPriceJson.sol";
import {INitroAttestationValidator} from "./INitroAttestationValidator.sol";

/// @notice Demo oracle: verify Nitro attestation (PCR0 + wasm hash + body hash), then store prices.
///
/// Host `user_data` (64 bytes): `sha256(wasm) || sha256(response body)`.
/// Body: `{"ids":["eth"],"prices":[350012000000]}` (USD × 1e8, no whitespace).
///
/// PRECONDITION: CertManager must already have the attestation cert chain cached
/// (cold path) before {updatePrice}.
contract NitrumOracle {
    using CborDecode for bytes;
    using LibCborElement for CborElement;

    uint256 public constant PRICE_DECIMALS = 8;

    INitroAttestationValidator public immutable validator;
    /// @dev keccak256 of the trusted 48-byte PCR0.
    bytes32 public immutable expectedPcr0Hash;
    /// @dev sha256 of the trusted oracle.wasm (first 32 bytes of user_data).
    bytes32 public immutable expectedWasmHash;
    uint256 public immutable maxAge;

    uint64 public lastTimestampMs;
    mapping(string => uint256) private _price;
    mapping(string => uint64) private _priceTimestampMs;

    event PriceUpdated(string id, uint256 price, uint64 timestampMs);

    error MissingPcr0();
    error InvalidPcr0();
    error DebugPcr0();
    error AttestationTooOld();
    error StaleTimestamp();
    error MissingUserData();
    error InvalidUserDataLength();
    error WasmHashMismatch();
    error BodyHashMismatch();
    error CanonicalBodyMismatch();
    error UnknownId();

    constructor(
        INitroAttestationValidator validator_,
        bytes32 expectedPcr0Hash_,
        bytes32 expectedWasmHash_,
        uint256 maxAge_
    ) {
        require(address(validator_) != address(0), "missing validator");
        require(expectedPcr0Hash_ != bytes32(0), "missing pcr0");
        require(expectedWasmHash_ != bytes32(0), "missing wasm hash");
        require(maxAge_ > 0, "missing max age");
        validator = validator_;
        expectedPcr0Hash = expectedPcr0Hash_;
        expectedWasmHash = expectedWasmHash_;
        maxAge = maxAge_;
    }

    /// @notice Verify attestation policy, then write attested prices.
    function updatePrice(
        bytes calldata attestationTbs,
        bytes calldata signature,
        bytes calldata attestationHints,
        bytes calldata body
    ) external {
        bytes memory tbs = attestationTbs;
        bytes memory bodyMem = body;

        NitroValidator.Ptrs memory ptrs =
            validator.validateAttestationWithHints(tbs, signature, attestationHints);

        _checkPcr0(tbs, ptrs);
        _checkFreshness(ptrs.timestamp);
        _checkUserData(tbs, ptrs, bodyMem);

        CanonicalPriceJson.Payload memory payload = CanonicalPriceJson.parse(bodyMem);
        bytes memory rebuilt = CanonicalPriceJson.encode(payload.ids, payload.prices);
        if (keccak256(rebuilt) != keccak256(bodyMem)) revert CanonicalBodyMismatch();

        lastTimestampMs = ptrs.timestamp;
        for (uint256 i = 0; i < payload.ids.length; i++) {
            _price[payload.ids[i]] = payload.prices[i];
            _priceTimestampMs[payload.ids[i]] = ptrs.timestamp;
            emit PriceUpdated(payload.ids[i], payload.prices[i], ptrs.timestamp);
        }
    }

    /// @notice Latest price for `id` (USD × 1e8). Reverts if never set.
    function getPrice(string calldata id) external view returns (uint256) {
        uint256 price = _price[id];
        if (price == 0 && _priceTimestampMs[id] == 0) revert UnknownId();
        return price;
    }

    /// @notice Latest price + attestation timestamp (ms) for `id`.
    function getPriceData(string calldata id) external view returns (uint256 price, uint64 timestampMs) {
        timestampMs = _priceTimestampMs[id];
        if (timestampMs == 0) revert UnknownId();
        return (_price[id], timestampMs);
    }

    /// @notice Whether `id` has ever been written.
    function hasPrice(string calldata id) external view returns (bool) {
        return _priceTimestampMs[id] != 0;
    }

    function _checkPcr0(bytes memory attestationTbs, NitroValidator.Ptrs memory ptrs) private view {
        if (ptrs.pcrs.length == 0 || ptrs.pcrs[0].isNull()) revert MissingPcr0();
        bytes memory pcr0 = attestationTbs.slice(ptrs.pcrs[0]);
        if (_isAllZero(pcr0)) revert DebugPcr0();
        if (keccak256(pcr0) != expectedPcr0Hash) revert InvalidPcr0();
    }

    function _checkFreshness(uint64 timestampMs) private view {
        if (timestampMs / 1000 + maxAge <= block.timestamp) revert AttestationTooOld();
        if (timestampMs <= lastTimestampMs) revert StaleTimestamp();
    }

    function _checkUserData(bytes memory attestationTbs, NitroValidator.Ptrs memory ptrs, bytes memory body)
        private
        view
    {
        if (ptrs.userData.isNull()) revert MissingUserData();
        bytes memory userData = attestationTbs.slice(ptrs.userData);
        if (userData.length != 64) revert InvalidUserDataLength();

        bytes32 wasmHash;
        bytes32 bodyHash;
        assembly ("memory-safe") {
            wasmHash := mload(add(userData, 32))
            bodyHash := mload(add(userData, 64))
        }
        if (wasmHash != expectedWasmHash) revert WasmHashMismatch();
        if (bodyHash != sha256(body)) revert BodyHashMismatch();
    }

    function _isAllZero(bytes memory data) private pure returns (bool) {
        for (uint256 i = 0; i < data.length; i++) {
            if (data[i] != 0) return false;
        }
        return true;
    }
}
