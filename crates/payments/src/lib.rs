//! Local x402 `exact` check and facilitator settle for invoke-credit purchases.
//!
//! The host does not depend on this crate. Credit the ledger only after
//! [`apply_after_settle`] returns.

mod settler;

pub use settler::{HttpFacilitator, X402Settler};

use std::future::Future;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PaymentError {
    #[error("invalid payment: {0}")]
    Invalid(String),
    #[error("payment amount {got} does not equal {expected} atomic USDC")]
    Amount { got: u64, expected: u64 },
    #[error("payment recipient does not match the platform")]
    Recipient,
    #[error("payment asset does not match USDC")]
    Asset,
    #[error("payment expired")]
    Expired,
    #[error("payment nonce missing")]
    Nonce,
    #[error("settle failed: {0}")]
    Settle(String),
}

/// Atomic USDC (6 decimals) due for `invokes` credits at the platform price.
pub fn due_usdc(invokes: u64, usdc_per_invoke: u64) -> Result<u64, PaymentError> {
    invokes
        .checked_mul(usdc_per_invoke)
        .ok_or_else(|| PaymentError::Invalid("USDC amount overflow".into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactAuthorization {
    pub value: u64,
    pub asset: String,
    pub pay_to: String,
    pub valid_before: u64,
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactRequirements {
    pub amount: u64,
    pub asset: String,
    pub pay_to: String,
}

/// Local check: amount, USDC asset, platform `payTo`, expiry, and nonce.
pub fn check_exact(
    auth: &ExactAuthorization,
    expected: &ExactRequirements,
    now_unix: u64,
) -> Result<(), PaymentError> {
    if auth.nonce.trim().is_empty() {
        return Err(PaymentError::Nonce);
    }
    if auth.valid_before <= now_unix {
        return Err(PaymentError::Expired);
    }
    if !eq_addr(&auth.asset, &expected.asset) {
        return Err(PaymentError::Asset);
    }
    if !eq_addr(&auth.pay_to, &expected.pay_to) {
        return Err(PaymentError::Recipient);
    }
    if auth.value != expected.amount {
        return Err(PaymentError::Amount {
            got: auth.value,
            expected: expected.amount,
        });
    }
    Ok(())
}

/// Run `credit` only after `settle` succeeds.
pub async fn apply_after_settle<T, E>(
    settle: impl Future<Output = Result<(), E>>,
    credit: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    settle.await?;
    credit()
}

pub fn parse_payment(header: &str) -> Result<ExactAuthorization, PaymentError> {
    let raw = header.trim();
    let json = if raw.starts_with('{') {
        raw.to_string()
    } else {
        let bytes = BASE64
            .decode(raw)
            .map_err(|e| PaymentError::Invalid(format!("payment is not base64: {e}")))?;
        String::from_utf8(bytes)
            .map_err(|e| PaymentError::Invalid(format!("payment is not utf-8: {e}")))?
    };
    let value: Value = serde_json::from_str(&json)
        .map_err(|e| PaymentError::Invalid(format!("payment json: {e}")))?;
    let auth = value
        .get("payload")
        .and_then(|p| p.get("authorization"))
        .ok_or_else(|| PaymentError::Invalid("missing payload.authorization".into()))?;
    let value_units = json_u64(auth.get("value"))
        .ok_or_else(|| PaymentError::Invalid("authorization.value".into()))?;
    let pay_to =
        json_str(auth.get("to")).ok_or_else(|| PaymentError::Invalid("authorization.to".into()))?;
    let valid_before = json_u64(auth.get("validBefore"))
        .ok_or_else(|| PaymentError::Invalid("authorization.validBefore".into()))?;
    let nonce = json_str(auth.get("nonce"))
        .ok_or_else(|| PaymentError::Invalid("authorization.nonce".into()))?;
    let asset = json_str(value.get("asset"))
        .or_else(|| json_str(auth.get("asset")))
        .ok_or_else(|| PaymentError::Invalid("missing asset".into()))?;
    let scheme = json_str(value.get("scheme")).unwrap_or_default();
    if scheme != "exact" {
        return Err(PaymentError::Invalid(format!(
            "scheme {scheme} is not exact"
        )));
    }
    Ok(ExactAuthorization {
        value: value_units,
        asset,
        pay_to,
        valid_before,
        nonce,
    })
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn json_str(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

fn eq_addr(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn auth(value: u64) -> ExactAuthorization {
        ExactAuthorization {
            value,
            asset: "0xUSDC".into(),
            pay_to: "0xPlatform".into(),
            valid_before: 1_000,
            nonce: "nonce-1".into(),
        }
    }

    fn expected(amount: u64) -> ExactRequirements {
        ExactRequirements {
            amount,
            asset: "0xusdc".into(),
            pay_to: "0xplatform".into(),
        }
    }

    #[test]
    fn amount_must_equal_invokes_times_usdc_per_invoke() {
        assert_eq!(due_usdc(4, 25).unwrap(), 100);
        let err = check_exact(&auth(99), &expected(100), 10).unwrap_err();
        assert!(matches!(
            err,
            PaymentError::Amount {
                got: 99,
                expected: 100
            }
        ));
        check_exact(&auth(100), &expected(100), 10).unwrap();
        assert!(matches!(
            check_exact(&auth(100), &expected(100), 1_000).unwrap_err(),
            PaymentError::Expired
        ));
        let mut wrong_to = auth(100);
        wrong_to.pay_to = "0xother".into();
        assert!(matches!(
            check_exact(&wrong_to, &expected(100), 10).unwrap_err(),
            PaymentError::Recipient
        ));
        let mut wrong_asset = auth(100);
        wrong_asset.asset = "0xother".into();
        assert!(matches!(
            check_exact(&wrong_asset, &expected(100), 10).unwrap_err(),
            PaymentError::Asset
        ));
    }

    #[tokio::test]
    async fn credit_is_not_attempted_before_settle_succeeds() {
        let credited = AtomicBool::new(false);
        let err = apply_after_settle(
            async { Err(PaymentError::Settle("facilitator down".into())) },
            || {
                credited.store(true, Ordering::SeqCst);
                Ok::<(), PaymentError>(())
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PaymentError::Settle(_)));
        assert!(!credited.load(Ordering::SeqCst));

        let ok = apply_after_settle(async { Ok::<(), PaymentError>(()) }, || Ok(7u64))
            .await
            .unwrap();
        assert_eq!(ok, 7);
    }
}
