resource "aws_ecs_cluster" "worker" {
  name = "${var.project_name}-worker"
}

resource "aws_ecs_task_definition" "worker" {
  family                   = "${var.project_name}-worker"
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
      environment = [
        { name = "NITRUM_FN_ENV", value = "prod" },
        { name = "NITRUM_FN_ARTIFACTS__BUCKET", value = var.artifacts_bucket_name },
        { name = "NITRUM_FN_CATALOG__TABLE", value = var.catalog_table_name },
        { name = "NITRUM_FN_CATALOG__PUBLISH_LOCK_TABLE", value = var.publish_lock_table_name },
        { name = "NITRUM_FN_COMPILE__QUEUE_URL", value = var.compile_queue_url },
        { name = "AWS_REGION", value = data.aws_region.current.name },
        { name = "OTEL_SERVICE_NAME", value = "nitrum-fn-publish-worker" },
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.worker.name
          "awslogs-region"        = data.aws_region.current.name
          "awslogs-stream-prefix" = "worker"
        }
      }
    }
  ])
}

resource "aws_ecs_service" "worker" {
  name            = "${var.project_name}-worker"
  cluster         = aws_ecs_cluster.worker.id
  task_definition = aws_ecs_task_definition.worker.arn
  desired_count   = var.desired_count
  launch_type     = "FARGATE"

  network_configuration {
    subnets          = var.private_subnet_ids
    security_groups  = [aws_security_group.tasks.id]
    assign_public_ip = false
  }
}
