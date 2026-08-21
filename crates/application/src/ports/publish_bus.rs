use async_trait::async_trait;
use domain::PublishQueuedEvent;

use crate::AppError;

#[async_trait]
pub trait PublishBus: Send + Sync {
    /// Notify subscribers that a `.wasm` is stored and ready for AOT compile.
    async fn publish_queued(&self, event: &PublishQueuedEvent) -> Result<(), AppError>;
}
