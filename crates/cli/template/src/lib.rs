//! Sample nitrum-fn function.

use runtime::{Error, Request};
use serde_json::{json, Value};

#[runtime::main]
fn handler(_req: Request) -> Result<Value, Error> {
    Ok(json!({
        "message": "Hello, world!",
    }))
}
