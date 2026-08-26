//! Artifact store adapters. S3 for local (Floci) and cloud.

mod s3;

pub use s3::S3ArtifactStore;
