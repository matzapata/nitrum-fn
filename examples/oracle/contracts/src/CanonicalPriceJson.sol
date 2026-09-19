// SPDX-License-Identifier: MIT
pragma solidity ^0.8.26;

/// @notice Tiny parser/encoder for the oracle enclave's canonical JSON:
/// `{"ids":["eth"],"prices":[350012000000]}` (no whitespace).
library CanonicalPriceJson {
    error InvalidJson();
    error LengthMismatch();

    struct Payload {
        string[] ids;
        uint256[] prices;
    }

    function parse(bytes memory body) internal pure returns (Payload memory out) {
        uint256 i = 0;
        i = expect(body, i, '{"ids":[');
        (out.ids, i) = parseStringArray(body, i);
        i = expect(body, i, '],"prices":[');
        (out.prices, i) = parseUintArray(body, i);
        i = expect(body, i, "]}");
        if (i != body.length) revert InvalidJson();
        if (out.ids.length != out.prices.length) revert LengthMismatch();
        if (out.ids.length == 0) revert InvalidJson();
    }

    function encode(string[] memory ids, uint256[] memory prices) internal pure returns (bytes memory) {
        if (ids.length != prices.length) revert LengthMismatch();
        if (ids.length == 0) revert InvalidJson();

        bytes memory out = '{"ids":[';
        for (uint256 n = 0; n < ids.length; n++) {
            if (n > 0) out = bytes.concat(out, ",");
            out = bytes.concat(out, '"', bytes(ids[n]), '"');
        }
        out = bytes.concat(out, '],"prices":[');
        for (uint256 n = 0; n < prices.length; n++) {
            if (n > 0) out = bytes.concat(out, ",");
            out = bytes.concat(out, bytes(_uintToString(prices[n])));
        }
        return bytes.concat(out, "]}");
    }

    function _uintToString(uint256 value) private pure returns (string memory) {
        if (value == 0) return "0";
        uint256 temp = value;
        uint256 digits;
        while (temp != 0) {
            digits++;
            temp /= 10;
        }
        bytes memory buffer = new bytes(digits);
        while (value != 0) {
            digits -= 1;
            buffer[digits] = bytes1(uint8(48 + (value % 10)));
            value /= 10;
        }
        return string(buffer);
    }

    function parseStringArray(bytes memory body, uint256 i)
        private
        pure
        returns (string[] memory ids, uint256 next)
    {
        if (i >= body.length) revert InvalidJson();
        if (body[i] == "]") {
            string[] memory empty;
            return (empty, i);
        }

        uint256 count = 1;
        uint256 j = i;
        while (j < body.length && body[j] != "]") {
            if (body[j] == ",") count++;
            j++;
        }
        if (j >= body.length) revert InvalidJson();

        ids = new string[](count);
        for (uint256 n = 0; n < count; n++) {
            (ids[n], i) = parseQuotedString(body, i);
            if (n + 1 < count) {
                i = expect(body, i, ",");
            }
        }
        return (ids, i);
    }

    function parseUintArray(bytes memory body, uint256 i)
        private
        pure
        returns (uint256[] memory prices, uint256 next)
    {
        if (i >= body.length) revert InvalidJson();
        if (body[i] == "]") {
            uint256[] memory empty;
            return (empty, i);
        }

        uint256 count = 1;
        uint256 j = i;
        while (j < body.length && body[j] != "]") {
            if (body[j] == ",") count++;
            j++;
        }
        if (j >= body.length) revert InvalidJson();

        prices = new uint256[](count);
        for (uint256 n = 0; n < count; n++) {
            (prices[n], i) = parseUint(body, i);
            if (n + 1 < count) {
                i = expect(body, i, ",");
            }
        }
        return (prices, i);
    }

    function parseQuotedString(bytes memory body, uint256 i) private pure returns (string memory, uint256) {
        if (i >= body.length || body[i] != '"') revert InvalidJson();
        i++;
        uint256 start = i;
        while (i < body.length && body[i] != '"') {
            bytes1 c = body[i];
            bool ok = (c >= "a" && c <= "z") || (c >= "0" && c <= "9") || c == "-" || c == "_";
            if (!ok) revert InvalidJson();
            i++;
        }
        if (i >= body.length || start == i) revert InvalidJson();
        bytes memory s = new bytes(i - start);
        for (uint256 k = 0; k < s.length; k++) {
            s[k] = body[start + k];
        }
        return (string(s), i + 1);
    }

    function parseUint(bytes memory body, uint256 i) private pure returns (uint256 value, uint256 next) {
        if (i >= body.length || body[i] < "0" || body[i] > "9") revert InvalidJson();
        if (body[i] == "0") {
            uint256 n = i + 1;
            if (n < body.length && body[n] >= "0" && body[n] <= "9") revert InvalidJson();
            return (0, n);
        }
        while (i < body.length && body[i] >= "0" && body[i] <= "9") {
            value = value * 10 + uint8(body[i]) - 48;
            i++;
        }
        return (value, i);
    }

    function expect(bytes memory body, uint256 i, bytes memory needle) private pure returns (uint256) {
        if (i + needle.length > body.length) revert InvalidJson();
        for (uint256 k = 0; k < needle.length; k++) {
            if (body[i + k] != needle[k]) revert InvalidJson();
        }
        return i + needle.length;
    }
}
