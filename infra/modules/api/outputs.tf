output "api_url" {
  description = "HTTPS URL of the management API"
  value       = "https://${var.api_hostname}"
}

output "api_hostname" {
  description = "FQDN of the management API"
  value       = var.api_hostname
}

output "alb_dns_name" {
  description = "API ALB DNS name"
  value       = aws_lb.api.dns_name
}

output "ecr_repository_url" {
  description = "ECR repository URL for nitrum-fn-api images"
  value       = aws_ecr_repository.api.repository_url
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
