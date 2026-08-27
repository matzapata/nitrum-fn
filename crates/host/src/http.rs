use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use domain::{FunctionId, InvokeRequest, VersionLabel, MAX_INVOKE_BODY_BYTES};
use runtime::{decode_response, encode_request, Request as FnRequest};
use tower_http::trace::TraceLayer;

use crate::error::HttpError;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    telemetry::http::instrument_router(
        Router::new()
            .route("/healthz", get(|| async { StatusCode::OK }))
            .route("/invoke/{name}", post(invoke))
            .layer(DefaultBodyLimit::max(MAX_INVOKE_BODY_BYTES))
            .layer(TraceLayer::new_for_http())
            .with_state(state),
    )
}

async fn invoke(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let function = FunctionId::new(&name).map_err(application::AppError::from)?;
    let version = match headers
        .get("x-nitrum-fn-version")
        .and_then(|v| v.to_str().ok())
    {
        Some(raw) => VersionLabel::new(raw).map_err(application::AppError::from)?,
        None => VersionLabel::latest(),
    };

    let path = format!("/invoke/{name}");
    let fn_headers = headers
        .iter()
        .filter_map(|(k, v)| {
            let value = v.to_str().ok()?.to_string();
            Some((k.as_str().to_string(), value))
        })
        .collect();

    if body.len() > MAX_INVOKE_BODY_BYTES {
        return Err(application::AppError::PayloadTooLarge(format!(
            "invoke body {} bytes exceeds max {MAX_INVOKE_BODY_BYTES}",
            body.len()
        ))
        .into());
    }

    let fn_req = FnRequest::new("POST", path, fn_headers, body.to_vec());
    let payload = encode_request(&fn_req)
        .map_err(|e| application::AppError::Invoke(format!("encode request: {e}")))?;

    let response = state
        .invoke
        .execute(InvokeRequest {
            function,
            version,
            payload,
        })
        .await?;

    let fn_res = decode_response(&response.output)
        .map_err(|e| application::AppError::Invoke(format!("decode response: {e}")))?;

    let status = StatusCode::from_u16(fn_res.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut out_headers = HeaderMap::new();
    for (name, value) in fn_res.headers() {
        let Ok(header_name) = HeaderName::try_from(name.as_str()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::try_from(value.as_str()) else {
            continue;
        };
        out_headers.insert(header_name, header_value);
    }

    Ok((status, out_headers, fn_res.body().to_vec()))
}
