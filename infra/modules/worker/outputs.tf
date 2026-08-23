output "image" {
  description = "Container image URI the worker task pulls"
  value       = var.image
}

output "ecs_cluster_name" {
  description = "ECS cluster name"
  value       = aws_ecs_cluster.worker.name
}

output "ecs_service_name" {
  description = "ECS service name"
  value       = aws_ecs_service.worker.name
}

output "task_role_arn" {
  description = "Worker task role ARN"
  value       = aws_iam_role.task.arn
}
