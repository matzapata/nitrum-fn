use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;

#[derive(Debug, Parser)]
#[command(
    name = "nitrum-fn",
    about = "Publish WASM functions to a nitrum-fn host"
)]
struct Cli {
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Upload a .wasm; the API queues AOT compile. Polls until the function is ready.
    Publish {
        /// Path to a compiled .wasm module
        wasm: PathBuf,
        /// Function name as used in /invoke/{name}
        #[arg(long)]
        name: String,
        #[arg(long, env = "NITRUM_FN_URL", default_value = "http://127.0.0.1:8080")]
        url: String,
        /// How long to wait for the compile worker to upsert the catalog
        #[arg(long, env = "NITRUM_FN_PUBLISH_TIMEOUT_SECS", default_value_t = 180)]
        timeout_secs: u64,
        /// Retry the same deploy without a second compile enqueue. Generated if omitted;
        /// printed immediately so a failed or timed-out publish can be retried.
        #[arg(long, env = "NITRUM_FN_IDEMPOTENCY_KEY")]
        idempotency_key: Option<String>,
    },
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .init();

    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Publish {
            wasm,
            name,
            url,
            timeout_secs,
            idempotency_key,
        } => publish(wasm, name, url, timeout_secs, idempotency_key).await,
    }
}

async fn publish(
    wasm_path: PathBuf,
    name: String,
    url: String,
    timeout_secs: u64,
    idempotency_key: Option<String>,
) -> Result<()> {
    let wasm = tokio::fs::read(&wasm_path)
        .await
        .with_context(|| format!("read {}", wasm_path.display()))?;

    let key = match idempotency_key {
        Some(k) => k,
        None => uuid::Uuid::new_v4().to_string(),
    };
    eprintln!("idempotency_key={key}");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .context("http client")?;

    let base = url.trim_end_matches('/');
    let endpoint = format!("{base}/functions/{name}");

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
        anyhow::bail!("publish failed ({status}): {msg} (idempotency_key={key})");
    }

    let body: PublishBody = serde_json::from_slice(&bytes).context("decode publish response")?;
    wait_until_ready(&client, &endpoint, &body.hash, timeout_secs, &key).await?;

    println!(
        "published {}@{} hash={} wasm_bytes={} status=ready idempotency_key={}",
        body.name, body.version, body.hash, body.wasm_bytes, key
    );
    Ok(())
}

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
                "{}",
                timed_out_msg(timeout_secs, expected_hash, idempotency_key)
            );
        }

        let response = client
            .get(endpoint)
            .send()
            .await
            .with_context(|| format!("GET {endpoint} (idempotency_key={idempotency_key})"))?;

        if response.status().as_u16() == 404 {
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

        // Previous version still published; wait for upsert of the new hash.
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
}

fn timed_out_msg(timeout_secs: u64, expected_hash: &str, idempotency_key: &str) -> String {
    format!(
        "timed out after {timeout_secs}s waiting for function ready (hash={expected_hash} idempotency_key={idempotency_key})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_publish() {
        let cli = Cli::try_parse_from([
            "nitrum-fn",
            "publish",
            "./echo.wasm",
            "--name",
            "echo",
            "--url",
            "http://127.0.0.1:8080",
        ])
        .expect("parse");
        match cli.command {
            Commands::Publish {
                wasm,
                name,
                url,
                timeout_secs,
                idempotency_key,
            } => {
                assert_eq!(wasm, PathBuf::from("./echo.wasm"));
                assert_eq!(name, "echo");
                assert_eq!(url, "http://127.0.0.1:8080");
                assert_eq!(timeout_secs, 180);
                assert!(idempotency_key.is_none());
            }
        }
    }

    #[test]
    fn parses_idempotency_key() {
        let cli = Cli::try_parse_from([
            "nitrum-fn",
            "publish",
            "./echo.wasm",
            "--name",
            "echo",
            "--idempotency-key",
            "retry-1",
        ])
        .expect("parse");
        match cli.command {
            Commands::Publish {
                idempotency_key, ..
            } => {
                assert_eq!(idempotency_key.as_deref(), Some("retry-1"));
            }
        }
    }

    #[test]
    fn timeout_message_includes_idempotency_key() {
        let msg = timed_out_msg(180, "abcd", "retry-1");
        assert!(msg.contains("idempotency_key=retry-1"), "{msg}");
        assert!(msg.contains("hash=abcd"), "{msg}");
    }
}
