use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use domain::{FunctionId, PublishRequest, VersionLabel, MAX_WASM_BYTES};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::error::HttpError;
use crate::state::{ApiState, CatalogState, PublishState};
use application::ports::FunctionCatalog;
use application::PublishFunction;

/// Health + catalog GET.
fn catalog_router(catalog: Arc<dyn FunctionCatalog>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/functions/{name}", get(get_function))
        .layer(TraceLayer::new_for_http())
        .with_state(CatalogState { catalog })
}

/// PUT /functions/{name} — requires a publish bus.
fn publish_router(usecase: Arc<PublishFunction>) -> Router {
    Router::new()
        .route("/functions/{name}", put(publish))
        .layer(DefaultBodyLimit::max(MAX_WASM_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(PublishState { publish: usecase })
}

pub fn router(state: ApiState) -> Router {
    telemetry::http::instrument_router(
        catalog_router(state.catalog).merge(publish_router(state.publish)),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use application::ports::{ArtifactStore, PublishBus, PublishLock};
    use application::AppError;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use domain::{ContentHash, PublishQueuedEvent};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tower::ServiceExt;

    struct MemArtifacts;

    #[async_trait]
    impl ArtifactStore for MemArtifacts {
        async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError> {
            Ok(ContentHash::from_bytes(wasm))
        }
        async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
            Err(AppError::ArtifactMissing(hash.to_hex()))
        }
        async fn put_compiled(
            &self,
            _hash: &ContentHash,
            _compiled: &[u8],
        ) -> Result<(), AppError> {
            Ok(())
        }
        async fn get_compiled(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
            Err(AppError::ArtifactMissing(hash.to_hex()))
        }
    }

    struct MemBus {
        events: Mutex<Vec<PublishQueuedEvent>>,
    }

    impl MemBus {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl PublishBus for MemBus {
        async fn publish_queued(&self, event: &PublishQueuedEvent) -> Result<(), AppError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    struct MemLock {
        held: Mutex<HashMap<String, String>>,
    }

    impl MemLock {
        fn new() -> Self {
            Self {
                held: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl PublishLock for MemLock {
        async fn acquire(
            &self,
            function: &FunctionId,
            hash: &ContentHash,
            _queued_at_ms: u64,
        ) -> Result<(), AppError> {
            let mut held = self.held.lock().unwrap();
            if held.contains_key(function.as_str()) {
                return Err(AppError::Conflict(format!(
                    "publish already in progress for {function}"
                )));
            }
            held.insert(function.as_str().to_string(), hash.to_hex());
            Ok(())
        }

        async fn release(&self, function: &FunctionId, hash: &ContentHash) -> Result<(), AppError> {
            let mut held = self.held.lock().unwrap();
            if held
                .get(function.as_str())
                .is_some_and(|h| h == &hash.to_hex())
            {
                held.remove(function.as_str());
            }
            Ok(())
        }
    }

    fn publish_app() -> (Router, Arc<MemBus>) {
        let bus = Arc::new(MemBus::new());
        let usecase = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts),
            bus.clone(),
            Arc::new(MemLock::new()),
        ));
        (publish_router(usecase), bus)
    }

    fn put(body: &'static [u8]) -> Request<Body> {
        Request::builder()
            .method("PUT")
            .uri("/functions/echo")
            .header("content-type", "application/wasm")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn second_publish_while_in_progress_is_409() {
        let usecase = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts),
            Arc::new(MemBus::new()),
            Arc::new(MemLock::new()),
        ));
        let first = publish_router(usecase.clone())
            .oneshot(put(b"\0asm one"))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let second = publish_router(usecase)
            .oneshot(put(b"\0asm two"))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn publish_without_extra_headers_is_accepted() {
        let (app, bus) = publish_app();
        let res = app.oneshot(put(b"\0asm one")).await.unwrap();
        assert_eq!(res.status(), StatusCode::ACCEPTED);
        assert_eq!(bus.events.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn body_over_default_limit_is_413() {
        let (app, bus) = publish_app();
        let oversize = vec![0u8; domain::MAX_WASM_BYTES + 1];
        let req = Request::builder()
            .method("PUT")
            .uri("/functions/echo")
            .header("content-type", "application/wasm")
            .body(Body::from(oversize))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(bus.events.lock().unwrap().is_empty());
    }
}
