use std::io::{BufRead, IsTerminal, Write};
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
    /// Refuse to sign if the quoted price exceeds this many atomic USDC (6 decimals).
    /// Also skips the confirmation prompt.
    #[arg(long)]
    pub max_usdc: Option<u64>,
    /// Sign without asking. Prefer --max-usdc in scripts.
    #[arg(long)]
    pub yes: bool,
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
    eprintln!(
        "price: {} atomic USDC ({} USDC) to {} on {} (asset {})",
        challenge.amount,
        format_usdc(challenge.amount),
        challenge.pay_to,
        challenge.network,
        challenge.asset
    );
    match price_decision(
        challenge.amount,
        args.max_usdc,
        args.yes,
        std::io::stdin().is_terminal(),
    ) {
        PriceDecision::Sign => {}
        PriceDecision::Ask => {
            if !confirm("Sign and pay? [y/N] ")? {
                bail!("cancelled; nothing was signed");
            }
        }
        PriceDecision::Refuse(reason) => bail!("{reason}; nothing was signed"),
    }
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

#[derive(Debug, PartialEq, Eq)]
enum PriceDecision {
    Sign,
    Ask,
    Refuse(String),
}

/// A price cap always applies. Without one, the caller must pass `--yes` or confirm on a terminal.
fn price_decision(
    amount: u64,
    max_usdc: Option<u64>,
    yes: bool,
    interactive: bool,
) -> PriceDecision {
    if let Some(max) = max_usdc {
        return if amount > max {
            PriceDecision::Refuse(format!(
                "price {amount} atomic USDC exceeds --max-usdc {max}"
            ))
        } else {
            PriceDecision::Sign
        };
    }
    if yes {
        PriceDecision::Sign
    } else if interactive {
        PriceDecision::Ask
    } else {
        PriceDecision::Refuse(
            "pass --max-usdc <atomic USDC> or --yes to sign without a prompt".into(),
        )
    }
}

fn format_usdc(atomic: u64) -> String {
    format!("{}.{:06}", atomic / 1_000_000, atomic % 1_000_000)
}

fn confirm(prompt: &str) -> Result<bool> {
    eprint!("{prompt}");
    std::io::stderr().flush().context("flush prompt")?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("read confirmation")?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
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

    #[test]
    fn a_price_cap_refuses_a_higher_quote_even_with_yes() {
        let decision = price_decision(200, Some(100), true, true);
        assert!(matches!(decision, PriceDecision::Refuse(_)), "{decision:?}");
    }

    #[test]
    fn a_quote_within_the_cap_signs_without_a_prompt() {
        assert_eq!(
            price_decision(100, Some(100), false, true),
            PriceDecision::Sign
        );
    }

    #[test]
    fn without_a_cap_a_terminal_is_asked_and_a_script_is_refused() {
        assert_eq!(price_decision(5, None, false, true), PriceDecision::Ask);
        assert!(matches!(
            price_decision(5, None, false, false),
            PriceDecision::Refuse(_)
        ));
        assert_eq!(price_decision(5, None, true, false), PriceDecision::Sign);
    }

    #[test]
    fn usdc_is_shown_with_six_decimals() {
        assert_eq!(format_usdc(20_000), "0.020000");
        assert_eq!(format_usdc(1_500_000), "1.500000");
    }

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
