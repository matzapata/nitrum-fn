use application::error::AppError;
use application::ports::PublishBus;
use async_trait::async_trait;
use aws_sdk_sns::Client;
use domain::PublishQueuedEvent;

/// Publishes `PublishQueuedEvent` to an SNS topic (SQS subscribers compile).
pub struct SnsPublishBus {
    client: Client,
    topic_arn: String,
}

impl SnsPublishBus {
    pub fn new(client: Client, topic_arn: impl Into<String>) -> Self {
        Self {
            client,
            topic_arn: topic_arn.into(),
        }
    }
}

#[async_trait]
impl PublishBus for SnsPublishBus {
    async fn publish_queued(&self, event: &PublishQueuedEvent) -> Result<(), AppError> {
        let body = serde_json::to_string(event)
            .map_err(|e| AppError::Storage(format!("serialize publish event: {e}")))?;
        self.client
            .publish()
            .topic_arn(&self.topic_arn)
            .message(body)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(())
    }
}
