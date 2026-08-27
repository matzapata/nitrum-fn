data "aws_region" "current" {}

locals {
  container_name      = "nitrum-fn-api"
  container_port      = 8080
  otel_container_name = "aws-otel-collector"

  # ADOT config: OTLP in → CloudWatch EMF + Logs (+ optional X-Ray).
  # Matches the enclave host collector so Fargate and Nitro share backends.
  otel_config = var.enable_xray_tracing ? local.otel_config_xray : local.otel_config_base

  otel_config_xray = <<-YAML
    receivers:
      otlp:
        protocols:
          grpc:
            endpoint: 0.0.0.0:4317
    processors:
      batch:
    exporters:
      awsemf:
        namespace: Nitrum
        log_group_name: "${var.metrics_log_group_name}"
        log_stream_name: "{ServiceName}"
      awscloudwatchlogs:
        log_group_name: "/nitrum/${var.project_name}/{ServiceName}"
        log_stream_name: "otel"
      awsxray: {}
    service:
      pipelines:
        traces:
          receivers: [otlp]
          processors: [batch]
          exporters: [awsxray]
        metrics:
          receivers: [otlp]
          processors: [batch]
          exporters: [awsemf]
        logs:
          receivers: [otlp]
          processors: [batch]
          exporters: [awscloudwatchlogs]
  YAML

  otel_config_base = <<-YAML
    receivers:
      otlp:
        protocols:
          grpc:
            endpoint: 0.0.0.0:4317
    processors:
      batch:
    exporters:
      awsemf:
        namespace: Nitrum
        log_group_name: "${var.metrics_log_group_name}"
        log_stream_name: "{ServiceName}"
      awscloudwatchlogs:
        log_group_name: "/nitrum/${var.project_name}/{ServiceName}"
        log_stream_name: "otel"
    service:
      pipelines:
        metrics:
          receivers: [otlp]
          processors: [batch]
          exporters: [awsemf]
        logs:
          receivers: [otlp]
          processors: [batch]
          exporters: [awscloudwatchlogs]
  YAML
}
