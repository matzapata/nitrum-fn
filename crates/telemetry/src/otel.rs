//! OpenTelemetry OTLP exporter and provider wiring.
//!
//! Builds the three OTLP exporters (traces, metrics, logs) and their SDK
//! providers from a collector endpoint. The providers are owned by the
//! caller (stored in the telemetry guard) so they stay alive for export and can
//! be flushed on shutdown.

use anyhow::{Context, Result};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

use crate::env;

/// The set of OTLP providers backing traces, metrics, and logs export.
pub struct Providers {
    pub tracer_provider: SdkTracerProvider,
    pub meter_provider: SdkMeterProvider,
    pub logger_provider: SdkLoggerProvider,
}

enum Transport {
    Grpc,
    HttpBinary,
    HttpJson,
}

impl Transport {
    fn from_env() -> Self {
        match std::env::var(env::OTEL_EXPORTER_OTLP_PROTOCOL)
            .unwrap_or_else(|_| "grpc".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "http/protobuf" | "http" => Self::HttpBinary,
            "http/json" => Self::HttpJson,
            _ => Self::Grpc,
        }
    }
}

/// Build the OpenTelemetry resource describing this service instance.
pub fn build_resource(service_name: &str, attributes: &[(String, String)]) -> Resource {
    Resource::builder()
        .with_service_name(service_name.to_string())
        .with_attributes(
            attributes
                .iter()
                .map(|(k, v)| KeyValue::new(k.clone(), v.clone())),
        )
        .build()
}

/// Build the trace, metric, and log providers exporting to `endpoint`.
pub fn build_providers(endpoint: &str, resource: Resource) -> Result<Providers> {
    let transport = Transport::from_env();
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter(endpoint, &transport)?)
        .with_resource(resource.clone())
        .build();

    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter(endpoint, &transport)?)
        .with_resource(resource.clone())
        .build();

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(log_exporter(endpoint, &transport)?)
        .with_resource(resource)
        .build();

    Ok(Providers {
        tracer_provider,
        meter_provider,
        logger_provider,
    })
}

fn span_exporter(
    endpoint: &str,
    transport: &Transport,
) -> Result<opentelemetry_otlp::SpanExporter> {
    match transport {
        Transport::HttpJson => opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpJson)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP HTTP span exporter"),
        Transport::HttpBinary => opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP HTTP span exporter"),
        Transport::Grpc => opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_protocol(Protocol::Grpc)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP gRPC span exporter"),
    }
}

fn metric_exporter(
    endpoint: &str,
    transport: &Transport,
) -> Result<opentelemetry_otlp::MetricExporter> {
    match transport {
        Transport::HttpJson => opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpJson)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP HTTP metrics exporter"),
        Transport::HttpBinary => opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP HTTP metrics exporter"),
        Transport::Grpc => opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_protocol(Protocol::Grpc)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP gRPC metrics exporter"),
    }
}

fn log_exporter(endpoint: &str, transport: &Transport) -> Result<opentelemetry_otlp::LogExporter> {
    match transport {
        Transport::HttpJson => opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpJson)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP HTTP log exporter"),
        Transport::HttpBinary => opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP HTTP log exporter"),
        Transport::Grpc => opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_protocol(Protocol::Grpc)
            .with_endpoint(endpoint)
            .build()
            .context("build OTLP gRPC log exporter"),
    }
}
