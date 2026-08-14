use std::path::PathBuf;
use std::time::Duration;

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
    /// Compile-on-deploy: upload a .wasm; the host compiles and stores the module.
    Publish {
        /// Path to a compiled .wasm module
        wasm: PathBuf,
        /// Function name as used in /invoke/{name}
        #[arg(long)]
        name: String,
        #[arg(long, env = "NITRUM_FN_URL", default_value = "http://127.0.0.1:8080")]
        url: String,
    },
}

#[derive(Debug, Deserialize)]
struct PublishBody {
    name: String,
    version: String,
    hash: String,
    wasm_bytes: usize,
    compiled_bytes: usize,
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
        Commands::Publish { wasm, name, url } => publish(wasm, name, url).await,
    }
}

async fn publish(wasm_path: PathBuf, name: String, url: String) -> Result<()> {
    let wasm = tokio::fs::read(&wasm_path)
        .await
        .with_context(|| format!("read {}", wasm_path.display()))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .context("http client")?;

    let endpoint = format!("{}/functions/{}", url.trim_end_matches('/'), name);

    let response = client
        .put(&endpoint)
        .header("content-type", "application/wasm")
        .body(wasm)
        .send()
        .await
        .with_context(|| format!("PUT {endpoint}"))?;

    let status = response.status();
    let bytes = response.bytes().await.context("read response")?;

    if !status.is_success() {
        let msg = serde_json::from_slice::<ErrorBody>(&bytes)
            .map(|b| b.error)
            .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
        anyhow::bail!("publish failed ({status}): {msg}");
    }

    let body: PublishBody = serde_json::from_slice(&bytes).context("decode publish response")?;
    println!(
        "published {}@{} hash={} wasm_bytes={} compiled_bytes={}",
        body.name, body.version, body.hash, body.wasm_bytes, body.compiled_bytes
    );
    Ok(())
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
            Commands::Publish { wasm, name, url } => {
                assert_eq!(wasm, PathBuf::from("./echo.wasm"));
                assert_eq!(name, "echo");
                assert_eq!(url, "http://127.0.0.1:8080");
            }
        }
    }
}
