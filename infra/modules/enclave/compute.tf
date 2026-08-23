resource "aws_launch_template" "nitro" {
  name = "${var.project_name}-${var.eif_version_label}"

  image_id      = local.ami_id
  instance_type = var.instance_type

  iam_instance_profile {
    arn = aws_iam_instance_profile.instance.arn
  }

  vpc_security_group_ids = [aws_security_group.instance.id]

  metadata_options {
    http_tokens            = "required"
    http_endpoint          = "enabled"
    instance_metadata_tags = "enabled"
  }

  enclave_options {
    enabled = true
  }

  block_device_mappings {
    device_name = "/dev/xvda"
    ebs {
      volume_size           = 32
      volume_type           = "gp3"
      encrypted             = true
      delete_on_termination = !var.retain
    }
  }

  user_data = base64encode(templatefile("${path.module}/userdata.sh.tpl", {
    project_name            = var.project_name
    aws_region              = data.aws_region.current.name
    enclave_memory_mib      = var.enclave_memory_mib
    enclave_cpu_count       = var.enclave_cpu_count
    control_plane_image     = var.control_plane_image
    eif_s3_bucket           = var.eif_s3_bucket
    eif_version_label       = var.eif_version_label
    control_plane_debug_arg = var.control_plane_debug_arg
    enable_xray_tracing     = var.enable_xray_tracing ? "true" : "false"
    otel_collector_image    = var.otel_collector_image
  }))

  lifecycle {
    create_before_destroy = true
  }
}

resource "aws_autoscaling_group" "nitro" {
  # Instances download the EIF at boot; do not launch until the object exists.
  depends_on = [aws_s3_object.eif]

  min_size                  = var.asg_min_size
  max_size                  = var.asg_max_size
  desired_capacity          = var.asg_desired_capacity
  vpc_zone_identifier       = var.private_subnet_ids
  target_group_arns         = [aws_lb_target_group.https.arn, aws_lb_target_group.http.arn]
  health_check_type         = "ELB"
  health_check_grace_period = 300

  launch_template {
    id      = aws_launch_template.nitro.id
    version = aws_launch_template.nitro.latest_version
  }

  instance_refresh {
    strategy = "Rolling"
    preferences {
      min_healthy_percentage = local.instance_refresh_min_healthy
      max_healthy_percentage = local.instance_refresh_max_healthy
      instance_warmup        = 300
    }
    triggers = ["launch_template"]
  }

  tag {
    key                 = "Name"
    value               = var.project_name
    propagate_at_launch = true
  }
}
