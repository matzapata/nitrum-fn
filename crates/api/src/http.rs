use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use domain::{FunctionId, IdempotencyKey, PublishRequest, VersionLabel};
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
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let function = FunctionId::new(&name).map_err(application::AppError::from)?;
    let idempotency_key = match headers.get("idempotency-key") {
        Some(value) => {
            let raw = value.to_str().map_err(|_| {
                application::AppError::from(domain::DomainError::InvalidIdempotencyKey(
                    "<non-utf8>".into(),
                ))
            })?;
            Some(IdempotencyKey::new(raw).map_err(application::AppError::from)?)
        }
        None => None,
    };
    let response = state
        .publish
        .execute(PublishRequest {
            function,
            wasm: body.to_vec(),
            idempotency_key,
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
    use application::ports::{ArtifactStore, PublishBus};
    use application::AppError;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use catalog::InMemoryPublishIdempotency;
    use domain::{ContentHash, PublishQueuedEvent};
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

    fn publish_app() -> (Router, Arc<MemBus>) {
        let bus = Arc::new(MemBus::new());
        let usecase = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts),
            bus.clone(),
            Arc::new(InMemoryPublishIdempotency::new()),
        ));
        (publish_router(usecase), bus)
    }

    fn put(key: Option<&str>, body: &'static [u8]) -> Request<Body> {
        let mut builder = Request::builder()
            .method("PUT")
            .uri("/functions/echo")
            .header("content-type", "application/wasm");
        if let Some(key) = key {
            builder = builder.header("idempotency-key", key);
        }
        builder.body(Body::from(body)).unwrap()
    }

    #[tokio::test]
    async fn invalid_idempotency_key_is_400() {
        let (app, _) = publish_app();
        let res = app
            .oneshot(put(Some("retry/1"), b"\0asm one"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn non_utf8_idempotency_key_is_400() {
        let (app, _) = publish_app();
        let mut req = put(None, b"\0asm one");
        req.headers_mut().insert(
            "idempotency-key",
            axum::http::HeaderValue::from_bytes(&[0xff]).unwrap(),
        );
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn reused_key_different_body_is_409() {
        let usecase = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts),
            Arc::new(MemBus::new()),
            Arc::new(InMemoryPublishIdempotency::new()),
        ));
        let first = publish_router(usecase.clone())
            .oneshot(put(Some("retry-1"), b"\0asm one"))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let second = publish_router(usecase)
            .oneshot(put(Some("retry-1"), b"\0asm two"))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn reused_key_same_body_is_accepted_once_on_the_bus() {
        let bus = Arc::new(MemBus::new());
        let usecase = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts),
            bus.clone(),
            Arc::new(InMemoryPublishIdempotency::new()),
        ));
        let first = publish_router(usecase.clone())
            .oneshot(put(Some("retry-1"), b"\0asm one"))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let second = publish_router(usecase)
            .oneshot(put(Some("retry-1"), b"\0asm one"))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::ACCEPTED);
        assert_eq!(bus.events.lock().unwrap().len(), 1);
    }
}
