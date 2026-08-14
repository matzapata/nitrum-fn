//! Function catalog adapters. Filesystem / in-memory for local; DynamoDB for shared store.

mod dynamodb;
mod filesystem;
mod memory;

pub use dynamodb::DynamoDbCatalog;
pub use filesystem::FilesystemCatalog;
pub use memory::InMemoryCatalog;
