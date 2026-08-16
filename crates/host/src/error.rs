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
            AppError::Domain(_) | AppError::HashMismatch { .. } | AppError::Compile(_) => {
                StatusCode::BAD_REQUEST
            }
            AppError::Invoke(_) | AppError::Trap(_) | AppError::Storage(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let body = Json(ErrorBody {
            error: self.0.to_string(),
        });
        (status, body).into_response()
    }
}
