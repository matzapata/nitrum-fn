use std::sync::RwLock;

use crate::http::{IntoResponse, Request, Response};
use crate::service::ServiceFn;
use crate::Error;

type BoxedHandler = Box<dyn Fn(Request) -> Result<Response, Error> + Send + Sync>;

static HANDLER: RwLock<Option<BoxedHandler>> = RwLock::new(None);

/// Register `service` as this module's entrypoint (called once on first `invoke`).
pub fn run<F, R>(service: ServiceFn<F>)
where
    F: Fn(Request) -> Result<R, Error> + Send + Sync + 'static,
    R: IntoResponse + 'static,
{
    let mut slot = HANDLER.write().unwrap_or_else(|e| e.into_inner());
    *slot = Some(Box::new(move |req| service.call(req)));
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn call_handler(req: Request) -> Result<Response, Error> {
    let slot = HANDLER.read().unwrap_or_else(|e| e.into_inner());
    let Some(handler) = slot.as_ref() else {
        return Err(Error::from_message(
            "runtime not initialized — #[runtime::main] registers the handler on first invoke",
        ));
    };
    handler(req)
}
