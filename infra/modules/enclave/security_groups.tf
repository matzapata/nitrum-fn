resource "aws_security_group" "nlb" {
  name        = "${var.project_name} NLB"
  description = "Network Load Balancer - HTTP/HTTPS from internet, egress to targets (${var.project_name})"
  vpc_id      = var.vpc_id

  ingress {
    description      = "HTTP from internet (IPv4, ACME HTTP-01)"
    from_port        = 80
    to_port          = 80
    protocol         = "tcp"
    cidr_blocks      = ["0.0.0.0/0"]
    ipv6_cidr_blocks = ["::/0"]
  }

  ingress {
    description      = "HTTPS from internet (IPv4)"
    from_port        = 443
    to_port          = 443
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
    Name = "${var.project_name} NLB Security Group"
  }
}

resource "aws_security_group" "instance" {
  name        = "${var.project_name} Nitro Instance"
  description = "Nitro Enclave EC2 instances - HTTP/HTTPS, NLB, and VPC access (${var.project_name})"
  vpc_id      = var.vpc_id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "${var.project_name} Instance Security Group"
  }
}

resource "aws_vpc_security_group_ingress_rule" "instance_vpc_443" {
  security_group_id = aws_security_group.instance.id
  ip_protocol       = "tcp"
  from_port         = 443
  to_port           = 443
  cidr_ipv4         = var.vpc_cidr_block
  description       = "Allow HTTPS from within VPC"
}

resource "aws_vpc_security_group_ingress_rule" "instance_nlb_443" {
  security_group_id            = aws_security_group.instance.id
  ip_protocol                  = "tcp"
  from_port                    = 443
  to_port                      = 443
  referenced_security_group_id = aws_security_group.nlb.id
  description                  = "HTTPS from NLB (health checks + traffic)"
}

resource "aws_vpc_security_group_ingress_rule" "instance_self_443" {
  security_group_id            = aws_security_group.instance.id
  ip_protocol                  = "tcp"
  from_port                    = 443
  to_port                      = 443
  referenced_security_group_id = aws_security_group.instance.id
  description                  = "Intra-SG HTTPS"
}

resource "aws_vpc_security_group_ingress_rule" "instance_self_ping" {
  security_group_id            = aws_security_group.instance.id
  ip_protocol                  = "icmp"
  from_port                    = 8
  to_port                      = -1
  referenced_security_group_id = aws_security_group.instance.id
  description                  = "Intra-SG ping"
}

resource "aws_vpc_security_group_ingress_rule" "instance_direct_443" {
  security_group_id = aws_security_group.instance.id
  ip_protocol       = "tcp"
  from_port         = 443
  to_port           = 443
  cidr_ipv4         = "0.0.0.0/0"
  description       = "HTTPS (optional direct / debugging)"
}

resource "aws_vpc_security_group_ingress_rule" "instance_vpc_80" {
  security_group_id = aws_security_group.instance.id
  ip_protocol       = "tcp"
  from_port         = 80
  to_port           = 80
  cidr_ipv4         = var.vpc_cidr_block
  description       = "HTTP from within VPC (ACME HTTP-01)"
}

resource "aws_vpc_security_group_ingress_rule" "instance_nlb_80" {
  security_group_id            = aws_security_group.instance.id
  ip_protocol                  = "tcp"
  from_port                    = 80
  to_port                      = 80
  referenced_security_group_id = aws_security_group.nlb.id
  description                  = "HTTP from NLB (ACME HTTP-01)"
}

resource "aws_vpc_security_group_ingress_rule" "instance_self_80" {
  security_group_id            = aws_security_group.instance.id
  ip_protocol                  = "tcp"
  from_port                    = 80
  to_port                      = 80
  referenced_security_group_id = aws_security_group.instance.id
  description                  = "Intra-SG HTTP"
}

resource "aws_vpc_security_group_ingress_rule" "instance_direct_80" {
  security_group_id = aws_security_group.instance.id
  ip_protocol       = "tcp"
  from_port         = 80
  to_port           = 80
  cidr_ipv4         = "0.0.0.0/0"
  description       = "HTTP (optional direct / debugging, ACME)"
}
