//! Function catalog adapters. Filesystem for the local host; in-memory for tests.

mod filesystem;
mod memory;

pub use filesystem::FilesystemCatalog;
pub use memory::InMemoryCatalog;
