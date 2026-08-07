use crate::http::{IntoResponse, Request, Response};
use crate::Error;

/// Wrap a handler function the same way Vercel's `service_fn` does.
pub fn service_fn<F, R>(f: F) -> ServiceFn<F>
where
    F: Fn(Request) -> Result<R, Error>,
    R: IntoResponse,
{
    ServiceFn { f }
}

pub struct ServiceFn<F> {
    f: F,
}

impl<F, R> ServiceFn<F>
where
    F: Fn(Request) -> Result<R, Error>,
    R: IntoResponse,
{
    pub fn call(&self, req: Request) -> Result<Response, Error> {
        (self.f)(req)?.into_response()
    }
}
