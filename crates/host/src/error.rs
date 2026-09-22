use application::AppError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

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

impl From<AppError> for HttpError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        if let AppError::InsufficientCredits {
            credit_cost,
            balance,
            credit_url,
            spend_cap_remaining,
        } = &self.0
        {
            let body = Json(CreditsBody {
                error: "insufficient invoke credits".into(),
                credit_cost: *credit_cost,
                balance: *balance,
                credit_url: credit_url.clone(),
                spend_cap_remaining: *spend_cap_remaining,
            });
            return (StatusCode::PAYMENT_REQUIRED, body).into_response();
        }
        let status = match &self.0 {
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
            | AppError::Attestation(_)
            | AppError::DeployMinimum { .. }
            | AppError::PaymentChallenge { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::InsufficientCredits { .. } => unreachable!(),
        };
        if self.0.is_internal() {
            tracing::error!(error = %self.0, "request failed");
        }
        let body = Json(ErrorBody {
            error: self.0.public_message(),
        });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn bad_request_maps_to_400() {
        let res = HttpError(AppError::BadRequest("invalid nonce".into())).into_response();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("invalid nonce"));
    }

    #[tokio::test]
    async fn timeout_maps_to_504() {
        let res = HttpError(AppError::Timeout("epoch".into())).into_response();
        assert_eq!(res.status(), StatusCode::GATEWAY_TIMEOUT);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("timed out"));
    }

    #[tokio::test]
    async fn payload_too_large_maps_to_413() {
        let res = HttpError(AppError::PayloadTooLarge("big".into())).into_response();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn trap_maps_to_500() {
        let res = HttpError(AppError::Trap("unreachable".into())).into_response();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("internal error"), "{text}");
        assert!(!text.contains("unreachable"), "{text}");
    }

    #[tokio::test]
    async fn storage_does_not_leak_internals() {
        let res =
            HttpError(AppError::Storage("AccessDeniedException secret".into())).into_response();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(res.into_body(), 1024).await.unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("internal error"), "{text}");
        assert!(!text.contains("AccessDeniedException"), "{text}");
    }
}
