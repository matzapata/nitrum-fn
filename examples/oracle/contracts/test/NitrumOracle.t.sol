// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

import {Test} from "forge-std/Test.sol";

import {CanonicalPriceJson} from "../src/CanonicalPriceJson.sol";
import {NitrumOracle} from "../src/NitrumOracle.sol";
import {MockNitroValidator} from "./MockNitroValidator.sol";

contract NitrumOracleTest is Test {
    MockNitroValidator internal mock;
    NitrumOracle internal oracle;

    bytes32 internal pcr0Hash;
    bytes32 internal contentHash;
    bytes internal pcr0;
    bytes internal body;
    bytes internal attestationTbs;

    uint256 internal constant MAX_AGE = 60 minutes;

    function setUp() public {
        mock = new MockNitroValidator();

        pcr0 = hex"0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f30";
        require(pcr0.length == 48, "pcr0 len");
        pcr0Hash = keccak256(pcr0);

        contentHash = keccak256("oracle.wasm");
        body = bytes('{"ids":["eth"],"prices":[350012000000]}');

        bytes32 bodyHash = sha256(body);
        bytes memory userData = bytes.concat(abi.encodePacked(contentHash), abi.encodePacked(bodyHash));
        attestationTbs = bytes.concat(pcr0, userData);

        oracle = new NitrumOracle(mock, MAX_AGE);
        oracle.setEnclave(pcr0Hash, contentHash);
        mock.setTimestampMs(uint64(block.timestamp * 1000));
    }

    function test_revert_enclaveNotSet() public {
        NitrumOracle unset = new NitrumOracle(mock, MAX_AGE);
        vm.expectRevert(NitrumOracle.EnclaveNotSet.selector);
        unset.updatePrice(attestationTbs, "", "", body);
    }

    function test_setEnclave() public {
        assertEq(oracle.pcr0Hash(), pcr0Hash);
        assertEq(oracle.contentHash(), contentHash);

        bytes32 otherPcr0 = keccak256("other-pcr0");
        bytes32 otherContent = keccak256("other-wasm");
        oracle.setEnclave(otherPcr0, otherContent);
        assertEq(oracle.pcr0Hash(), otherPcr0);
        assertEq(oracle.contentHash(), otherContent);
    }

    function test_updatePrice_andGetters() public {
        oracle.updatePrice(attestationTbs, "", "", body);

        assertEq(oracle.getPrice("eth"), 350012000000);
        assertTrue(oracle.hasPrice("eth"));

        (uint256 price, uint64 ts) = oracle.getPriceData("eth");
        assertEq(price, 350012000000);
        assertGt(ts, 0);

        vm.expectRevert(NitrumOracle.UnknownId.selector);
        oracle.getPrice("btc");
    }

    function test_updatePrice_multipleIds() public {
        body = bytes('{"ids":["eth","btc"],"prices":[1,2]}');
        bytes32 bodyHash = sha256(body);
        bytes memory userData = bytes.concat(abi.encodePacked(contentHash), abi.encodePacked(bodyHash));
        attestationTbs = bytes.concat(pcr0, userData);

        oracle.updatePrice(attestationTbs, "", "", body);
        assertEq(oracle.getPrice("eth"), 1);
        assertEq(oracle.getPrice("btc"), 2);
    }

    function test_revert_wrongPcr0() public {
        bytes memory badPcr0 =
            hex"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        bytes32 bodyHash = sha256(body);
        bytes memory userData = bytes.concat(abi.encodePacked(contentHash), abi.encodePacked(bodyHash));
        bytes memory tbs = bytes.concat(badPcr0, userData);

        vm.expectRevert(NitrumOracle.InvalidPcr0.selector);
        oracle.updatePrice(tbs, "", "", body);
    }

    function test_revert_debugPcr0() public {
        bytes memory zeroPcr0 = new bytes(48);
        bytes32 bodyHash = sha256(body);
        bytes memory userData = bytes.concat(abi.encodePacked(contentHash), abi.encodePacked(bodyHash));
        bytes memory tbs = bytes.concat(zeroPcr0, userData);

        vm.expectRevert(NitrumOracle.DebugPcr0.selector);
        oracle.updatePrice(tbs, "", "", body);
    }

    function test_revert_contentHashMismatch() public {
        bytes32 wrongContent = keccak256("other.wasm");
        bytes32 bodyHash = sha256(body);
        bytes memory userData = bytes.concat(abi.encodePacked(wrongContent), abi.encodePacked(bodyHash));
        bytes memory tbs = bytes.concat(pcr0, userData);

        vm.expectRevert(NitrumOracle.ContentHashMismatch.selector);
        oracle.updatePrice(tbs, "", "", body);
    }

    function test_revert_bodyHashMismatch() public {
        bytes memory otherBody = bytes('{"ids":["eth"],"prices":[1]}');
        vm.expectRevert(NitrumOracle.BodyHashMismatch.selector);
        oracle.updatePrice(attestationTbs, "", "", otherBody);
    }

    function test_revert_staleTimestamp() public {
        oracle.updatePrice(attestationTbs, "", "", body);
        vm.expectRevert(NitrumOracle.StaleTimestamp.selector);
        oracle.updatePrice(attestationTbs, "", "", body);
    }

    function test_revert_attestationTooOld() public {
        vm.warp(block.timestamp + MAX_AGE + 100);
        mock.setTimestampMs(uint64((block.timestamp - MAX_AGE - 1) * 1000));
        vm.expectRevert(NitrumOracle.AttestationTooOld.selector);
        oracle.updatePrice(attestationTbs, "", "", body);
    }

    function test_canonicalRoundTrip() public pure {
        string[] memory ids = new string[](2);
        ids[0] = "eth";
        ids[1] = "btc";
        uint256[] memory prices = new uint256[](2);
        prices[0] = 350012000000;
        prices[1] = 9500000000000;
        bytes memory encoded = CanonicalPriceJson.encode(ids, prices);
        assertEq(string(encoded), '{"ids":["eth","btc"],"prices":[350012000000,9500000000000]}');

        CanonicalPriceJson.Payload memory p = CanonicalPriceJson.parse(encoded);
        assertEq(p.ids.length, 2);
        assertEq(p.ids[0], "eth");
        assertEq(p.prices[1], 9500000000000);
    }
}
