// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {CborElement, LibCborElement} from "@nitro-validator/CborDecode.sol";
import {NitroValidator} from "@nitro-validator/NitroValidator.sol";

import {INitroAttestationValidator} from "../src/INitroAttestationValidator.sol";

/// @dev Test double: returns CBOR pointers into a hand-built attestationTbs.
/// Layout: [0..48) PCR0, [48..112) user_data (64 bytes).
contract MockNitroValidator is INitroAttestationValidator {
    uint64 public timestampMs;
    bool public revertNext;

    function setTimestampMs(uint64 ts) external {
        timestampMs = ts;
    }

    function setRevertNext(bool v) external {
        revertNext = v;
    }

    function validateAttestationWithHints(bytes memory, bytes memory, bytes memory)
        external
        view
        returns (NitroValidator.Ptrs memory ptrs)
    {
        require(!revertNext, "mock revert");
        ptrs.timestamp = timestampMs;
        ptrs.pcrs = new CborElement[](32);
        ptrs.pcrs[0] = LibCborElement.toCborElement(0x40, 0, 48);
        for (uint256 i = 1; i < 32; i++) {
            ptrs.pcrs[i] = LibCborElement.toCborElement(0xf6, 0, 0);
        }
        ptrs.userData = LibCborElement.toCborElement(0x40, 48, 64);
    }
}
