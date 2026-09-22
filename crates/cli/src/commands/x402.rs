//! EIP-3009 signature for an x402 `exact` USDC authorization.
//! The CLI sends an invoke count. The USDC value comes from the API challenge.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use k256::ecdsa::{RecoveryId, SigningKey, VerifyingKey};
use sha3::{Digest, Keccak256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeAccept {
    pub amount: u64,
    pub pay_to: String,
    pub asset: String,
    pub network: String,
}

pub fn parse_challenge(body: &str) -> Result<ChallengeAccept> {
    let value: serde_json::Value = serde_json::from_str(body).context("payment challenge")?;
    let accept = value
        .get("accepts")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .context("challenge missing accepts")?;
    let amount = accept
        .get("maxAmountRequired")
        .and_then(|v| v.as_str())
        .context("challenge missing amount")?
        .parse()
        .context("challenge amount")?;
    Ok(ChallengeAccept {
        amount,
        pay_to: json_str(accept, "payTo")?,
        asset: json_str(accept, "asset")?,
        network: json_str(accept, "network")?,
    })
}

fn json_str(value: &serde_json::Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .with_context(|| format!("challenge missing {key}"))
}

/// Build a `PAYMENT-SIGNATURE` header. `amount` is the challenge's atomic USDC, not a CLI field.
pub fn payment_signature(
    private_key_hex: &str,
    accept: &ChallengeAccept,
    chain_id: u64,
    valid_before: u64,
) -> Result<String> {
    let signing = signing_key(private_key_hex)?;
    let from = address_of(signing.verifying_key());
    let nonce = random_bytes32()?;
    let digest = eip712_digest(
        &from,
        &accept.pay_to,
        accept.amount,
        valid_before,
        &nonce,
        chain_id,
        &accept.asset,
    )?;
    let (sig, recid) = signing
        .sign_prehash_recoverable(&digest)
        .context("sign payment")?;
    let signature = signature_hex(&sig.to_bytes(), recid);
    let payload = serde_json::json!({
        "x402Version": 1,
        "scheme": "exact",
        "network": accept.network,
        "asset": accept.asset,
        "payload": {
            "signature": signature,
            "authorization": {
                "from": from,
                "to": accept.pay_to,
                "value": accept.amount.to_string(),
                "validAfter": "0",
                "validBefore": valid_before.to_string(),
                "nonce": nonce_hex(&nonce),
            }
        }
    });
    Ok(BASE64.encode(serde_json::to_vec(&payload)?))
}

pub fn credit_body(invokes: u64) -> String {
    serde_json::json!({ "invokes": invokes }).to_string()
}

fn signing_key(hex_key: &str) -> Result<SigningKey> {
    let raw = hex::decode(hex_key.trim().trim_start_matches("0x")).context("private key hex")?;
    if raw.len() != 32 {
        bail!("private key must be 32 bytes");
    }
    SigningKey::from_slice(&raw).context("private key")
}

fn address_of(key: &VerifyingKey) -> String {
    let encoded = key.to_encoded_point(false);
    let hash = keccak(&encoded.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}

fn eip712_digest(
    from: &str,
    to: &str,
    value: u64,
    valid_before: u64,
    nonce: &[u8; 32],
    chain_id: u64,
    asset: &str,
) -> Result<[u8; 32]> {
    let domain_type = keccak(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name = keccak(b"USD Coin");
    let version = keccak(b"2");
    let mut domain = Vec::with_capacity(32 * 5);
    domain.extend(domain_type);
    domain.extend(name);
    domain.extend(version);
    domain.extend(word_u256(chain_id));
    domain.extend(word_address(asset)?);
    let domain_separator = keccak(&domain);

    let struct_type = keccak(
        b"TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)",
    );
    let mut message = Vec::with_capacity(32 * 7);
    message.extend(struct_type);
    message.extend(word_address(from)?);
    message.extend(word_address(to)?);
    message.extend(word_u256(value));
    message.extend(word_u256(0));
    message.extend(word_u256(valid_before));
    message.extend(nonce);
    let struct_hash = keccak(&message);

    let mut digest_preimage = Vec::with_capacity(2 + 64);
    digest_preimage.extend_from_slice(&[0x19, 0x01]);
    digest_preimage.extend(domain_separator);
    digest_preimage.extend(struct_hash);
    Ok(keccak(&digest_preimage))
}

fn signature_hex(rs: &[u8], recid: RecoveryId) -> String {
    let mut out = Vec::with_capacity(65);
    out.extend_from_slice(rs);
    out.push(recid.to_byte() + 27);
    format!("0x{}", hex::encode(out))
}

fn nonce_hex(nonce: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(nonce))
}

fn random_bytes32() -> Result<[u8; 32]> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("payment nonce: {e}"))?;
    Ok(nonce)
}

fn keccak(data: &[u8]) -> [u8; 32] {
    let digest = Keccak256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn word_u256(value: u64) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[24..].copy_from_slice(&value.to_be_bytes());
    out
}

fn word_address(addr: &str) -> Result<[u8; 32]> {
    let raw = hex::decode(addr.trim().trim_start_matches("0x")).context("address")?;
    if raw.len() != 20 {
        bail!("address must be 20 bytes");
    }
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(&raw);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_body_is_an_invoke_count() {
        let body: serde_json::Value = serde_json::from_str(&credit_body(4)).unwrap();
        assert_eq!(body["invokes"], 4);
        assert!(body.get("usdc").is_none());
        assert!(body.get("amount").is_none());
        assert!(body.get("value").is_none());
    }

    #[test]
    fn signature_uses_the_challenge_amount() {
        let accept = ChallengeAccept {
            amount: 40000,
            pay_to: "0x0000000000000000000000000000000000000001".into(),
            asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
            network: "base".into(),
        };
        let header = payment_signature(
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
            &accept,
            8453,
            2_000_000_000,
        )
        .unwrap();
        let json = String::from_utf8(BASE64.decode(header).unwrap()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["payload"]["authorization"]["value"], "40000");
        assert_eq!(value["scheme"], "exact");
        let sig = value["payload"]["signature"].as_str().unwrap();
        assert!(sig.starts_with("0x"));
        assert_eq!(hex::decode(sig.trim_start_matches("0x")).unwrap().len(), 65);
    }
}
