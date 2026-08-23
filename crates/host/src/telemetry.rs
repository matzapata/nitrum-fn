//! OpenTelemetry metrics for the invoke path.
//!
//! Enabled when `OTEL_EXPORTER_OTLP_ENDPOINT` is set (Nitrum injects this for
//! `start_command`). Default protocol is **gRPC** (`OTEL_EXPORTER_OTLP_PROTOCOL=grpc`),
//! matching Nitrum’s ADOT collector on `:4317`. Set `http/protobuf` only for ad-hoc
//! HTTP collectors.

use std::sync::OnceLock;
use std::time::Instant;

use anyhow::{Context, Result};
use application::AppError;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{MetricExporter, Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::Resource;
use tracing::{info, warn};

static INVOKE_METRICS: OnceLock<InvokeMetrics> = OnceLock::new();

pub struct Telemetry {
    meter_provider: Option<SdkMeterProvider>,
}

struct InvokeMetrics {
    calls: Counter<u64>,
    duration_ms: Histogram<f64>,
    traps: Counter<u64>,
}

impl Telemetry {
    /// No-op metrics (used when OTLP is unset or exporter setup fails).
    fn disabled() -> Self {
        let _ = INVOKE_METRICS.get_or_init(InvokeMetrics::noop);
        Self {
            meter_provider: None,
        }
    }

    /// Init OTLP metrics when `OTEL_EXPORTER_OTLP_ENDPOINT` is set.
    ///
    /// Never fails startup: Nitrum injects OTLP into every guest, and a bad
    /// exporter must not take down the process (the data-plane exits with it).
    pub fn init() -> Self {
        match Self::try_init() {
            Ok(telemetry) => telemetry,
            Err(err) => {
                warn!(error = %format!("{err:#}"), "OTLP metrics disabled");
                Self::disabled()
            }
        }
    }

    fn try_init() -> Result<Self> {
        let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty());

        let Some(endpoint) = endpoint else {
            info!("OTEL_EXPORTER_OTLP_ENDPOINT unset — invoke metrics disabled");
            return Ok(Self::disabled());
        };

        let service_name = std::env::var("OTEL_SERVICE_NAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "nitrum-fn-host".into());

        let protocol = std::env::var("OTEL_EXPORTER_OTLP_PROTOCOL")
            .unwrap_or_else(|_| "grpc".into())
            .to_ascii_lowercase();

        // Endpoint / protocol from standard OTEL_* env (Nitrum injects these).
        let exporter = match protocol.as_str() {
            "http/protobuf" | "http/json" | "http" => MetricExporter::builder()
                .with_http()
                .with_protocol(if protocol == "http/json" {
                    Protocol::HttpJson
                } else {
                    Protocol::HttpBinary
                })
                .build()
                .context("build OTLP HTTP metrics exporter")?,
            // Nitrum default: gRPC → ADOT on parent :4317
            _ => MetricExporter::builder()
                .with_tonic()
                .with_protocol(Protocol::Grpc)
                .build()
                .context("build OTLP gRPC metrics exporter")?,
        };

        let resource = Resource::builder()
            .with_service_name(service_name.clone())
            .build();

        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter)
            .with_resource(resource)
            .build();

        global::set_meter_provider(provider.clone());
        let _ = INVOKE_METRICS.get_or_init(InvokeMetrics::from_global);

        info!(%endpoint, %protocol, %service_name, "OTLP metrics exporter enabled");
        Ok(Self {
            meter_provider: Some(provider),
        })
    }

    pub fn shutdown(self) {
        if let Some(provider) = self.meter_provider {
            if let Err(err) = provider.shutdown() {
                tracing::warn!(error = %err, "OTLP meter provider shutdown failed");
            }
        }
    }
}

impl InvokeMetrics {
    fn from_global() -> Self {
        let meter = global::meter("nitrum-fn");
        Self {
            calls: meter
                .u64_counter("nitrum_fn.invoke.calls")
                .with_description("Invoke attempts by function and outcome")
                .build(),
            duration_ms: meter
                .f64_histogram("nitrum_fn.invoke.duration_ms")
                .with_description("Invoke wall time in milliseconds")
                .with_unit("ms")
                .build(),
            traps: meter
                .u64_counter("nitrum_fn.invoke.traps")
                .with_description("Guest Wasmtime traps during invoke")
                .build(),
        }
    }

    /// Instruments against the default global provider (no export).
    fn noop() -> Self {
        Self::from_global()
    }
}

fn metrics() -> &'static InvokeMetrics {
    INVOKE_METRICS.get_or_init(InvokeMetrics::noop)
}

pub fn outcome_for_error(err: &AppError) -> &'static str {
    match err {
        AppError::NotFound(_) | AppError::ArtifactMissing(_) => "not_found",
        AppError::Trap(_) => "trap",
        _ => "error",
    }
}

/// Record one invoke attempt. Call from the HTTP edge after the request finishes.
pub fn record_invoke(function: &str, outcome: &str, started: Instant) {
    let m = metrics();
    let attrs = [
        KeyValue::new("function", function.to_string()),
        KeyValue::new("outcome", outcome.to_string()),
    ];
    m.calls.add(1, &attrs);
    m.duration_ms
        .record(started.elapsed().as_secs_f64() * 1000.0, &attrs);
    if outcome == "trap" {
        m.traps
            .add(1, &[KeyValue::new("function", function.to_string())]);
    }
}
