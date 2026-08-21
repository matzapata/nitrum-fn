resource "aws_security_group" "alb" {
  name        = "${var.project_name} API ALB"
  description = "Management API ALB (${var.project_name})"
  vpc_id      = var.vpc_id

  ingress {
    description      = "HTTP (publish / catalog; no custom DNS)"
    from_port        = 80
    to_port          = 80
    protocol         = "tcp"
    cidr_blocks      = ["0.0.0.0/0"]
    ipv6_cidr_blocks = ["::/0"]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "${var.project_name} API ALB"
  }
}

resource "aws_security_group" "tasks" {
  name        = "${var.project_name} API tasks"
  description = "nitrum-fn-api Fargate tasks (${var.project_name})"
  vpc_id      = var.vpc_id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "${var.project_name} API tasks"
  }
}

resource "aws_vpc_security_group_ingress_rule" "tasks_from_alb" {
  security_group_id            = aws_security_group.tasks.id
  ip_protocol                  = "tcp"
  from_port                    = local.container_port
  to_port                      = local.container_port
  referenced_security_group_id = aws_security_group.alb.id
  description                  = "HTTP from API ALB"
}
