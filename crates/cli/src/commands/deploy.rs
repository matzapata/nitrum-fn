use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
use domain::{normalize_egress_allow, EgressOrigin};
use serde::Deserialize;

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Path to a compiled .wasm module
    pub wasm: PathBuf,
    /// Function name as used in /invoke/{name}
    #[arg(long)]
    pub name: String,
    #[arg(long, env = "NITRUM_FN_URL", default_value = "http://127.0.0.1:8080")]
    pub url: String,
    /// HTTPS origin the function may fetch (repeatable)
    #[arg(long = "allow-url", value_name = "URL")]
    pub allow_urls: Vec<String>,
    /// Optional TOML config (`allow_urls = [...]`)
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct FunctionConfig {
    #[serde(default)]
    allow_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PublishBody {
    name: String,
    version: String,
    hash: String,
    wasm_bytes: usize,
    status: String,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(name = %args.name, wasm = %args.wasm.display()), err)]
pub async fn run(args: DeployArgs) -> Result<()> {
    let allow_urls = collect_allow_urls(&args)?;
    let wasm = tokio::fs::read(&args.wasm)
        .await
        .with_context(|| format!("read {}", args.wasm.display()))?;
    tracing::debug!(bytes = wasm.len(), "read wasm");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .context("http client")?;

    let base = args.url.trim_end_matches('/');
    let endpoint = format!("{base}/functions/{}", args.name);

    let mut request = client
        .put(&endpoint)
        .header("content-type", "application/wasm")
        .body(wasm);
    for origin in &allow_urls {
        request = request.header("x-nitrum-fn-allow-url", origin.as_str());
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("PUT {endpoint}"))?;

    let status = response.status();
    let bytes = response.bytes().await.context("read response")?;

    if !status.is_success() {
        let msg = serde_json::from_slice::<ErrorBody>(&bytes)
            .map(|b| b.error)
            .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
        anyhow::bail!("deploy failed ({status}): {msg}");
    }

    let body: PublishBody = serde_json::from_slice(&bytes).context("decode response")?;
    tracing::debug!(
        status = %body.status,
        hash = %body.hash,
        version = %body.version,
        "deployed"
    );

    println!(
        "deployed {}@{} hash={} wasm_bytes={} status={}",
        body.name, body.version, body.hash, body.wasm_bytes, body.status
    );
    Ok(())
}

fn collect_allow_urls(args: &DeployArgs) -> Result<Vec<EgressOrigin>> {
    let mut raw = args.allow_urls.clone();
    if let Some(path) = &args.config {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: FunctionConfig =
            toml::from_str(&text).with_context(|| format!("parse config {}", path.display()))?;
        raw.extend(cfg.allow_urls);
    }
    let origins = raw
        .iter()
        .map(|u| EgressOrigin::parse(u))
        .collect::<Result<Vec<_>, _>>()
        .context("invalid allow-url")?;
    normalize_egress_allow(origins).context("egress allowlist")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_config_and_flags() {
        let dir = std::env::temp_dir().join(format!("nitrum-fn-deploy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("function.toml");
        std::fs::write(&path, r#"allow_urls = ["https://api.coingecko.com"]"#).unwrap();

        let args = DeployArgs {
            wasm: PathBuf::from("./f.wasm"),
            name: "oracle".into(),
            url: "http://127.0.0.1:8080".into(),
            allow_urls: vec!["https://example.com".into()],
            config: Some(path),
        };
        let origins = collect_allow_urls(&args).expect("origins");
        assert_eq!(origins.len(), 2);
    }
}
