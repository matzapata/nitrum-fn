//! nitrum-fn function runtime — Vercel/Lambda-style HTTP handlers.
//!
//! ```ignore
//! use runtime::{Error, Request};
//! use serde_json::{json, Value};
//!
//! #[runtime::main]
//! fn handler(_req: Request) -> Result<Value, Error> {
//!     Ok(json!({ "message": "Hello, world!" }))
//! }
//! ```
//!
//! Guest ABI (wasm32): export `memory` + `invoke(ptr, len) -> len`. The first
//! `invoke` registers the handler.

mod error;
mod http;
mod register;
mod service;
mod wire;

#[cfg(target_arch = "wasm32")]
mod abi;

pub use error::Error;
pub use http::{IntoResponse, Request, Response};
pub use register::run;
pub use runtime_macros::main;
pub use service::{service_fn, ServiceFn};
pub use wire::{decode_request, decode_response, encode_request, encode_response};

#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
pub use abi::__invoke;
