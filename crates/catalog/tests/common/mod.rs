use aws_sdk_dynamodb::config::Builder as DdbConfigBuilder;
use aws_sdk_dynamodb::Client;

pub async fn load_aws_config() -> aws_config::SdkConfig {
    let http_client = aws_smithy_http_client::Builder::new()
        .tls_provider(aws_smithy_http_client::tls::Provider::Rustls(
            aws_smithy_http_client::tls::rustls_provider::CryptoMode::Ring,
        ))
        .build_https();
    aws_config::defaults(aws_config::BehaviorVersion::latest())
        .http_client(http_client)
        .load()
        .await
}

pub async fn ddb_client() -> Client {
    let endpoint = std::env::var("NITRUM_FN_CATALOG__ENDPOINT")
        .expect("NITRUM_FN_CATALOG__ENDPOINT must be set (Floci, e.g. http://127.0.0.1:4566)");
    let sdk = load_aws_config().await;
    Client::from_conf(DdbConfigBuilder::from(&sdk).endpoint_url(endpoint).build())
}

pub fn unique(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}-{}-{n}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}
