use ::config::{Config, Environment, File};
use anyhow::{Context, Result};
use serde::Deserialize;

/// `config/api/base.yaml` → `config/api/{NITRUM_FN_ENV}.yaml` → `NITRUM_FN_*` env.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    pub server: ServerConfig,
    pub artifacts: ArtifactsConfig,
    pub catalog: CatalogConfig,
    pub publish: PublishConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactsConfig {
    pub bucket: String,
    pub prefix: String,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogConfig {
    pub table: String,
    pub idempotency_table: String,
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishConfig {
    pub topic_arn: String,
    pub endpoint: Option<String>,
}

impl ApiConfig {
    pub fn load() -> Result<Self> {
        let run_env = std::env::var("NITRUM_FN_ENV")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "local".to_string());

        Config::builder()
            .add_source(File::with_name("config/api/base").required(false))
            .add_source(File::with_name(&format!("config/api/{run_env}")).required(false))
            .add_source(
                Environment::with_prefix("NITRUM_FN")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true)
                    .ignore_empty(true),
            )
            .build()
            .context("load config")?
            .try_deserialize()
            .context("parse config")
    }
}
