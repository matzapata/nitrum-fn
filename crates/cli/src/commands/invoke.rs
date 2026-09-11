use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;

#[derive(Debug, Args)]
pub struct InvokeArgs {
    /// Function name as used in /invoke/{name}
    pub name: String,
    #[arg(long, env = "NITRUM_FN_INVOKE_URL", default_value = "http://127.0.0.1:8081")]
    pub url: String,
    /// JSON request body
    #[arg(short = 'd', long, default_value = "{}")]
    pub data: String,
    /// Version label (default: latest)
    #[arg(long)]
    pub version: Option<String>,
    /// Accept self-signed TLS (cloud NLB invoke)
    #[arg(long)]
    pub insecure: bool,
}

#[derive(Debug, serde::Deserialize)]
struct ErrorBody {
    error: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(name = %args.name), err)]
pub async fn run(args: InvokeArgs) -> Result<()> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(3));
    if args.insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }
    let client = builder.build().context("http client")?;

    let base = args.url.trim_end_matches('/');
    let endpoint = format!("{base}/invoke/{}", args.name);

    let mut request = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .body(args.data);
    if let Some(version) = &args.version {
        request = request.header("x-nitrum-fn-version", version.as_str());
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("POST {endpoint}"))?;

    let status = response.status();
    let body = response.text().await.context("read response")?;

    if !status.is_success() {
        let msg = serde_json::from_str::<ErrorBody>(&body)
            .map(|b| b.error)
            .unwrap_or(body);
        anyhow::bail!("invoke failed ({status}): {msg}");
    }

    println!("{body}");
    Ok(())
}
