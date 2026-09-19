// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {CborDecode, CborElement, LibCborElement} from "@nitro-validator/CborDecode.sol";
import {NitroValidator} from "@nitro-validator/NitroValidator.sol";

import {CanonicalPriceJson} from "./CanonicalPriceJson.sol";
import {INitroAttestationValidator} from "./INitroAttestationValidator.sol";

/// @notice Demo oracle: verify Nitro attestation (PCR0 + content hash + body hash), then store prices.
///
/// Host `user_data` (64 bytes): `sha256(wasm) || sha256(response body)`.
/// Body: `{"ids":["eth"],"prices":[350012000000]}` (USD × 1e8, no whitespace).
///
/// Pin the trusted enclave via {setEnclave} before {updatePrice}.
/// Do **not** call the content-hash pin "PCR1" — AWS PCR1 is a different Nitro measurement;
/// this value is the guest `.wasm` content hash from nitrum-fn `user_data`.
///
/// PRECONDITION: CertManager must already have the attestation cert chain cached
/// (cold path) before {updatePrice}.
contract NitrumOracle {
    using CborDecode for bytes;
    using LibCborElement for CborElement;

    uint256 public constant PRICE_DECIMALS = 8;

    INitroAttestationValidator public immutable validator;
    uint256 public immutable maxAge;

    address public owner;
    /// @dev keccak256 of the trusted 48-byte Nitro PCR0 (EIF measurement).
    bytes32 public pcr0Hash;
    /// @dev sha256 of the trusted oracle.wasm (first 32 bytes of attestation user_data).
    bytes32 public contentHash;

    uint64 public lastTimestampMs;
    mapping(string => uint256) private _price;
    mapping(string => uint64) private _priceTimestampMs;

    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event EnclaveSet(bytes32 pcr0Hash, bytes32 contentHash);
    event PriceUpdated(string id, uint256 price, uint64 timestampMs);

    error NotOwner();
    error EnclaveNotSet();
    error MissingPcr0();
    error InvalidPcr0();
    error DebugPcr0();
    error AttestationTooOld();
    error StaleTimestamp();
    error MissingUserData();
    error InvalidUserDataLength();
    error ContentHashMismatch();
    error BodyHashMismatch();
    error CanonicalBodyMismatch();
    error UnknownId();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    constructor(INitroAttestationValidator validator_, uint256 maxAge_) {
        require(address(validator_) != address(0), "missing validator");
        require(maxAge_ > 0, "missing max age");
        validator = validator_;
        maxAge = maxAge_;
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
    }

    /// @notice Pin the trusted Nitro image (PCR0) and guest wasm content hash.
    /// @param pcr0Hash_ `keccak256` of the 48-byte PCR0 from `nitrum build` / eif.json.
    /// @param contentHash_ `sha256` of the published `.wasm` (matches `x-nitrum-fn-shasum` / user_data[0:32]).
    function setEnclave(bytes32 pcr0Hash_, bytes32 contentHash_) external onlyOwner {
        require(pcr0Hash_ != bytes32(0), "missing pcr0");
        require(contentHash_ != bytes32(0), "missing content hash");
        pcr0Hash = pcr0Hash_;
        contentHash = contentHash_;
        emit EnclaveSet(pcr0Hash_, contentHash_);
    }

    function transferOwnership(address newOwner) external onlyOwner {
        require(newOwner != address(0), "zero owner");
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }

    /// @notice Verify attestation policy, then write attested prices.
    function updatePrice(
        bytes calldata attestationTbs,
        bytes calldata signature,
        bytes calldata attestationHints,
        bytes calldata body
    ) external {
        if (pcr0Hash == bytes32(0) || contentHash == bytes32(0)) revert EnclaveNotSet();

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
        if (keccak256(pcr0) != pcr0Hash) revert InvalidPcr0();
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

        bytes32 gotContentHash;
        bytes32 bodyHash;
        assembly ("memory-safe") {
            gotContentHash := mload(add(userData, 32))
            bodyHash := mload(add(userData, 64))
        }
        if (gotContentHash != contentHash) revert ContentHashMismatch();
        if (bodyHash != sha256(body)) revert BodyHashMismatch();
    }

    function _isAllZero(bytes memory data) private pure returns (bool) {
        for (uint256 i = 0; i < data.length; i++) {
            if (data[i] != 0) return false;
        }
        return true;
    }
}
