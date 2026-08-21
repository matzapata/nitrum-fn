use application::error::AppError;
use application::ports::{CompileQueue, QueuedMessage};
use async_trait::async_trait;
use aws_sdk_sqs::Client;
use tracing::warn;

use crate::event::parse_queued_event;

/// Long-poll SQS consumer for the compile worker.
pub struct SqsCompileConsumer {
    client: Client,
    queue_url: String,
    wait_seconds: i32,
}

impl SqsCompileConsumer {
    pub fn new(client: Client, queue_url: impl Into<String>) -> Self {
        Self {
            client,
            queue_url: queue_url.into(),
            wait_seconds: 20,
        }
    }

    pub fn with_wait_seconds(mut self, wait_seconds: i32) -> Self {
        self.wait_seconds = wait_seconds;
        self
    }
}

#[async_trait]
impl CompileQueue for SqsCompileConsumer {
    async fn receive(&self) -> Result<Option<QueuedMessage>, AppError> {
        let out = self
            .client
            .receive_message()
            .queue_url(&self.queue_url)
            .max_number_of_messages(1)
            .wait_time_seconds(self.wait_seconds)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;

        let Some(msg) = out.messages.and_then(|mut m| m.pop()) else {
            return Ok(None);
        };

        let receipt = msg
            .receipt_handle
            .ok_or_else(|| AppError::Storage("SQS message missing receipt handle".into()))?;
        let body = msg
            .body
            .ok_or_else(|| AppError::Storage("SQS message missing body".into()))?;

        match parse_queued_event(&body) {
            Ok(event) => Ok(Some(QueuedMessage {
                receipt_handle: receipt,
                event,
            })),
            Err(err) => {
                // Malformed payloads never become valid; delete so they do not
                // loop. They will not appear on the compile DLQ.
                warn!(error = %err, "dropping malformed compile queue message");
                self.delete(&receipt).await?;
                Ok(None)
            }
        }
    }

    async fn delete(&self, receipt_handle: &str) -> Result<(), AppError> {
        self.client
            .delete_message()
            .queue_url(&self.queue_url)
            .receipt_handle(receipt_handle)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn extend_visibility(&self, receipt_handle: &str, seconds: i32) -> Result<(), AppError> {
        self.client
            .change_message_visibility()
            .queue_url(&self.queue_url)
            .receipt_handle(receipt_handle)
            .visibility_timeout(seconds)
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(())
    }
}
