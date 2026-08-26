//! SNS / SQS adapters for async publish.

mod event;
mod sns_bus;
mod sqs_consumer;

/// Matches staging Terraform (`visibility_timeout_seconds = 300`).
pub const COMPILE_VISIBILITY_TIMEOUT_SECS: i32 = 300;

pub use event::parse_queued_event;
pub use sns_bus::SnsPublishBus;
pub use sqs_consumer::SqsCompileConsumer;
