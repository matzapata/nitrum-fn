use application::error::AppError;
use application::ports::ArtifactStore;
use async_trait::async_trait;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use domain::{ContentHash, MAX_COMPILED_BYTES, MAX_WASM_BYTES};

fn is_missing_object(err: &SdkError<GetObjectError>) -> bool {
    match err {
        SdkError::ServiceError(e) => matches!(e.err(), GetObjectError::NoSuchKey(_)),
        _ => err
            .raw_response()
            .map(|r| r.status().as_u16() == 404)
            .unwrap_or(false),
    }
}

/// Stores `artifacts/{sha256}.wasm` and `artifacts/{sha256}.cwasm` in one bucket.
pub struct S3ArtifactStore {
    client: Client,
    bucket: String,
    /// Key prefix without trailing slash (e.g. `artifacts`).
    prefix: String,
}

impl S3ArtifactStore {
    pub fn new(client: Client, bucket: impl Into<String>, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into().trim_matches('/').to_string();
        Self {
            client,
            bucket: bucket.into(),
            prefix,
        }
    }

    fn wasm_key(&self, hash: &ContentHash) -> String {
        format!("{}/{}.wasm", self.prefix, hash.to_hex())
    }

    fn cwasm_key(&self, hash: &ContentHash) -> String {
        format!("{}/{}.cwasm", self.prefix, hash.to_hex())
    }

    async fn get_object(
        &self,
        key: &str,
        hash: &ContentHash,
        max_bytes: usize,
    ) -> Result<Vec<u8>, AppError> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => {
                if let Some(len) = out.content_length() {
                    if len < 0 {
                        return Err(AppError::Storage(format!(
                            "negative content-length for {key}"
                        )));
                    }
                    if len as u64 > max_bytes as u64 {
                        return Err(AppError::PayloadTooLarge(format!(
                            "object {key} is {len} bytes (max {max_bytes})"
                        )));
                    }
                }
                out.body
                    .collect()
                    .await
                    .map(|b| b.to_vec())
                    .map_err(|e| AppError::Storage(e.to_string()))
                    .and_then(|bytes| {
                        if bytes.len() > max_bytes {
                            Err(AppError::PayloadTooLarge(format!(
                                "object {key} is {} bytes (max {max_bytes})",
                                bytes.len()
                            )))
                        } else {
                            Ok(bytes)
                        }
                    })
            }
            Err(err) if is_missing_object(&err) => Err(AppError::ArtifactMissing(hash.to_hex())),
            Err(err) => Err(AppError::Storage(err.to_string())),
        }
    }

    async fn put_object(&self, key: &str, bytes: &[u8]) -> Result<(), AppError> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(bytes.to_vec()))
            .send()
            .await
            .map_err(|e| AppError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl ArtifactStore for S3ArtifactStore {
    async fn put(&self, wasm: &[u8]) -> Result<ContentHash, AppError> {
        let hash = ContentHash::from_bytes(wasm);
        self.put_object(&self.wasm_key(&hash), wasm).await?;
        Ok(hash)
    }

    async fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
        self.get_object(&self.wasm_key(hash), hash, MAX_WASM_BYTES)
            .await
    }

    async fn put_compiled(&self, hash: &ContentHash, compiled: &[u8]) -> Result<(), AppError> {
        if compiled.len() > MAX_COMPILED_BYTES {
            return Err(AppError::PayloadTooLarge(format!(
                "compiled {} bytes exceeds max {MAX_COMPILED_BYTES}",
                compiled.len()
            )));
        }
        self.put_object(&self.cwasm_key(hash), compiled).await
    }

    async fn get_compiled(&self, hash: &ContentHash) -> Result<Vec<u8>, AppError> {
        self.get_object(&self.cwasm_key(hash), hash, MAX_COMPILED_BYTES)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::ports::ArtifactStore;
    use aws_sdk_s3::config::Builder as S3ConfigBuilder;

    async fn s3_client() -> Option<Client> {
        let endpoint = std::env::var("NITRUM_FN_S3_ENDPOINT").ok()?;
        let sdk = crate::load_test_aws_config().await;
        let conf = S3ConfigBuilder::from(&sdk)
            .endpoint_url(endpoint)
            .force_path_style(true)
            .build();
        Some(Client::from_conf(conf))
    }

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
        let Some(client) = s3_client().await else {
            eprintln!("skip: NITRUM_FN_S3_ENDPOINT not set");
            return;
        };
        let bucket = format!(
            "nitrum-fn-artifacts-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
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
        let Some(client) = s3_client().await else {
            eprintln!("skip: NITRUM_FN_S3_ENDPOINT not set");
            return;
        };
        let bucket = format!(
            "nitrum-fn-artifacts-miss-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        ensure_bucket(&client, &bucket).await;

        let store = S3ArtifactStore::new(client, bucket, "artifacts");
        let hash = ContentHash::from_bytes(b"missing");
        let err = store.get(&hash).await.expect_err("missing");
        assert!(matches!(err, AppError::ArtifactMissing(_)), "{err}");
    }
}
