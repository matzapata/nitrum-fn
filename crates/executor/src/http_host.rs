//! Guest `nitrum.http_get` host import and outbound HTTP client.

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use domain::{
    EgressOrigin, HTTP_TIMEOUT, MAX_HTTP_BODY_BYTES, MAX_HTTP_URL_BYTES,
};
use serde::Serialize;

/// Result codes returned to the guest when `http_get` fails.
pub const ERR_BAD_ARGS: i32 = -1;
pub const ERR_DENIED: i32 = -2;
pub const ERR_TIMEOUT: i32 = -3;
pub const ERR_TOO_LARGE: i32 = -4;
pub const ERR_TRANSPORT: i32 = -5;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum HttpGetError {
    Denied,
    Timeout,
    TooLarge,
    Transport(String),
}

impl HttpGetError {
    pub fn code(&self) -> i32 {
        match self {
            Self::Denied => ERR_DENIED,
            Self::Timeout => ERR_TIMEOUT,
            Self::TooLarge => ERR_TOO_LARGE,
            Self::Transport(_) => ERR_TRANSPORT,
        }
    }
}

pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str, allow: &[EgressOrigin]) -> Result<HttpResponse, HttpGetError>;
}

#[derive(Default)]
pub struct UreqClient;

impl HttpClient for UreqClient {
    fn get(&self, url: &str, allow: &[EgressOrigin]) -> Result<HttpResponse, HttpGetError> {
        if !is_allowed(url, allow) {
            return Err(HttpGetError::Denied);
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(HTTP_TIMEOUT)
            .redirects(0)
            .build();
        let response = agent.get(url).call().map_err(|e| {
            let msg = e.to_string();
            if msg.to_ascii_lowercase().contains("timeout") {
                HttpGetError::Timeout
            } else {
                HttpGetError::Transport(msg)
            }
        })?;
        let status = response.status();
        let mut body = Vec::new();
        response
            .into_reader()
            .take(MAX_HTTP_BODY_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|e| HttpGetError::Transport(e.to_string()))?;
        if body.len() > MAX_HTTP_BODY_BYTES {
            return Err(HttpGetError::TooLarge);
        }
        Ok(HttpResponse { status, body })
    }
}

use std::io::Read;

#[derive(Debug, Clone)]
pub struct StubHttpClient {
    pub response: Result<HttpResponse, HttpGetError>,
}

impl HttpClient for StubHttpClient {
    fn get(&self, url: &str, allow: &[EgressOrigin]) -> Result<HttpResponse, HttpGetError> {
        if !is_allowed(url, allow) {
            return Err(HttpGetError::Denied);
        }
        self.response.clone()
    }
}

fn is_allowed(url: &str, allow: &[EgressOrigin]) -> bool {
    if allow.is_empty() {
        return false;
    }
    let Ok(origin) = EgressOrigin::parse(url) else {
        return false;
    };
    allow.iter().any(|a| a.as_str() == origin.as_str())
}

#[derive(Serialize)]
struct HttpEnvelope {
    status: u16,
    body_base64: String,
}

pub fn encode_http_envelope(res: &HttpResponse) -> Result<Vec<u8>, ()> {
    let wire = HttpEnvelope {
        status: res.status,
        body_base64: BASE64.encode(&res.body),
    };
    serde_json::to_vec(&wire).map_err(|_| ())
}

pub fn fetch_url(
    client: &dyn HttpClient,
    url: &str,
    allow: &[EgressOrigin],
) -> Result<Vec<u8>, i32> {
    if url.len() > MAX_HTTP_URL_BYTES {
        return Err(ERR_BAD_ARGS);
    }
    match client.get(url, allow) {
        Ok(res) => encode_http_envelope(&res).map_err(|_| ERR_TRANSPORT),
        Err(e) => Err(e.code()),
    }
}

pub type SharedHttpClient = Arc<dyn HttpClient>;

pub fn default_http_client() -> SharedHttpClient {
    Arc::new(UreqClient)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_when_allowlist_empty() {
        let client = UreqClient;
        let err = client
            .get("https://example.com/", &[])
            .expect_err("denied");
        assert!(matches!(err, HttpGetError::Denied));
    }

    #[test]
    fn denies_origin_not_in_allowlist() {
        let client = StubHttpClient {
            response: Ok(HttpResponse {
                status: 200,
                body: b"ok".to_vec(),
            }),
        };
        let allow = vec![EgressOrigin::parse("https://allowed.example.com").unwrap()];
        let err = client
            .get("https://other.example.com/path", &allow)
            .expect_err("denied");
        assert!(matches!(err, HttpGetError::Denied));
    }
}
