//! Well-known OpenTelemetry environment variable names.

/// Standard OpenTelemetry env var for the OTLP collector endpoint.
pub const OTEL_EXPORTER_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// Standard OpenTelemetry env var for the OTLP transport protocol.
pub const OTEL_EXPORTER_OTLP_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";

/// Standard OpenTelemetry env var for `service.name`.
pub const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// `service.namespace` shared by nitrum-fn binaries.
pub const NAMESPACE: &str = "nitrum";
