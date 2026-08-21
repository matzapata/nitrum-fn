use std::sync::Arc;

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
use crate::state::{ApiState, CatalogState, PublishState};
use application::ports::FunctionCatalog;
use application::PublishFunction;

/// Health + catalog GET. Safe to mount without a publish bus (enclave / seed-only host).
pub fn catalog_router(catalog: Arc<dyn FunctionCatalog>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/functions/{name}", get(get_function))
        .layer(TraceLayer::new_for_http())
        .with_state(CatalogState { catalog })
}

/// PUT /functions/{name} — requires a publish bus.
pub fn publish_router(usecase: Arc<PublishFunction>) -> Router {
    Router::new()
        .route("/functions/{name}", put(publish))
        .layer(TraceLayer::new_for_http())
        .with_state(PublishState { publish: usecase })
}

pub fn router(state: ApiState) -> Router {
    catalog_router(state.catalog).merge(publish_router(state.publish))
}

#[derive(Serialize)]
struct PublishBody {
    name: String,
    version: String,
    hash: String,
    wasm_bytes: usize,
    status: String,
}

#[derive(Serialize)]
struct FunctionBody {
    name: String,
    version: String,
    hash: String,
}

async fn publish(
    State(state): State<PublishState>,
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
        StatusCode::ACCEPTED,
        Json(PublishBody {
            name: response.function.to_string(),
            version: response.version.to_string(),
            hash: response.content_hash.to_hex(),
            wasm_bytes: response.wasm_bytes,
            status: response.status.to_string(),
        }),
    ))
}

async fn get_function(
    State(state): State<CatalogState>,
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
