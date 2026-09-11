//! Host `nitrum.http_get` import (GET only).

#[cfg(target_arch = "wasm32")]
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
#[cfg(target_arch = "wasm32")]
use serde::Deserialize;

use crate::http::Response;
use crate::Error;

#[cfg(target_arch = "wasm32")]
const ERR_BAD_ARGS: i32 = -1;
#[cfg(target_arch = "wasm32")]
const ERR_DENIED: i32 = -2;
#[cfg(target_arch = "wasm32")]
const ERR_TIMEOUT: i32 = -3;
#[cfg(target_arch = "wasm32")]
const ERR_TOO_LARGE: i32 = -4;
#[cfg(target_arch = "wasm32")]
const ERR_TRANSPORT: i32 = -5;

#[cfg(target_arch = "wasm32")]
const OUT_CAP: usize = 512 * 1024;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Deserialize)]
struct HttpEnvelope {
    status: u16,
    body_base64: String,
}

/// Perform an HTTPS GET via the host import.
pub fn fetch_get(url: &str) -> Result<Response, Error> {
    #[cfg(target_arch = "wasm32")]
    {
        wasm_get(url)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = url;
        Err(Error::from_message(
            "outbound HTTP is only available in wasm32 guest builds",
        ))
    }
}

/// Back-compat alias for [`fetch_get`].
pub fn get(url: &str) -> Result<Response, Error> {
    fetch_get(url)
}

#[cfg(target_arch = "wasm32")]
fn wasm_get(url: &str) -> Result<Response, Error> {
    let url_bytes = url.as_bytes();
    let mut out = vec![0u8; OUT_CAP];
    let written = unsafe {
        http_get(
            url_bytes.as_ptr() as i32,
            i32::try_from(url_bytes.len()).map_err(|_| Error::from_message("url too long"))?,
            out.as_mut_ptr() as i32,
            i32::try_from(OUT_CAP).map_err(|_| Error::from_message("out cap overflow"))?,
        )
    };
    if written < 0 {
        return Err(map_host_code(written));
    }
    let written = written as usize;
    let envelope: HttpEnvelope = serde_json::from_slice(&out[..written])?;
    let body = BASE64
        .decode(envelope.body_base64.as_bytes())
        .map_err(|e| Error::from_message(e.to_string()))?;
    Ok(Response::from_parts(envelope.status, Vec::new(), body))
}

#[cfg(target_arch = "wasm32")]
fn map_host_code(code: i32) -> Error {
    match code {
        ERR_BAD_ARGS => Error::from_message("http_get: bad arguments"),
        ERR_DENIED => Error::from_message("http_get: origin not allowed"),
        ERR_TIMEOUT => Error::from_message("http_get: timeout"),
        ERR_TOO_LARGE => Error::from_message("http_get: response too large"),
        ERR_TRANSPORT => Error::from_message("http_get: transport error"),
        other => Error::from_message(format!("http_get failed: {other}")),
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "nitrum")]
extern "C" {
    fn http_get(url_ptr: i32, url_len: i32, out_ptr: i32, out_cap: i32) -> i32;
}
