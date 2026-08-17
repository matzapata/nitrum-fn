resource "aws_lb" "nlb" {
  name                             = "${var.project_name}-nlb"
  load_balancer_type               = "network"
  internal                         = false
  subnets                          = var.public_subnet_ids
  security_groups                  = [aws_security_group.nlb.id]
  enable_cross_zone_load_balancing = true

  tags = {
    Name = "${var.project_name}-nlb"
  }
}

resource "aws_lb_target_group" "https" {
  name        = "${var.project_name}-tg"
  vpc_id      = var.vpc_id
  protocol    = "TCP"
  port        = 443
  target_type = "instance"

  health_check {
    protocol            = "HTTPS"
    port                = "443"
    path                = "/.well-known/enclave/status"
    matcher             = "200"
    interval            = 30
    timeout             = 10
    healthy_threshold   = 2
    unhealthy_threshold = 5
  }

  tags = {
    Name = "${var.project_name}-tg"
  }
}

resource "aws_lb_target_group" "http" {
  name        = "${var.project_name}-tg-http"
  vpc_id      = var.vpc_id
  protocol    = "TCP"
  port        = 80
  target_type = "instance"

  health_check {
    protocol            = "HTTPS"
    port                = "443"
    path                = "/.well-known/enclave/status"
    matcher             = "200"
    interval            = 30
    timeout             = 10
    healthy_threshold   = 2
    unhealthy_threshold = 5
  }

  tags = {
    Name = "${var.project_name}-tg-http"
  }
}

resource "aws_lb_listener" "https" {
  load_balancer_arn = aws_lb.nlb.arn
  protocol          = "TCP"
  port              = 443

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.https.arn
  }
}

resource "aws_lb_listener" "http" {
  load_balancer_arn = aws_lb.nlb.arn
  protocol          = "TCP"
  port              = 80

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.http.arn
  }
}
