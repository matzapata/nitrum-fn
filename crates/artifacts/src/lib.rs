//! Artifact store adapters. Filesystem for local host testing; S3 for shared store.

mod filesystem;
mod s3;

pub use filesystem::FilesystemArtifactStore;
pub use s3::S3ArtifactStore;
