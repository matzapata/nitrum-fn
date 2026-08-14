//! Management HTTP routes. Merged into the host binary for the local MVP.

mod error;
mod http;
mod state;

pub use http::router;
pub use state::ApiState;
