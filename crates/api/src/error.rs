use application::AppError;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Serialize;

use crate::accounts_http::payment_required_name;
#[cfg(test)]
use crate::accounts_http::PAYMENT_REQUIRED;

#[derive(Debug)]
pub struct HttpError(pub AppError);

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Serialize)]
struct CreditsBody {
    error: String,
    credit_cost: u64,
    balance: u64,
    credit_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    spend_cap_remaining: Option<u64>,
}

#[derive(Serialize)]
struct DeployBody {
    error: String,
    min_deploy_credits: u64,
    balance: u64,
    credit_url: String,
}

#[derive(Serialize)]
struct AcceptBody {
    scheme: &'static str,
    network: String,
    #[serde(rename = "maxAmountRequired")]
    max_amount_required: String,
    asset: String,
    #[serde(rename = "payTo")]
    pay_to: String,
    resource: String,
}

#[derive(Serialize)]
struct ChallengeBody {
    error: String,
    accepts: Vec<AcceptBody>,
}

impl From<AppError> for HttpError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        if self.0.is_internal() {
            tracing::error!(error = %self.0, "request failed");
        }
        match self.0 {
            AppError::PaymentChallenge {
                invokes: _,
                amount,
                asset,
                pay_to,
                resource,
                network,
            } => {
                let body = ChallengeBody {
                    error: "payment required".into(),
                    accepts: vec![AcceptBody {
                        scheme: "exact",
                        network,
                        max_amount_required: amount.to_string(),
                        asset,
                        pay_to,
                        resource,
                    }],
                };
                let encoded = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
                let mut res = (StatusCode::PAYMENT_REQUIRED, Json(body)).into_response();
                if let Ok(value) = HeaderValue::from_str(&BASE64.encode(encoded)) {
                    res.headers_mut().insert(payment_required_name(), value);
                }
                res
            }
            AppError::InsufficientCredits {
                credit_cost,
                balance,
                credit_url,
                spend_cap_remaining,
            } => (
                StatusCode::PAYMENT_REQUIRED,
                Json(CreditsBody {
                    error: "insufficient invoke credits".into(),
                    credit_cost,
                    balance,
                    credit_url,
                    spend_cap_remaining,
                }),
            )
                .into_response(),
            AppError::DeployMinimum {
                min_deploy_credits,
                balance,
                credit_url,
            } => (
                StatusCode::PAYMENT_REQUIRED,
                Json(DeployBody {
                    error: "balance below minimum deploy credits".into(),
                    min_deploy_credits,
                    balance,
                    credit_url,
                }),
            )
                .into_response(),
            other => {
                let status = match &other {
                    AppError::NotFound(_) | AppError::ArtifactMissing(_) => StatusCode::NOT_FOUND,
                    AppError::Conflict(_) => StatusCode::CONFLICT,
                    AppError::Unauthorized => StatusCode::UNAUTHORIZED,
                    AppError::Domain(_)
                    | AppError::HashMismatch { .. }
                    | AppError::BadRequest(_)
                    | AppError::Compile(_) => StatusCode::BAD_REQUEST,
                    AppError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
                    AppError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
                    AppError::Invoke(_)
                    | AppError::Trap(_)
                    | AppError::Storage(_)
                    | AppError::Attestation(_) => StatusCode::INTERNAL_SERVER_ERROR,
                    AppError::InsufficientCredits { .. }
                    | AppError::DeployMinimum { .. }
                    | AppError::PaymentChallenge { .. } => unreachable!(),
                };
                let body = Json(ErrorBody {
                    error: other.public_message(),
                });
                (status, body).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn does_not_leak_internals() {
        let res =
            HttpError(AppError::Storage("AccessDeniedException secret".into())).into_response();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        let text = String::from_utf8_lossy(&body);

        assert!(text.contains("internal error"), "{text}");
        assert!(!text.contains("AccessDeniedException"), "{text}");
    }

    #[tokio::test]
    async fn x402_challenge_sets_payment_required_header() {
        let res = HttpError(AppError::PaymentChallenge {
            invokes: 2,
            amount: 20,
            asset: "usdc".into(),
            pay_to: "0xabc".into(),
            resource: "http://api/accounts/aa/credit".into(),
            network: "base".into(),
        })
        .into_response();
        assert_eq!(res.status(), StatusCode::PAYMENT_REQUIRED);
        assert!(res.headers().get(PAYMENT_REQUIRED).is_some());
    }
}
