use application::error::AppError;
use aws_sdk_sqs::types::QueueAttributeName;
use aws_sdk_sqs::Client;

/// Matches staging Terraform (`visibility_timeout_seconds = 300`).
pub const COMPILE_VISIBILITY_TIMEOUT_SECS: i32 = 300;

/// Create the queue if missing (Floci / local). `queue_url` is
/// `http://host:port/000000000000/<name>` — the last path segment is the name.
pub async fn ensure_queue(client: &Client, queue_url: &str) -> Result<(), AppError> {
    let name = queue_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Storage(format!("cannot parse queue name from {queue_url}")))?;

    let resolved_url = match client.get_queue_url().queue_name(name).send().await {
        Ok(out) => out.queue_url.unwrap_or_else(|| queue_url.to_string()),
        Err(err)
            if err
                .as_service_error()
                .map(|e| e.is_queue_does_not_exist())
                .unwrap_or(false) =>
        {
            match client
                .create_queue()
                .queue_name(name)
                .attributes(
                    QueueAttributeName::VisibilityTimeout,
                    COMPILE_VISIBILITY_TIMEOUT_SECS.to_string(),
                )
                .send()
                .await
            {
                Ok(out) => out.queue_url.unwrap_or_else(|| queue_url.to_string()),
                Err(err)
                    if err
                        .as_service_error()
                        .map(|e| e.is_queue_name_exists())
                        .unwrap_or(false) =>
                {
                    queue_url.to_string()
                }
                Err(err) => return Err(AppError::Storage(err.to_string())),
            }
        }
        Err(err) => return Err(AppError::Storage(err.to_string())),
    };

    set_visibility_timeout(client, &resolved_url).await
}

async fn set_visibility_timeout(client: &Client, queue_url: &str) -> Result<(), AppError> {
    client
        .set_queue_attributes()
        .queue_url(queue_url)
        .attributes(
            QueueAttributeName::VisibilityTimeout,
            COMPILE_VISIBILITY_TIMEOUT_SECS.to_string(),
        )
        .send()
        .await
        .map_err(|e| AppError::Storage(e.to_string()))?;
    Ok(())
}
