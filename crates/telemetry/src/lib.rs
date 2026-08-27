//! Shared observability for nitrum-fn (OpenTelemetry-first).
//!
//! Every binary initializes telemetry once via [`init`]. Always wires structured
//! logs to stdout (non-ANSI, container-friendly).
//!
//! When an OTLP endpoint is configured, traces, metrics, and logs are also
//! exported over OTLP. Default protocol is **gRPC**
//! (`OTEL_EXPORTER_OTLP_PROTOCOL=grpc`). Set `http/protobuf` for HTTP collectors.
//!
//! With the `http` feature, [`http::instrument_router`] records
//! `http.server.request.duration` (OpenTelemetry HTTP semantic conventions).
//! Product/business metrics are not defined yet.
//!
//! Redaction: only low-cardinality, non-sensitive attributes are ever attached
//! to telemetry. Request/response headers, bodies, and secrets are never recorded.

pub mod env;

#[cfg(feature = "http")]
pub mod http;

mod otel;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::filter::FilterFn;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

/// Telemetry configuration supplied by each binary at startup.
pub struct TelemetryConfig {
    /// `service.name` reported on all exported telemetry (e.g. `nitrum-fn-api`).
    pub service_name: String,
    /// Extra OpenTelemetry resource attributes (e.g. `service.instance.id`).
    pub resource_attributes: Vec<(String, String)>,
    /// OTLP collector endpoint (e.g. `http://127.0.0.1:4317`).
    /// When `None`, only stdout logging is configured (local dev / tests).
    pub otlp_endpoint: Option<String>,
}

impl TelemetryConfig {
    /// Create telemetry config for `service_name` with stdout logging only.
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            resource_attributes: vec![("service.namespace".into(), env::NAMESPACE.into())],
            otlp_endpoint: None,
        }
    }

    /// Attach an OTLP collector endpoint when export is enabled.
    #[must_use]
    pub fn with_otlp_endpoint(mut self, endpoint: Option<impl AsRef<str>>) -> Self {
        self.otlp_endpoint = endpoint.map(|e| e.as_ref().to_string());
        self
    }

    /// Attach OpenTelemetry resource attributes (e.g. `service.instance.id`).
    #[must_use]
    pub fn with_resource_attributes(
        mut self,
        attributes: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.resource_attributes.extend(
            attributes
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }
}

/// Owns the OpenTelemetry providers so they stay alive for the process lifetime.
///
/// Flushes and stops all OTLP exporters on [`Drop`]. Call [`shutdown`] explicitly
/// before [`std::process::exit`], which skips destructors.
#[derive(Default)]
pub struct TelemetryGuard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<SdkLoggerProvider>,
}

impl TelemetryGuard {
    fn shutdown_providers(&mut self) {
        if let Some(provider) = self.tracer_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.meter_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.logger_provider.take() {
            let _ = provider.shutdown();
        }
    }

    /// Flush and stop all OTLP exporters before the guard is dropped.
    pub fn shutdown(self) {
        drop(self);
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        self.shutdown_providers();
    }
}

fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

fn service_name(cfg: &TelemetryConfig) -> String {
    std::env::var(env::OTEL_SERVICE_NAME)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.service_name.clone())
}

/// Initialize process-wide telemetry and return a guard that flushes on shutdown.
///
/// Always installs a stdout logging layer. When `cfg.otlp_endpoint` is set, also
/// installs OTLP trace and log layers, builds and registers the global OTLP meter
/// provider, and returns the providers in the guard. If the OTLP exporters cannot
/// be built, telemetry degrades to stdout-only logging rather than failing.
///
/// Must be called from within a Tokio runtime: the OTLP gRPC exporters require one.
#[must_use]
pub fn init(cfg: TelemetryConfig) -> TelemetryGuard {
    let service_name = service_name(&cfg);
    let guard = if let Some(endpoint) = cfg.otlp_endpoint.as_deref().filter(|e| !e.is_empty()) {
        let resource = otel::build_resource(&service_name, &cfg.resource_attributes);
        match otel::build_providers(endpoint, resource) {
            Ok(providers) => {
                let guard = install_with_otlp(providers);
                tracing::info!(
                    %endpoint,
                    %service_name,
                    "OTLP traces/metrics/logs exporters enabled"
                );
                guard
            }
            Err(error) => {
                eprintln!(
                    "telemetry: OTLP exporters disabled ({error:#}); falling back to stdout-only logging"
                );
                init_stdout_only();
                TelemetryGuard::default()
            }
        }
    } else {
        init_stdout_only();
        tracing::info!("OTEL_EXPORTER_OTLP_ENDPOINT unset — OTLP export disabled");
        TelemetryGuard::default()
    };
    #[cfg(feature = "http")]
    http::init_instruments();
    guard
}

fn init_stdout_only() {
    tracing_subscriber::registry()
        .with(env_filter())
        .with(fmt::layer().with_ansi(false))
        .init();
}

fn install_with_otlp(providers: otel::Providers) -> TelemetryGuard {
    let trace_layer =
        tracing_opentelemetry::layer().with_tracer(providers.tracer_provider.tracer("nitrum-fn"));
    // Filter SDK/internal targets out of the bridge. Their own AfterShutdown
    // warnings are emitted via `tracing`; feeding them back into the logger
    // provider recurses until the tokio worker stack overflows.
    let logs_layer = OpenTelemetryTracingBridge::new(&providers.logger_provider).with_filter(
        FilterFn::new(|metadata| {
            let target = metadata.target();
            !(target.starts_with("opentelemetry") || target.starts_with("tonic"))
        }),
    );

    opentelemetry::global::set_tracer_provider(providers.tracer_provider.clone());
    opentelemetry::global::set_meter_provider(providers.meter_provider.clone());

    tracing_subscriber::registry()
        .with(env_filter())
        .with(fmt::layer().with_ansi(false))
        .with(trace_layer)
        .with(logs_layer)
        .init();

    TelemetryGuard {
        tracer_provider: Some(providers.tracer_provider),
        meter_provider: Some(providers.meter_provider),
        logger_provider: Some(providers.logger_provider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Key;

    #[test]
    fn build_resource_includes_service_name_and_attributes() {
        let resource = otel::build_resource(
            "nitrum-fn-api",
            &[("service.instance.id".to_string(), "i-123".to_string())],
        );

        assert_eq!(
            resource
                .get(&Key::from_static_str("service.name"))
                .map(|v| v.to_string()),
            Some("nitrum-fn-api".to_string())
        );
        assert_eq!(
            resource
                .get(&Key::from_static_str("service.instance.id"))
                .map(|v| v.to_string()),
            Some("i-123".to_string())
        );
    }

    #[test]
    fn init_without_otlp_endpoint_is_stdout_only_and_does_not_panic() {
        let guard = init(TelemetryConfig::new("test"));
        assert!(guard.tracer_provider.is_none());
        assert!(guard.meter_provider.is_none());
        assert!(guard.logger_provider.is_none());
    }
}
