//! Generic axum request-metrics middleware (OpenTelemetry HTTP semantic conventions).
//!
//! Records `http.server.request.duration` dimensioned by the matched route
//! template, method, status code, and scheme. Request spans and logs stay on
//! `tower_http::trace::TraceLayer`.

use std::sync::OnceLock;
use std::time::Instant;

use axum::{
    extract::{MatchedPath, Request},
    middleware::{from_fn, Next},
    response::Response,
    Router,
};
use opentelemetry::metrics::Histogram;
use opentelemetry::{global, KeyValue};

static HTTP_DURATION: OnceLock<Histogram<f64>> = OnceLock::new();

fn http_duration() -> &'static Histogram<f64> {
    HTTP_DURATION.get_or_init(|| {
        global::meter("nitrum-fn")
            .f64_histogram("http.server.request.duration")
            .with_description("Duration of HTTP server requests")
            .with_unit("s")
            .build()
    })
}

/// Register HTTP instruments against the global meter provider.
pub fn init_instruments() {
    let _ = http_duration();
}

/// Attach HTTP semconv request metrics to all routes of `router`.
///
/// Apply after routes are registered so the matched-path extension is available.
/// Skips `/healthz`.
pub fn instrument_router<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(from_fn(track))
}

async fn track(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_owned();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "other".to_string(), |m| m.as_str().to_owned());
    let scheme = req.uri().scheme_str().unwrap_or("http").to_owned();

    if route == "/healthz" {
        return next.run(req).await;
    }

    let start = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let elapsed_s = start.elapsed().as_secs_f64();

    let mut attrs = vec![
        KeyValue::new("http.request.method", method),
        KeyValue::new("http.route", route),
        KeyValue::new("http.response.status_code", i64::from(status)),
        KeyValue::new("url.scheme", scheme),
    ];
    if status >= 500 {
        attrs.push(KeyValue::new("error.type", status.to_string()));
    }
    http_duration().record(elapsed_s, &attrs);

    response
}
