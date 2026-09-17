use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, put};
use axum::{Json, Router};
use domain::{
    normalize_egress_allow, EgressOrigin, FunctionId, PublishRequest, VersionLabel, MAX_WASM_BYTES,
};
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

/// PUT /functions/{name} — validate, store, upsert catalog.
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
    egress_allow: Vec<String>,
}

const ALLOW_URL_HEADER: &str = "x-nitrum-fn-allow-url";

fn parse_allow_headers(headers: &HeaderMap) -> Result<Vec<EgressOrigin>, HttpError> {
    let mut origins = Vec::new();
    for value in headers.get_all(ALLOW_URL_HEADER) {
        let raw = value
            .to_str()
            .map_err(|_| application::AppError::BadRequest("invalid allow-url header".into()))?;
        origins.push(EgressOrigin::parse(raw).map_err(application::AppError::from)?);
    }
    normalize_egress_allow(origins)
        .map_err(application::AppError::from)
        .map_err(Into::into)
}

async fn publish(
    State(state): State<PublishState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let function = FunctionId::new(&name).map_err(application::AppError::from)?;
    let egress_allow = parse_allow_headers(&headers)?;
    let response = state
        .publish
        .execute(PublishRequest {
            function,
            wasm: body.to_vec(),
            egress_allow,
        })
        .await?;
    Ok((
        StatusCode::OK,
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
        egress_allow: version
            .egress_allow
            .iter()
            .map(|o| o.as_str().to_string())
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ports::{ArtifactStore, FunctionRunner, PublishLock, RunOutcome};
    use application::AppError;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{HeaderValue, Request, StatusCode};
    use domain::{ContentHash, FunctionVersion};
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
    }

    struct MemCatalog {
        rows: Mutex<HashMap<String, (ContentHash, Vec<EgressOrigin>)>>,
    }

    impl MemCatalog {
        fn new() -> Self {
            Self {
                rows: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl FunctionCatalog for MemCatalog {
        async fn upsert(
            &self,
            id: &FunctionId,
            _label: &VersionLabel,
            hash: ContentHash,
            _queued_at_ms: u64,
            egress_allow: &[EgressOrigin],
        ) -> Result<bool, AppError> {
            self.rows
                .lock()
                .unwrap()
                .insert(id.as_str().to_string(), (hash, egress_allow.to_vec()));
            Ok(true)
        }

        async fn resolve(
            &self,
            id: &FunctionId,
            label: &VersionLabel,
        ) -> Result<FunctionVersion, AppError> {
            let (hash, egress_allow) = self
                .rows
                .lock()
                .unwrap()
                .get(id.as_str())
                .cloned()
                .ok_or_else(|| AppError::NotFound(id.to_string()))?;
            Ok(FunctionVersion {
                id: id.clone(),
                label: label.clone(),
                content_hash: hash,
                egress_allow,
            })
        }

        async fn list(&self) -> Result<Vec<FunctionVersion>, AppError> {
            Ok(vec![])
        }
    }

    struct MemLock {
        held: Mutex<HashMap<String, String>>,
        release: bool,
    }

    impl MemLock {
        fn new() -> Self {
            Self {
                held: Mutex::new(HashMap::new()),
                release: true,
            }
        }

        fn sticky() -> Self {
            Self {
                held: Mutex::new(HashMap::new()),
                release: false,
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
            if !self.release {
                return Ok(());
            }
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

    struct AcceptingRunner;

    #[async_trait]
    impl FunctionRunner for AcceptingRunner {
        async fn validate(&self, _wasm: &[u8]) -> Result<(), AppError> {
            Ok(())
        }

        async fn run(
            &self,
            _hash: &ContentHash,
            _wasm: &[u8],
            _input: &[u8],
            _egress_allow: &[domain::EgressOrigin],
        ) -> Result<RunOutcome, AppError> {
            unimplemented!()
        }
    }

    fn publish_app() -> (Router, Arc<MemCatalog>) {
        let catalog = Arc::new(MemCatalog::new());
        let usecase = Arc::new(PublishFunction::new(
            Arc::new(MemArtifacts),
            catalog.clone(),
            Arc::new(MemLock::new()),
            Arc::new(AcceptingRunner),
        ));
        (publish_router(usecase), catalog)
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
            Arc::new(MemCatalog::new()),
            Arc::new(MemLock::sticky()),
            Arc::new(AcceptingRunner),
        ));
        let first = publish_router(usecase.clone())
            .oneshot(put(b"\0asm one"))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let second = publish_router(usecase)
            .oneshot(put(b"\0asm two"))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn publish_records_allow_urls_in_catalog() {
        let (app, catalog) = publish_app();
        let req = Request::builder()
            .method("PUT")
            .uri("/functions/echo")
            .header("content-type", "application/wasm")
            .header("x-nitrum-fn-allow-url", "https://api.example.com")
            .body(Body::from(b"\0asm one".as_slice()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let rows = catalog.rows.lock().unwrap();
        let (_, egress) = rows.get("echo").expect("catalog row");
        assert_eq!(egress.len(), 1);
        assert_eq!(egress[0].as_str(), "https://api.example.com");
    }

    #[tokio::test]
    async fn publish_rejects_non_utf8_allow_url() {
        let (app, _catalog) = publish_app();
        let mut req = Request::builder()
            .method("PUT")
            .uri("/functions/echo")
            .header("content-type", "application/wasm")
            .body(Body::from(b"\0asm one".as_slice()))
            .unwrap();
        req.headers_mut().insert(
            ALLOW_URL_HEADER,
            HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
        );
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn publish_without_extra_headers_is_ok() {
        let (app, catalog) = publish_app();
        let res = app.oneshot(put(b"\0asm one")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(catalog.rows.lock().unwrap().contains_key("echo"));
    }

    #[tokio::test]
    async fn body_over_default_limit_is_413() {
        let (app, catalog) = publish_app();
        let oversize = vec![0u8; domain::MAX_WASM_BYTES + 1];
        let req = Request::builder()
            .method("PUT")
            .uri("/functions/echo")
            .header("content-type", "application/wasm")
            .body(Body::from(oversize))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(catalog.rows.lock().unwrap().is_empty());
    }
}
