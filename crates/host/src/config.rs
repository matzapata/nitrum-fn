use ::config::{Config, Environment, File};
use anyhow::{Context, Result};
use serde::Deserialize;

/// `config/shared/base.yaml` → `config/shared/{NITRUM_FN_ENV}.yaml` →
/// `config/host/base.yaml` → `config/host/{NITRUM_FN_ENV}.yaml` → `NITRUM_FN_*` env.
#[derive(Debug, Clone, Deserialize)]
pub struct HostConfig {
    pub server: ServerConfig,
    pub artifacts: ArtifactsConfig,
    pub catalog: CatalogConfig,
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
    pub endpoint: Option<String>,
}

impl HostConfig {
    pub fn load() -> Result<Self> {
        let run_env = std::env::var("NITRUM_FN_ENV")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "local".to_string());
        Config::builder()
            .add_source(File::with_name("config/shared/base").required(false))
            .add_source(File::with_name(&format!("config/shared/{run_env}")).required(false))
            .add_source(File::with_name("config/host/base").required(false))
            .add_source(File::with_name(&format!("config/host/{run_env}")).required(false))
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
