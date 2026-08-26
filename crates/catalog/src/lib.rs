//! Function catalog adapters. DynamoDB for local (DynamoDB Local) and cloud.

mod dynamodb;
mod idempotency;
mod idempotency_dynamodb;

pub use dynamodb::DynamoDbCatalog;
pub use idempotency_dynamodb::DynamoDbPublishIdempotency;

#[cfg(test)]
pub(crate) async fn load_test_aws_config() -> aws_config::SdkConfig {
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
