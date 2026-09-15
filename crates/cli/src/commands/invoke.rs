use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::Args;
use domain::ContentHash;
use nitrum_verify::{verify_attestation, AttestationResult, VerifyOptions};

const HASH_HEADER: &str = "x-nitrum-fn-hash";
const NONCE_HEADER: &str = "x-nitrum-fn-nonce";
const ATTESTATION_HEADER: &str = "x-nitrum-fn-attestation";

#[derive(Debug, Args)]
pub struct InvokeArgs {
    /// Function name as used in /invoke/{name}
    pub name: String,
    #[arg(
        long,
        env = "NITRUM_FN_INVOKE_URL",
        default_value = "http://127.0.0.1:8081"
    )]
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
    /// Expected wasm content hash (sha256 hex). Compared to `x-nitrum-fn-hash`.
    #[arg(long = "expect-hash", value_name = "HEX")]
    pub expect_hash: Option<String>,
    /// Local `.wasm` file; sha256 is used as `--expect-hash`.
    #[arg(long, value_name = "FILE")]
    pub wasm: Option<PathBuf>,
    /// Pin Nitro PCR0 (hex). Requires attestation; always sends a fresh nonce.
    #[arg(long = "pcr0", value_name = "HEX")]
    pub pcr0: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ErrorBody {
    error: String,
}

#[tracing::instrument(level = "debug", skip_all, fields(name = %args.name), err)]
pub async fn run(args: InvokeArgs) -> Result<()> {
    let expect_hash = resolve_expect_hash(&args)?;
    if args.pcr0.is_some() && expect_hash.is_none() {
        bail!("--pcr0 requires --expect-hash or --wasm so user_data can be checked");
    }

    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(3));
    if args.insecure {
        builder = builder.danger_accept_invalid_certs(true);
    }
    let client = builder.build().context("http client")?;

    let base = args.url.trim_end_matches('/');
    let endpoint = format!("{base}/invoke/{}", args.name);

    let nonce = if args.pcr0.is_some() {
        Some(random_nonce()?)
    } else {
        None
    };

    let mut request = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .body(args.data.clone());
    if let Some(version) = &args.version {
        request = request.header("x-nitrum-fn-version", version.as_str());
    }
    if let Some(ref nonce) = nonce {
        request = request.header(NONCE_HEADER, BASE64.encode(nonce));
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("POST {endpoint}"))?;

    let status = response.status();
    let headers = response.headers().clone();
    let body_bytes = response.bytes().await.context("read response")?;
    let body = String::from_utf8_lossy(&body_bytes).into_owned();

    if !status.is_success() {
        let msg = serde_json::from_str::<ErrorBody>(&body)
            .map(|b| b.error)
            .unwrap_or(body);
        bail!("invoke failed ({status}): {msg}");
    }

    let reported_hash = headers
        .get(HASH_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if let Some(ref h) = reported_hash {
        eprintln!("x-nitrum-fn-hash={h}");
    } else {
        eprintln!("warning: missing {HASH_HEADER} on invoke response");
    }

    if let Some(ref expected) = expect_hash {
        match reported_hash.as_deref() {
            Some(got) if got.eq_ignore_ascii_case(expected) => {}
            Some(got) => bail!("hash mismatch: expected {expected}, got {got}"),
            None => bail!("missing {HASH_HEADER}; cannot verify --expect-hash"),
        }
    }

    if let Some(ref pcr0) = args.pcr0 {
        let nonce = nonce.expect("nonce set when --pcr0");
        let expected = expect_hash.expect("checked above");
        let attestation_b64 = headers
            .get(ATTESTATION_HEADER)
            .and_then(|v| v.to_str().ok())
            .context(format!(
                "missing {ATTESTATION_HEADER}; host crypto API not configured?"
            ))?;
        let doc = BASE64
            .decode(attestation_b64.trim())
            .context("decode attestation header")?;
        verify_invoke_attestation(&doc, &nonce, pcr0, &expected, &body_bytes)?;
    }

    println!("{body}");
    Ok(())
}

fn resolve_expect_hash(args: &InvokeArgs) -> Result<Option<String>> {
    let from_wasm = if let Some(path) = &args.wasm {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        Some(ContentHash::from_bytes(&bytes).to_hex())
    } else {
        None
    };
    let from_flag = args
        .expect_hash
        .as_ref()
        .map(|s| s.trim().to_ascii_lowercase());
    if let (Some(a), Some(b)) = (&from_flag, &from_wasm) {
        if a != b {
            bail!("--expect-hash ({a}) does not match sha256(--wasm) ({b})");
        }
    }
    if let Some(h) = from_flag.or(from_wasm) {
        ContentHash::from_hex(&h).context("invalid expect hash")?;
        Ok(Some(h))
    } else {
        Ok(None)
    }
}

fn random_nonce() -> Result<[u8; 32]> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("generate nonce: {e}"))?;
    Ok(nonce)
}

fn verify_invoke_attestation(
    document: &[u8],
    nonce: &[u8],
    pcr0_hex: &str,
    expect_hash_hex: &str,
    body: &[u8],
) -> Result<()> {
    let mut expected_pcrs = BTreeMap::new();
    expected_pcrs.insert("pcr0".to_string(), pcr0_hex.trim().to_ascii_lowercase());

    let options = VerifyOptions {
        debug: false,
        trusted_root: None,
        nonce: Some(nonce.to_vec()),
        max_age_ms: Some(5 * 60 * 1000),
        expected_pcrs: Some(expected_pcrs),
    };

    let parsed = match verify_attestation(document, &options) {
        AttestationResult::Success(s) => s.document,
        AttestationResult::Failure(f) => bail!("attestation verify failed: {}", f.reason),
    };

    let user_data = parsed
        .user_data
        .as_deref()
        .context("attestation missing user_data")?;
    if user_data.len() != 64 {
        bail!(
            "attestation user_data must be 64 bytes (H || sha256(body)), got {}",
            user_data.len()
        );
    }

    let expect_hash = ContentHash::from_hex(expect_hash_hex)?;
    let body_hash = ContentHash::from_bytes(body);
    let ud_wasm = hex::encode(&user_data[..32]);
    let ud_body = hex::encode(&user_data[32..]);
    if user_data[..32] != *expect_hash.as_bytes() {
        bail!(
            "attestation user_data wasm hash mismatch: expected {}, got {ud_wasm}",
            expect_hash.to_hex(),
        );
    }
    if user_data[32..] != *body_hash.as_bytes() {
        bail!(
            "attestation user_data body hash mismatch: expected {}, got {ud_body}",
            body_hash.to_hex(),
        );
    }

    let pcr0 = parsed
        .pcrs
        .get("pcr0")
        .map(String::as_str)
        .unwrap_or("(missing)");
    eprintln!("attestation: ok (NSM document verified)");
    eprintln!("  pcr0={pcr0}");
    eprintln!("  user_data.wasm_hash={ud_wasm}");
    eprintln!("  user_data.body_hash={ud_body}");
    eprintln!("  document={}", BASE64.encode(document));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_hash_from_bytes() {
        let dir = std::env::temp_dir().join(format!("nitrum-fn-invoke-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.wasm");
        let wasm = b"\0asm test";
        std::fs::write(&path, wasm).unwrap();
        let expected = ContentHash::from_bytes(wasm).to_hex();

        let args = InvokeArgs {
            name: "f".into(),
            url: "http://127.0.0.1:8081".into(),
            data: "{}".into(),
            version: None,
            insecure: false,
            expect_hash: None,
            wasm: Some(path),
            pcr0: None,
        };
        assert_eq!(
            resolve_expect_hash(&args).unwrap().as_deref(),
            Some(expected.as_str())
        );
    }
}
