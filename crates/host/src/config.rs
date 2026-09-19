use ::config::{Config, Environment, File};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// `config/shared/base.yaml` → `config/shared/{NITRUM_FN_ENV}.yaml` →
/// `config/host/base.yaml` → `config/host/{NITRUM_FN_ENV}.yaml` → `NITRUM_FN_*` env.
#[derive(Debug, Clone, Deserialize)]
pub struct HostConfig {
    pub server: ServerConfig,
    pub artifacts: ArtifactsConfig,
    pub catalog: CatalogConfig,

    /// Overlay name that was loaded (`NITRUM_FN_ENV`, else `local`); not itself a
    /// config source, stamped on after deserializing so callers don't re-derive it.
    #[serde(default, skip_deserializing)]
    pub run_env: String,
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
        // Overlay name: `NITRUM_FN_ENV`, else `local`. The enclave gets the var from
        // SSM after the data-plane clears process env.
        let run_env = std::env::var("NITRUM_FN_ENV")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "local".to_string());

        // Config lives next to the binary (`config/` beside `/app/nitrum-fn-host` in
        // the EIF). The data-plane doesn't guarantee cwd == the Docker WORKDIR, so
        // resolve relative to the exe rather than `.`; fall back to `.` (e.g. `cargo
        // run`, where `current_exe` is under `target/debug`).
        let root = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from))
            .filter(|dir| dir.join("config").is_dir())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut config: HostConfig = Config::builder()
            .add_source(File::from(root.join("config/shared/base")).required(false))
            .add_source(File::from(root.join(format!("config/shared/{run_env}"))).required(false))
            .add_source(File::from(root.join("config/host/base")).required(false))
            .add_source(File::from(root.join(format!("config/host/{run_env}"))).required(false))
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
            .context("parse config")?;

        config.run_env = run_env;
        Ok(config)
    }
}
