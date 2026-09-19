//! Nitro attestation adapters for the host (noop locally, Nitrum crypto API in-enclave).

use application::ports::FunctionAttestor;
use application::AppError;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};

/// Nitrum data-plane loopback crypto API (fixed by the Nitrum enclave platform).
pub const NITRUM_CRYPTO_API: &str = "http://127.0.0.1:3000";

/// Local / Floci: never mint a document.
pub struct NoopAttestor;

#[async_trait]
impl FunctionAttestor for NoopAttestor {
    async fn attest(&self, _user_data: &[u8], _nonce: &[u8]) -> Result<Option<Vec<u8>>, AppError> {
        Ok(None)
    }
}

/// Calls Nitrum data-plane `POST /attestation` on the loopback crypto API.
pub struct NitrumCryptoAttestor {
    client: reqwest::Client,
}

impl NitrumCryptoAttestor {
    pub fn new() -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| AppError::Attestation(e.to_string()))?;
        Ok(Self { client })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AttestRequest {
    nonce: String,
    user_data: String,
}

#[derive(Deserialize)]
struct AttestResponse {
    data: Option<String>,
    error: Option<String>,
}

#[async_trait]
impl FunctionAttestor for NitrumCryptoAttestor {
    async fn attest(&self, user_data: &[u8], nonce: &[u8]) -> Result<Option<Vec<u8>>, AppError> {
        let url = format!("{NITRUM_CRYPTO_API}/attestation");
        let body = AttestRequest {
            nonce: BASE64.encode(nonce),
            user_data: BASE64.encode(user_data),
        };
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Attestation(format!("crypto api: {e}")))?;
        let status = response.status();
        let parsed: AttestResponse = response
            .json()
            .await
            .map_err(|e| AppError::Attestation(format!("decode crypto api: {e}")))?;
        if let Some(err) = parsed.error.filter(|e| !e.is_empty()) {
            return Err(AppError::Attestation(err));
        }
        if !status.is_success() {
            return Err(AppError::Attestation(format!("crypto api status {status}")));
        }
        let data = parsed
            .data
            .ok_or_else(|| AppError::Attestation("crypto api missing data".into()))?;
        let doc = BASE64
            .decode(data.trim())
            .map_err(|e| AppError::Attestation(format!("attestation base64: {e}")))?;
        Ok(Some(doc))
    }
}
