output "api_url" {
  description = "HTTP URL of the management API (ALB DNS; no custom hostname)"
  value       = "http://${aws_lb.api.dns_name}"
}

output "alb_dns_name" {
  description = "API ALB DNS name"
  value       = aws_lb.api.dns_name
}

output "image" {
  description = "Container image URI the API task pulls"
  value       = var.image
}

output "ecs_cluster_name" {
  description = "ECS cluster name"
  value       = aws_ecs_cluster.api.name
}

output "ecs_service_name" {
  description = "ECS service name"
  value       = aws_ecs_service.api.name
}

output "task_role_arn" {
  description = "API task role ARN"
  value       = aws_iam_role.task.arn
}
