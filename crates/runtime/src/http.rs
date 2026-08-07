use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::Error;

/// Incoming HTTP request (as seen after Nitrum TLS termination / host proxy).
#[derive(Debug, Clone)]
pub struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            headers,
            body,
        }
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn body_string(&self) -> Result<&str, Error> {
        std::str::from_utf8(&self.body).map_err(|e| Error::from_message(e.to_string()))
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(serde_json::from_slice(&self.body)?)
    }
}

/// Outgoing HTTP response from a function.
#[derive(Debug, Clone)]
pub struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub fn builder() -> ResponseBuilder {
        ResponseBuilder {
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn json<T: Serialize>(value: &T) -> Result<Self, Error> {
        let body = serde_json::to_vec(value)?;
        Ok(Self {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body,
        })
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
            body: body.into().into_bytes(),
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub(crate) fn from_parts(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }
}

#[derive(Debug)]
pub struct ResponseBuilder {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl ResponseBuilder {
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn json<T: Serialize>(mut self, value: &T) -> Result<Response, Error> {
        self.body = serde_json::to_vec(value)?;
        if !self
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        {
            self.headers
                .push(("content-type".into(), "application/json".into()));
        }
        Ok(self.build())
    }

    pub fn build(self) -> Response {
        Response {
            status: self.status,
            headers: self.headers,
            body: self.body,
        }
    }
}

/// Convert handler return values into an HTTP [`Response`].
pub trait IntoResponse {
    fn into_response(self) -> Result<Response, Error>;
}

impl IntoResponse for Response {
    fn into_response(self) -> Result<Response, Error> {
        Ok(self)
    }
}

impl IntoResponse for Value {
    fn into_response(self) -> Result<Response, Error> {
        Response::json(&self)
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Result<Response, Error> {
        Ok(Response::text(self))
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Result<Response, Error> {
        Ok(Response::text(self))
    }
}

impl IntoResponse for Vec<u8> {
    fn into_response(self) -> Result<Response, Error> {
        Ok(Response::from_parts(
            200,
            vec![("content-type".into(), "application/octet-stream".into())],
            self,
        ))
    }
}
