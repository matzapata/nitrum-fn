use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Args;
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
    /// How long to wait for the compile worker to upsert the catalog
    #[arg(long, env = "NITRUM_FN_DEPLOY_TIMEOUT_SECS", default_value_t = 180)]
    pub timeout_secs: u64,
    /// Retry the same deploy without a second compile enqueue. Generated if omitted;
    /// printed immediately so a failed or timed-out deploy can be retried.
    #[arg(long, env = "NITRUM_FN_IDEMPOTENCY_KEY")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublishBody {
    name: String,
    version: String,
    hash: String,
    wasm_bytes: usize,
    #[allow(dead_code)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct FunctionBody {
    hash: String,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(name = %args.name, wasm = %args.wasm.display()), err)]
pub async fn run(args: DeployArgs) -> Result<()> {
    let wasm = tokio::fs::read(&args.wasm)
        .await
        .with_context(|| format!("read {}", args.wasm.display()))?;
    tracing::debug!(bytes = wasm.len(), "read wasm");

    let key = match args.idempotency_key {
        Some(k) => k,
        None => uuid::Uuid::new_v4().to_string(),
    };
    eprintln!("idempotency_key={key}");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .context("http client")?;

    let base = args.url.trim_end_matches('/');
    let endpoint = format!("{base}/functions/{}", args.name);

    let response = client
        .put(&endpoint)
        .header("content-type", "application/wasm")
        .header("idempotency-key", &key)
        .body(wasm)
        .send()
        .await
        .with_context(|| format!("PUT {endpoint} (idempotency_key={key})"))?;

    let status = response.status();
    let bytes = response.bytes().await.context("read response")?;

    if !status.is_success() {
        let msg = serde_json::from_slice::<ErrorBody>(&bytes)
            .map(|b| b.error)
            .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
        anyhow::bail!("deploy failed ({status}): {msg} (idempotency_key={key})");
    }

    let body: PublishBody = serde_json::from_slice(&bytes).context("decode response")?;
    tracing::debug!(
        status = %status,
        hash = %body.hash,
        version = %body.version,
        "deploy accepted"
    );
    wait_until_ready(&client, &endpoint, &body.hash, args.timeout_secs, &key).await?;

    println!(
        "deployed {}@{} hash={} wasm_bytes={} status=ready idempotency_key={}",
        body.name, body.version, body.hash, body.wasm_bytes, key
    );
    Ok(())
}

#[tracing::instrument(level = "debug", skip(client), err)]
async fn wait_until_ready(
    client: &reqwest::Client,
    endpoint: &str,
    expected_hash: &str,
    timeout_secs: u64,
    idempotency_key: &str,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut delay = Duration::from_millis(200);

    loop {
        if Instant::now() >= deadline {
            anyhow::bail!(
                "timed out after {timeout_secs}s waiting for function ready (hash={expected_hash} idempotency_key={idempotency_key})"
            );
        }

        let response = client
            .get(endpoint)
            .send()
            .await
            .with_context(|| format!("GET {endpoint} (idempotency_key={idempotency_key})"))?;

        if response.status().as_u16() == 404 {
            tracing::debug!(retry_in = ?delay, "function not in catalog yet");
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(2));
            continue;
        }

        if !response.status().is_success() {
            let status = response.status();
            let bytes = response.bytes().await.unwrap_or_default();
            let msg = serde_json::from_slice::<ErrorBody>(&bytes)
                .map(|b| b.error)
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
            anyhow::bail!("poll failed ({status}): {msg} (idempotency_key={idempotency_key})");
        }

        let meta: FunctionBody = response.json().await.context("decode function metadata")?;
        if meta.hash == expected_hash {
            return Ok(());
        }

        tracing::debug!(
            current = %meta.hash,
            expected = %expected_hash,
            retry_in = ?delay,
            "waiting for new hash"
        );
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
}
