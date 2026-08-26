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

impl From<AppError> for HttpError {
    fn from(value: AppError) -> Self {
        Self(value)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            AppError::NotFound(_) | AppError::ArtifactMissing(_) => StatusCode::NOT_FOUND,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Domain(_) | AppError::HashMismatch { .. } | AppError::Compile(_) => {
                StatusCode::BAD_REQUEST
            }
            AppError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            AppError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            AppError::Invoke(_) | AppError::Trap(_) | AppError::Storage(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
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
