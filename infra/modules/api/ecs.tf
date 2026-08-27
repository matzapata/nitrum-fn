resource "aws_ecs_cluster" "api" {
  name = "${var.project_name}-api"
}

resource "aws_ecs_task_definition" "api" {
  family                   = "${var.project_name}-api"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.cpu
  memory                   = var.memory
  execution_role_arn       = aws_iam_role.execution.arn
  task_role_arn            = aws_iam_role.task.arn

  runtime_platform {
    operating_system_family = "LINUX"
    cpu_architecture        = "X86_64"
  }

  container_definitions = jsonencode([
    {
      name      = local.container_name
      image     = var.image
      essential = true
      portMappings = [
        {
          containerPort = local.container_port
          protocol      = "tcp"
        }
      ]
      environment = [
        { name = "NITRUM_FN_ENV", value = "prod" },
        { name = "NITRUM_FN_ARTIFACTS__BUCKET", value = var.artifacts_bucket_name },
        { name = "NITRUM_FN_CATALOG__TABLE", value = var.catalog_table_name },
        { name = "NITRUM_FN_CATALOG__PUBLISH_LOCK_TABLE", value = var.publish_lock_table_name },
        { name = "NITRUM_FN_PUBLISH__TOPIC_ARN", value = var.publish_topic_arn },
        { name = "NITRUM_FN_SERVER__PORT", value = tostring(local.container_port) },
        { name = "AWS_REGION", value = data.aws_region.current.name },
        { name = "OTEL_SERVICE_NAME", value = "nitrum-fn-api" },
        { name = "OTEL_EXPORTER_OTLP_ENDPOINT", value = "http://127.0.0.1:4317" },
        { name = "OTEL_EXPORTER_OTLP_PROTOCOL", value = "grpc" },
      ]
      dependsOn = [
        {
          containerName = local.otel_container_name
          condition     = "START"
        }
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.api.name
          "awslogs-region"        = data.aws_region.current.name
          "awslogs-stream-prefix" = "api"
        }
      }
    },
    {
      name      = local.otel_container_name
      image     = var.otel_collector_image
      essential = true
      command   = ["--config=env:AOT_CONFIG_CONTENT"]
      environment = [
        { name = "AOT_CONFIG_CONTENT", value = local.otel_config },
        { name = "AWS_REGION", value = data.aws_region.current.name },
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.api.name
          "awslogs-region"        = data.aws_region.current.name
          "awslogs-stream-prefix" = "otel"
        }
      }
    }
  ])
}

resource "aws_ecs_service" "api" {
  name            = "${var.project_name}-api"
  cluster         = aws_ecs_cluster.api.id
  task_definition = aws_ecs_task_definition.api.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  network_configuration {
    subnets          = var.private_subnet_ids
    security_groups  = [aws_security_group.tasks.id]
    assign_public_ip = false
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.api.arn
    container_name   = local.container_name
    container_port   = local.container_port
  }

  depends_on = [aws_lb_listener.http]
}
