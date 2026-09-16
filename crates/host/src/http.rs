use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use domain::{ContentHash, FunctionId, InvokeRequest, VersionLabel, MAX_INVOKE_BODY_BYTES};
use runtime::{decode_response, encode_request, Request as FnRequest};
use tower_http::trace::TraceLayer;

use crate::error::HttpError;
use crate::state::AppState;

pub const SHASUM_HEADER: &str = "x-nitrum-fn-shasum";
pub const NONCE_HEADER: &str = "x-nitrum-fn-nonce";
pub const ATTESTATION_HEADER: &str = "x-nitrum-fn-attestation";

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

    let nonce = parse_nonce(&headers)?;

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
    let response_body = fn_res.body().to_vec();

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

    let hash_hex = response.content_hash.to_hex();
    out_headers.insert(
        HeaderName::from_static(SHASUM_HEADER),
        HeaderValue::from_str(&hash_hex)
            .map_err(|e| application::AppError::Invoke(format!("hash header: {e}")))?,
    );

    if let Some(nonce) = nonce {
        if let Some(doc) = state
            .attest_invoke(&response.content_hash, &response_body, &nonce)
            .await?
        {
            let b64 = BASE64.encode(&doc);
            out_headers.insert(
                HeaderName::from_static(ATTESTATION_HEADER),
                HeaderValue::from_str(&b64).map_err(|e| {
                    application::AppError::Invoke(format!("attestation header: {e}"))
                })?,
            );
        }
    }

    Ok((status, out_headers, response_body))
}

fn parse_nonce(headers: &HeaderMap) -> Result<Option<Vec<u8>>, HttpError> {
    let Some(raw) = headers.get(NONCE_HEADER).and_then(|v| v.to_str().ok()) else {
        return Ok(None);
    };
    let bytes = BASE64
        .decode(raw.trim())
        .map_err(|_| application::AppError::Compile("invalid x-nitrum-fn-nonce (base64)".into()))?;
    if !(16..=32).contains(&bytes.len()) {
        return Err(application::AppError::Compile(format!(
            "x-nitrum-fn-nonce must decode to 16..=32 bytes, got {}",
            bytes.len()
        ))
        .into());
    }
    Ok(Some(bytes))
}

/// Build NSM `user_data`: content hash (32) || sha256(HTTP body) (32).
pub fn invoke_user_data(hash: &ContentHash, body: &[u8]) -> [u8; 64] {
    let body_hash = ContentHash::from_bytes(body);
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(hash.as_bytes());
    out[32..].copy_from_slice(body_hash.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_is_hash_then_body_digest() {
        let hash = ContentHash::from_bytes(b"\0asm");
        let body = br#"{"ok":true}"#;
        let ud = invoke_user_data(&hash, body);
        assert_eq!(&ud[..32], hash.as_bytes());
        assert_eq!(&ud[32..], ContentHash::from_bytes(body).as_bytes());
    }
}
