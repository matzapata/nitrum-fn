use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};

use crate::http::{Request, Response};
use crate::Error;

#[derive(Debug, Serialize, Deserialize)]
pub struct WireRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body_base64: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WireResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body_base64: String,
}

pub fn encode_request(req: &Request) -> Result<Vec<u8>, Error> {
    let wire = WireRequest {
        method: req.method().to_string(),
        path: req.path().to_string(),
        headers: req.headers().to_vec(),
        body_base64: BASE64.encode(req.body()),
    };
    Ok(serde_json::to_vec(&wire)?)
}

pub fn decode_request(bytes: &[u8]) -> Result<Request, Error> {
    let wire: WireRequest = serde_json::from_slice(bytes)?;
    let body = BASE64
        .decode(wire.body_base64.as_bytes())
        .map_err(|e| Error::from_message(e.to_string()))?;
    Ok(Request::new(wire.method, wire.path, wire.headers, body))
}

pub fn encode_response(res: &Response) -> Result<Vec<u8>, Error> {
    let wire = WireResponse {
        status: res.status(),
        headers: res.headers().to_vec(),
        body_base64: BASE64.encode(res.body()),
    };
    Ok(serde_json::to_vec(&wire)?)
}

pub fn decode_response(bytes: &[u8]) -> Result<Response, Error> {
    let wire: WireResponse = serde_json::from_slice(bytes)?;
    let body = BASE64
        .decode(wire.body_base64.as_bytes())
        .map_err(|e| Error::from_message(e.to_string()))?;
    Ok(Response::from_parts(wire.status, wire.headers, body))
}
