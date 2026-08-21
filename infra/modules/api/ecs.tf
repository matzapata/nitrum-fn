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
      image     = local.image
      essential = true
      portMappings = [
        {
          containerPort = local.container_port
          protocol      = "tcp"
        }
      ]
      environment = [
        { name = "NITRUM_FN_STORE", value = "aws" },
        { name = "NITRUM_FN_S3_BUCKET", value = var.artifacts_bucket_name },
        { name = "NITRUM_FN_DDB_TABLE", value = var.catalog_table_name },
        { name = "NITRUM_FN_PORT", value = tostring(local.container_port) },
        { name = "AWS_REGION", value = data.aws_region.current.name },
        { name = "OTEL_SERVICE_NAME", value = "nitrum-fn-api" },
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.api.name
          "awslogs-region"        = data.aws_region.current.name
          "awslogs-stream-prefix" = "api"
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
