//! Function catalog adapters. DynamoDB for local (Floci) and cloud.

mod function_catalog;
mod publish_lock;

pub use function_catalog::DynamoDbFunctionCatalog;
pub use publish_lock::DynamoDbPublishLock;
