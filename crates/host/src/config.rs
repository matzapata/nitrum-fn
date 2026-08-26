use anyhow::{bail, Context, Result};

pub struct HostConfig {
    pub port: u16,
    pub s3_bucket: String,
    pub s3_endpoint: Option<String>,
    pub s3_create_bucket: bool,
    pub ddb_table: String,
    pub ddb_endpoint: Option<String>,
    pub ddb_create_table: bool,
    pub ddb_idempotency_table: String,
    pub sns_topic_arn: Option<String>,
    pub sqs_queue_url: Option<String>,
    pub sqs_endpoint: Option<String>,
    pub sqs_create_queue: bool,
}

impl HostConfig {
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
            s3_create_bucket: parse_flag("NITRUM_FN_S3_CREATE_BUCKET")?,
            ddb_table,
            ddb_endpoint: env_opt("NITRUM_FN_DDB_ENDPOINT")?.filter(|s| !s.is_empty()),
            ddb_create_table: parse_flag("NITRUM_FN_DDB_CREATE_TABLE")?,
            ddb_idempotency_table,
            sns_topic_arn: env_opt("NITRUM_FN_SNS_TOPIC_ARN")?.filter(|s| !s.is_empty()),
            sqs_queue_url: env_opt("NITRUM_FN_SQS_QUEUE_URL")?.filter(|s| !s.is_empty()),
            sqs_endpoint: env_opt("NITRUM_FN_SQS_ENDPOINT")?.filter(|s| !s.is_empty()),
            sqs_create_queue: parse_flag("NITRUM_FN_SQS_CREATE_QUEUE")?,
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

fn parse_flag(name: &str) -> Result<bool> {
    parse_flag_value(name, env_opt(name)?.as_deref())
}

fn parse_flag_value(name: &str, raw: Option<&str>) -> Result<bool> {
    match raw {
        None | Some("") => Ok(false),
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Ok(true),
            "0" | "false" | "no" => Ok(false),
            other => bail!("invalid {name}={other}; expected true or false"),
        },
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

    #[test]
    fn rejects_invalid_flag() {
        assert!(parse_flag_value("NITRUM_FN_S3_CREATE_BUCKET", Some("maybe")).is_err());
        assert!(!parse_flag_value("NITRUM_FN_S3_CREATE_BUCKET", Some("false")).unwrap());
        assert!(parse_flag_value("NITRUM_FN_S3_CREATE_BUCKET", Some("1")).unwrap());
    }
}
