use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::Client;

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

pub async fn s3_client() -> Client {
    let endpoint = std::env::var("NITRUM_FN_ARTIFACTS__ENDPOINT")
        .expect("NITRUM_FN_ARTIFACTS__ENDPOINT must be set (Floci, e.g. http://127.0.0.1:4566)");
    let sdk = load_aws_config().await;
    Client::from_conf(
        S3ConfigBuilder::from(&sdk)
            .endpoint_url(endpoint)
            .force_path_style(true)
            .build(),
    )
}

pub fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}
