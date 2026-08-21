resource "aws_security_group" "tasks" {
  name        = "${var.project_name} publish-worker tasks"
  description = "nitrum-fn-publish-worker Fargate tasks (${var.project_name})"
  vpc_id      = var.vpc_id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "${var.project_name} publish-worker tasks"
  }
}
