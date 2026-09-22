use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};

use super::x402::{self, credit_body};

#[derive(Debug, Args)]
pub struct AccountArgs {
    #[command(subcommand)]
    pub command: AccountCommand,
}

#[derive(Debug, Subcommand)]
pub enum AccountCommand {
    /// Open an account and print its first bearer secret once
    Create(CreateArgs),
    /// Buy invoke credits. The USDC amount is computed by the API.
    Credit(CreditArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    #[arg(long, env = "NITRUM_FN_URL", default_value = "http://127.0.0.1:8080")]
    pub url: String,
}

#[derive(Debug, Args)]
pub struct CreditArgs {
    /// Account id returned by `account create`
    #[arg(long)]
    pub account: String,
    /// Number of invoke credits to buy
    #[arg(long)]
    pub invokes: u64,
    #[arg(long, env = "NITRUM_FN_URL", default_value = "http://127.0.0.1:8080")]
    pub url: String,
    /// secp256k1 private key (hex) used to sign the x402 retry
    #[arg(long, env = "NITRUM_FN_PRIVATE_KEY")]
    pub private_key: Option<String>,
    /// EVM chain id for the USDC EIP-712 domain (Base mainnet is 8453)
    #[arg(long, default_value_t = 8453)]
    pub chain_id: u64,
}

pub async fn run(args: AccountArgs) -> Result<()> {
    match args.command {
        AccountCommand::Create(args) => create(args).await,
        AccountCommand::Credit(args) => credit(args).await,
    }
}

async fn create(args: CreateArgs) -> Result<()> {
    let client = http_client()?;
    let endpoint = format!("{}/accounts", args.url.trim_end_matches('/'));
    let response = client
        .post(&endpoint)
        .send()
        .await
        .with_context(|| format!("POST {endpoint}"))?;
    let status = response.status();
    let bytes = response.bytes().await.context("read response")?;
    if !status.is_success() {
        bail!("account create failed ({status}): {}", error_text(&bytes));
    }
    let body: serde_json::Value = serde_json::from_slice(&bytes).context("decode response")?;
    println!(
        "account_id={}\nkey_id={}\nsecret={}\nbalance={}",
        body["account_id"].as_str().unwrap_or(""),
        body["key_id"].as_str().unwrap_or(""),
        body["secret"].as_str().unwrap_or(""),
        body["balance"].as_u64().unwrap_or(0)
    );
    Ok(())
}

async fn credit(args: CreditArgs) -> Result<()> {
    if args.invokes == 0 {
        bail!("--invokes must be greater than zero");
    }
    let client = http_client()?;
    let endpoint = format!(
        "{}/accounts/{}/credit",
        args.url.trim_end_matches('/'),
        args.account
    );
    let body = credit_body(args.invokes);
    let response = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .with_context(|| format!("POST {endpoint}"))?;
    if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
        let status = response.status();
        let bytes = response.bytes().await.context("read response")?;
        if !status.is_success() {
            bail!("account credit failed ({status}): {}", error_text(&bytes));
        }
        print_balance(&bytes)?;
        return Ok(());
    }
    let challenge_bytes = response.bytes().await.context("read challenge")?;
    let challenge = x402::parse_challenge(&String::from_utf8_lossy(&challenge_bytes))?;
    let Some(private_key) = args.private_key.as_deref() else {
        bail!(
            "payment required for {} atomic USDC to {}; set --private-key to sign the retry",
            challenge.amount,
            challenge.pay_to
        );
    };
    let valid_before = unix_now().saturating_add(600);
    let signature = x402::payment_signature(private_key, &challenge, args.chain_id, valid_before)?;
    let paid = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("payment-signature", signature)
        .body(body)
        .send()
        .await
        .with_context(|| format!("POST {endpoint}"))?;
    let status = paid.status();
    let bytes = paid.bytes().await.context("read response")?;
    if !status.is_success() {
        bail!("account credit failed ({status}): {}", error_text(&bytes));
    }
    print_balance(&bytes)
}

fn print_balance(bytes: &[u8]) -> Result<()> {
    let body: serde_json::Value = serde_json::from_slice(bytes).context("decode response")?;
    println!(
        "account_id={} balance={} invokes={}",
        body["account_id"].as_str().unwrap_or(""),
        body["balance"].as_u64().unwrap_or(0),
        body["invokes"].as_u64().unwrap_or(0)
    );
    Ok(())
}

fn error_text(bytes: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(3))
        .build()
        .context("http client")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn credit_sends_invoke_count_not_a_usdc_amount() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });

        let client = http_client().unwrap();
        let body = credit_body(7);
        let _ = client
            .post(format!("http://{addr}/accounts/abc/credit"))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await;

        let raw = captured.await.unwrap();
        let lower = raw.to_ascii_lowercase();
        assert!(lower.contains("\"invokes\":7"), "{raw}");
        assert!(!lower.contains("\"usdc\""), "{raw}");
        assert!(!lower.contains("\"amount\""), "{raw}");
        assert!(!lower.contains("payment-signature"), "{raw}");
    }
}
