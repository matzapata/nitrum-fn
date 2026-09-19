// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {NitroValidator} from "@nitro-validator/NitroValidator.sol";

/// @notice Minimal surface NitrumOracle needs from nitro-validator (real or mock).
interface INitroAttestationValidator {
    function validateAttestationWithHints(
        bytes memory attestationTbs,
        bytes memory signature,
        bytes memory attestationHints
    ) external returns (NitroValidator.Ptrs memory);
}
