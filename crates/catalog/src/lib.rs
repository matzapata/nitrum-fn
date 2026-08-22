//! Function catalog adapters. Filesystem / in-memory for local; DynamoDB for shared store.

mod dynamodb;
mod filesystem;
mod idempotency;
mod idempotency_dynamodb;
mod idempotency_filesystem;
mod idempotency_memory;
mod memory;

pub use dynamodb::DynamoDbCatalog;
pub use filesystem::FilesystemCatalog;
pub use idempotency_dynamodb::DynamoDbPublishIdempotency;
pub use idempotency_filesystem::FilesystemPublishIdempotency;
pub use idempotency_memory::InMemoryPublishIdempotency;
pub use memory::InMemoryCatalog;
