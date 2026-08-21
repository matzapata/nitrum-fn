use application::error::AppError;
use application::ports::PublishBus;
use async_trait::async_trait;
use aws_sdk_sqs::Client;
use domain::PublishQueuedEvent;

/// Direct SQS send for local Floci (no SNS).
pub struct SqsPublishBus {
    client: Client,
    queue_url: String,
}

impl SqsPublishBus {
    pub fn new(client: Client, queue_url: impl Into<String>) -> Self {
        Self {
            client,
            queue_url: queue_url.into(),
        }
    }
}

#[async_trait]
impl PublishBus for SqsPublishBus {
    async fn publish_queued(&self, event: &PublishQueuedEvent) -> Result<(), AppError> {
        let body = serde_json::to_string(event)
            .map_err(|e| AppError::Storage(format!("serialize publish event: {e}")))?;
        self.client
            .send_message()
            .queue_url(&self.queue_url)
            .message_body(body)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(())
    }
}
