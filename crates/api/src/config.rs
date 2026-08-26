use anyhow::{bail, Context, Result};

/// Configuration struct for the API, populated from environment variables.
/// Bucket, tables, and queues must already exist (Terraform in cloud; AWS CLI against emulators locally).
pub struct ApiConfig {
    /// Port on which the API will listen.
    pub port: u16,
    /// Name of the S3 bucket to use for storage.
    pub s3_bucket: String,
    /// Optional endpoint URL for S3 (e.g., for localstack or custom endpoint).
    pub s3_endpoint: Option<String>,
    /// Name of the DynamoDB table for primary storage.
    pub ddb_table: String,
    /// Optional endpoint URL for DynamoDB (e.g., for local development).
    pub ddb_endpoint: Option<String>,
    /// Name of the DynamoDB idempotency table, derived as `{catalog}-idempotency` unless overridden by `NITRUM_FN_DDB_IDEMPOTENCY_TABLE`.
    pub ddb_idempotency_table: String,
    /// ARN of the SNS topic for publish fan-out (used in cloud deployments).
    pub sns_topic_arn: Option<String>,
    /// URL of the SQS queue for local use (Floci) or when SNS is not set; used for direct enqueues.
    pub sqs_queue_url: Option<String>,
    /// Optional endpoint URL for SQS (e.g., for localstack or custom endpoint).
    pub sqs_endpoint: Option<String>,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self> {
        reject_legacy_store()?;
        let port = parse_port(env_opt("NITRUM_FN_PORT")?.as_deref())?;
        let ddb_table = require_env("NITRUM_FN_DDB_TABLE")?;
        let ddb_idempotency_table = env_opt("NITRUM_FN_DDB_IDEMPOTENCY_TABLE")?
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{ddb_table}-idempotency"));
        Ok(Self {
            port,
            s3_bucket: require_env("NITRUM_FN_S3_BUCKET")?,
            s3_endpoint: env_opt("NITRUM_FN_S3_ENDPOINT")?.filter(|s| !s.is_empty()),
            ddb_table,
            ddb_endpoint: env_opt("NITRUM_FN_DDB_ENDPOINT")?.filter(|s| !s.is_empty()),
            ddb_idempotency_table,
            sns_topic_arn: env_opt("NITRUM_FN_SNS_TOPIC_ARN")?.filter(|s| !s.is_empty()),
            sqs_queue_url: env_opt("NITRUM_FN_SQS_QUEUE_URL")?.filter(|s| !s.is_empty()),
            sqs_endpoint: env_opt("NITRUM_FN_SQS_ENDPOINT")?.filter(|s| !s.is_empty()),
        })
    }
}

fn env_opt(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(s) => Ok(Some(s)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{name} is not valid UTF-8"),
    }
}

fn require_env(name: &str) -> Result<String> {
    env_opt(name)?
        .filter(|s| !s.is_empty())
        .with_context(|| format!("{name} is required"))
}

fn reject_legacy_store() -> Result<()> {
    match env_opt("NITRUM_FN_STORE")? {
        None => Ok(()),
        Some(s) if s.is_empty() || s.eq_ignore_ascii_case("aws") => Ok(()),
        Some(s) if s.eq_ignore_ascii_case("fs") || s.eq_ignore_ascii_case("filesystem") => {
            bail!("NITRUM_FN_STORE=fs was removed; use Floci (S3/SQS) and DynamoDB Local")
        }
        Some(s) => bail!("invalid NITRUM_FN_STORE={s}; store is always AWS"),
    }
}

fn parse_port(raw: Option<&str>) -> Result<u16> {
    match raw {
        None => Ok(8080),
        Some(s) => s
            .parse()
            .with_context(|| format!("invalid NITRUM_FN_PORT={s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_port() {
        assert!(parse_port(Some("not-a-port")).is_err());
        assert_eq!(parse_port(None).unwrap(), 8080);
    }

    #[test]
    fn rejects_legacy_filesystem_store() {
        assert!(reject_legacy_store_value(Some("fs")).is_err());
        assert!(reject_legacy_store_value(Some("aws")).is_ok());
        assert!(reject_legacy_store_value(None).is_ok());
    }

    fn reject_legacy_store_value(raw: Option<&str>) -> Result<()> {
        match raw {
            None => Ok(()),
            Some(s) if s.is_empty() || s.eq_ignore_ascii_case("aws") => Ok(()),
            Some(s) if s.eq_ignore_ascii_case("fs") || s.eq_ignore_ascii_case("filesystem") => {
                bail!("NITRUM_FN_STORE=fs was removed; use Floci (S3/SQS) and DynamoDB Local")
            }
            Some(s) => bail!("invalid NITRUM_FN_STORE={s}; store is always AWS"),
        }
    }
}
