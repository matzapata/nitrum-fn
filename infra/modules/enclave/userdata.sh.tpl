Content-Type: multipart/mixed; boundary="//"
MIME-Version: 1.0

--//
Content-Type: text/cloud-config; charset="us-ascii"
MIME-Version: 1.0
Content-Transfer-Encoding: 7bit
Content-Disposition: attachment; filename="cloud-config.txt"

#cloud-config
bootcmd:
  - [ dnf, install, aws-nitro-enclaves-cli, aws-nitro-enclaves-cli-devel, htop, git, jq, -y ]

--//
Content-Type: text/x-shellscript; charset="us-ascii"
MIME-Version: 1.0
Content-Transfer-Encoding: 7bit
Content-Disposition: attachment; filename="userdata.txt"

#!/bin/bash

exec > >(tee /var/log/user-data.log | logger -t user-data -s 2>/dev/console) 2>&1

set -x
set +e

usermod -aG docker ec2-user
usermod -aG ne ec2-user

# Instance id for CloudWatch log stream names below. Docker's `awslogs` driver
# ships container stdout/stderr straight to CloudWatch Logs, independent of
# the OTel/ADOT pipeline (which only starts *after* control-plane, and is
# itself fed by control-plane's own OTLP export). Without this, any failure
# in the first seconds of boot (bad IAM, bad EIF, gvproxy failure, nitro-cli
# failure) is only visible via an SSM session + `docker logs control-plane`.
IMDS_TOKEN=$(curl -sf -X PUT "http://169.254.169.254/latest/api/token" -H "X-aws-ec2-metadata-token-ttl-seconds: 60")
INSTANCE_ID=$(curl -sf -H "X-aws-ec2-metadata-token: $IMDS_TOKEN" http://169.254.169.254/latest/meta-data/instance-id)
INSTANCE_ID="$${INSTANCE_ID:-unknown}"

# Nitro enclaves allocator (must align with enclave_cpu_count / enclave_memory_mib)
ALLOCATOR_YAML=/etc/nitro_enclaves/allocator.yaml
sed -r "s/^(\s*memory_mib\s*:\s*).*/\1 ${enclave_memory_mib}/" -i "$ALLOCATOR_YAML"
sed -r "s/^(\s*cpu_count\s*:\s*).*/\1 ${enclave_cpu_count}/" -i "$ALLOCATOR_YAML"

# Enable services. Restart the allocator after writing yaml: package install
# may have started it with the default (empty) CPU pool.
systemctl enable --now docker
systemctl enable nitro-enclaves-allocator.service
systemctl restart nitro-enclaves-allocator.service
systemctl enable --now nitro-enclaves-vsock-proxy.service

sleep 5

docker pull "${control_plane_image}"

cat > /etc/systemd/system/control-plane.service <<UNIT_EOF
[Unit]
Description=Nitrum control-plane (gvproxy + enclave)
After=docker.service nitro-enclaves-allocator.service
Requires=docker.service
[Service]
Type=simple
Restart=always
RestartSec=10
ExecStartPre=-/usr/bin/docker rm -f control-plane
ExecStart=/usr/bin/docker run --rm --name control-plane --privileged --security-opt seccomp=unconfined -e NITRUM_PROJECT_NAME=${project_name} -e AWS_DEFAULT_REGION=${aws_region} -e NITRUM_OTLP_ENDPOINT=http://127.0.0.1:4317 --log-driver awslogs --log-opt awslogs-region=${aws_region} --log-opt awslogs-group=/nitrum/${project_name}/control-plane --log-opt awslogs-create-group=false --log-opt awslogs-stream=$INSTANCE_ID -p 80:80 -p 443:443 -p 4317:4317 ${control_plane_image} /app/control-plane --eif-bucket ${eif_s3_bucket} --eif-hash ${eif_version_label} --cpu-count ${enclave_cpu_count} --memory-mib ${enclave_memory_mib} ${control_plane_debug_arg}
ExecStop=/usr/bin/docker stop -t 10 control-plane
TimeoutStopSec=30
[Install]
WantedBy=multi-user.target
UNIT_EOF

systemctl daemon-reload
systemctl enable --now control-plane.service

# ── OpenTelemetry Collector (ADOT) ─────────────────────────────────────────────
# Receives OTLP/gRPC on :4317 and exports to CloudWatch (EMF metrics in the
# "Nitrum" namespace), CloudWatch Logs (per-service log groups), and optionally X-Ray
# when enable_xray_tracing=true. Runs in the control-plane container's network namespace.
mkdir -p /opt/nitrum
ENABLE_XRAY="${enable_xray_tracing}"
if [ "$ENABLE_XRAY" = "true" ]; then
cat > /opt/nitrum/otelcol-config.yaml <<'OTEL_EOF'
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
    log_group_name: "/nitrum/${project_name}/metrics"
    log_stream_name: "{ServiceName}"
  awscloudwatchlogs:
    log_group_name: "/nitrum/${project_name}/{ServiceName}"
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
OTEL_EOF
else
cat > /opt/nitrum/otelcol-config.yaml <<'OTEL_EOF'
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
    log_group_name: "/nitrum/${project_name}/metrics"
    log_stream_name: "{ServiceName}"
  awscloudwatchlogs:
    log_group_name: "/nitrum/${project_name}/{ServiceName}"
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
OTEL_EOF
fi

docker pull "${otel_collector_image}"

cat > /etc/systemd/system/otel-collector.service <<UNIT_EOF
[Unit]
Description=Nitrum OpenTelemetry Collector (ADOT)
After=control-plane.service
BindsTo=control-plane.service
[Service]
Type=simple
Restart=always
RestartSec=10
ExecStartPre=-/usr/bin/docker rm -f otel-collector
ExecStart=/usr/bin/docker run --rm --name otel-collector --network container:control-plane -e AWS_REGION=${aws_region} --log-driver awslogs --log-opt awslogs-region=${aws_region} --log-opt awslogs-group=/nitrum/${project_name}/control-plane --log-opt awslogs-create-group=false --log-opt awslogs-stream=$INSTANCE_ID-otel-collector -v /opt/nitrum/otelcol-config.yaml:/etc/otelcol/config.yaml ${otel_collector_image} --config /etc/otelcol/config.yaml
ExecStop=/usr/bin/docker stop -t 10 otel-collector
TimeoutStopSec=30
[Install]
WantedBy=multi-user.target
UNIT_EOF

systemctl daemon-reload
systemctl enable --now otel-collector.service

--//--
