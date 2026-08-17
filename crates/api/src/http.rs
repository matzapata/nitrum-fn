use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use domain::{FunctionId, PublishRequest, VersionLabel};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::error::HttpError;
use crate::state::ApiState;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/functions/{name}", put(publish).get(get_function))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Serialize)]
struct PublishBody {
    name: String,
    version: String,
    hash: String,
    wasm_bytes: usize,
    compiled_bytes: usize,
}

#[derive(Serialize)]
struct FunctionBody {
    name: String,
    version: String,
    hash: String,
}

async fn publish(
    State(state): State<ApiState>,
    Path(name): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let function = FunctionId::new(&name).map_err(application::AppError::from)?;
    let response = state
        .publish
        .execute(PublishRequest {
            function,
            wasm: body.to_vec(),
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(PublishBody {
            name: response.function.to_string(),
            version: response.version.to_string(),
            hash: response.content_hash.to_hex(),
            wasm_bytes: response.wasm_bytes,
            compiled_bytes: response.compiled_bytes,
        }),
    ))
}

async fn get_function(
    State(state): State<ApiState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, HttpError> {
    let function = FunctionId::new(&name).map_err(application::AppError::from)?;
    let version = state
        .catalog
        .resolve(&function, &VersionLabel::latest())
        .await?;
    Ok(Json(FunctionBody {
        name: version.id.to_string(),
        version: version.label.to_string(),
        hash: version.content_hash.to_hex(),
    }))
}
