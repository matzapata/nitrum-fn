use async_trait::async_trait;
use domain::PublishQueuedEvent;

use crate::AppError;

/// One queued compile job as delivered from SQS (after SNS unwrap if needed).
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub receipt_handle: String,
    pub event: PublishQueuedEvent,
}

#[async_trait]
pub trait CompileQueue: Send + Sync {
    /// Long-poll for up to one message. `None` means idle (no work).
    async fn receive(&self) -> Result<Option<QueuedMessage>, AppError>;

    /// Delete after successful compile + catalog upsert.
    async fn delete(&self, receipt_handle: &str) -> Result<(), AppError>;

    /// Keep the in-flight message hidden while AOT compile runs.
    async fn extend_visibility(&self, receipt_handle: &str, seconds: i32) -> Result<(), AppError>;
}
