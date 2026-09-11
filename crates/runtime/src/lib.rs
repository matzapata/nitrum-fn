//! nitrum-fn function runtime — Vercel/Lambda-style HTTP handlers.
//!
//! ```ignore
//! use runtime::http::{Client, Request};
//! use runtime::Error;
//! use serde_json::{json, Value};
//!
//! #[runtime::main]
//! async fn handler(req: Request) -> Result<Value, Error> {
//!     let upstream = Client::new()
//!         .get("https://example.com/data")
//!         .send()
//!         .await?
//!         .json::<Value>()?;
//!     Ok(json!({ "upstream": upstream }))
//! }
//! ```
//!
//! Guest ABI (wasm32): export `memory` + `invoke(ptr, len) -> len`. The first
//! `invoke` registers the handler.

mod block_on;
mod error;
mod host_fetch;
pub mod http;
mod register;
mod service;
mod wire;

#[cfg(target_arch = "wasm32")]
mod abi;

pub use block_on::block_on;
pub use error::Error;
pub use host_fetch::get;
pub use http::{
    Client, IntoResponse, PendingRequest, Request, RequestBuilder, Response, ResponseBuilder,
};
pub use register::run;
pub use runtime_macros::main;
pub use service::{service_fn, ServiceFn};
pub use wire::{decode_request, decode_response, encode_request, encode_response};

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use abi::__invoke;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_get_unavailable_on_native() {
        let err = block_on(async {
            Client::new()
                .get("https://example.com")
                .send()
                .await
        })
        .expect_err("native");
        assert!(err.to_string().contains("wasm32"));
    }
}
