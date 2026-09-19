//! Outbound HTTP client (`Client::new().get(url).send().await`).

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use crate::host_fetch;
use crate::Error;

use super::Response;

/// Outbound HTTP client backed by the host `nitrum.http_get` import.
#[derive(Debug, Default, Clone, Copy)]
pub struct Client;

impl Client {
    pub fn new() -> Self {
        Self
    }

    /// Start building a GET request.
    pub fn get(self, url: impl Into<String>) -> RequestBuilder {
        RequestBuilder { url: url.into() }
    }
}

/// Outbound GET request builder.
#[derive(Debug, Clone)]
pub struct RequestBuilder {
    url: String,
}

impl RequestBuilder {
    /// Send the request. Completes on the first poll (host fetch is synchronous).
    pub fn send(self) -> PendingRequest {
        PendingRequest { url: self.url }
    }
}

/// A single outbound request in flight.
#[derive(Debug, Clone)]
pub struct PendingRequest {
    url: String,
}

impl Future for PendingRequest {
    type Output = Result<Response, Error>;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let url = self.url.clone();
        Poll::Ready(host_fetch::fetch_get(&url))
    }
}
