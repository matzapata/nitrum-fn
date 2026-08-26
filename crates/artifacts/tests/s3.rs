//! Floci S3 artifact adapter. Requires `NITRUM_FN_ARTIFACTS__ENDPOINT`.

mod common;

use application::error::AppError;
use application::ports::ArtifactStore;
use artifacts::S3ArtifactStore;
use aws_sdk_s3::Client;
use domain::ContentHash;

/// Ensure the bucket exists.
async fn ensure_bucket(client: &Client, bucket: &str) {
    if client.head_bucket().bucket(bucket).send().await.is_ok() {
        return;
    }
    client
        .create_bucket()
        .bucket(bucket)
        .send()
        .await
        .expect("create bucket");
}

#[tokio::test]
async fn put_get_wasm_and_cwasm() {
    let client = common::s3_client().await;
    let bucket = common::unique("nitrum-fn-artifacts");
    ensure_bucket(&client, &bucket).await;

    let store = S3ArtifactStore::new(client, bucket, "artifacts");
    let wasm = b"\0asm\x01\x00\x00\x00fake";
    let hash = store.put(wasm).await.expect("put wasm");
    assert_eq!(store.get(&hash).await.expect("get wasm"), wasm);

    let compiled = b"fake-cwasm";
    store
        .put_compiled(&hash, compiled)
        .await
        .expect("put cwasm");
    assert_eq!(
        store.get_compiled(&hash).await.expect("get cwasm"),
        compiled
    );
}

#[tokio::test]
async fn missing_artifact_is_artifact_missing() {
    let client = common::s3_client().await;
    let bucket = common::unique("nitrum-fn-artifacts-miss");
    ensure_bucket(&client, &bucket).await;

    let store = S3ArtifactStore::new(client, bucket, "artifacts");
    let hash = ContentHash::from_bytes(b"missing");
    let err = store.get(&hash).await.expect_err("missing");
    assert!(matches!(err, AppError::ArtifactMissing(_)), "{err}");
}
