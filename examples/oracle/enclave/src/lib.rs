//! Price oracle: fetch token USD prices from CoinGecko inside the enclave.
//!
//! Successful responses use a **canonical, no-whitespace** JSON body so on-chain
//! verifiers can rebuild the exact bytes hashed into Nitro `user_data`:
//!
//! ```json
//! {"ids":["eth"],"prices":[350012000000]}
//! ```
//!
//! Prices are USD × 1e8, rounded to nearest integer.

use runtime::http::{Client, Request, Response};
use runtime::Error;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct InvokeBody {
    #[serde(default)]
    ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CoinGeckoPrice {
    usd: f64,
}

#[runtime::main]
async fn handler(req: Request) -> Result<Response, Error> {
    let body: InvokeBody = req.json()?;
    if body.ids.is_empty() {
        return Ok(err_json("ids must not be empty"));
    }

    let mut cg_ids = Vec::with_capacity(body.ids.len());
    for id in &body.ids {
        if !is_safe_id(id) {
            return Ok(err_json("id must be lowercase alphanumeric / hyphen / underscore"));
        }
        cg_ids.push(map_id(id));
    }

    let ids_param = cg_ids.join(",");
    let url = format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={ids_param}&vs_currencies=usd"
    );

    let upstream = Client::new().get(&url).send().await?;
    if upstream.status() != 200 {
        return Ok(err_json(&format!("upstream status {}", upstream.status())));
    }

    let raw: HashMap<String, CoinGeckoPrice> = upstream.json()?;
    let mut prices = Vec::with_capacity(body.ids.len());
    for (alias, cg_id) in body.ids.iter().zip(cg_ids.iter()) {
        let Some(entry) = raw.get(cg_id) else {
            return Ok(err_json(&format!("missing price for {alias}")));
        };
        prices.push(usd_to_fixed(entry.usd));
    }

    let canonical = encode_canonical(&body.ids, &prices);
    Ok(Response::builder()
        .header("content-type", "application/json")
        .body(canonical.into_bytes())
        .build())
}

/// `{"ids":["a","b"],"prices":[1,2]}` — no whitespace; ids already validated.
fn encode_canonical(ids: &[String], prices: &[u64]) -> String {
    debug_assert_eq!(ids.len(), prices.len());
    let mut out = String::from(r#"{"ids":["#);
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(id);
        out.push('"');
    }
    out.push_str(r#"],"prices":["#);
    for (i, price) in prices.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&price.to_string());
    }
    out.push_str("]}");
    out
}

fn usd_to_fixed(usd: f64) -> u64 {
    (usd * 100_000_000.0).round() as u64
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn err_json(msg: &str) -> Response {
    // Errors are not submitted on-chain; compactness is enough.
    let body = format!(r#"{{"error":"{msg}"}}"#);
    Response::builder()
        .status(400)
        .header("content-type", "application/json")
        .body(body.into_bytes())
        .build()
}

fn map_id(id: &str) -> String {
    match id {
        "btc" => "bitcoin".into(),
        "eth" => "ethereum".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_encoding_matches_contract_schema() {
        let ids = vec!["eth".into(), "btc".into()];
        let prices = vec![350_012_000_000u64, 9_500_000_000_000u64];
        assert_eq!(
            encode_canonical(&ids, &prices),
            r#"{"ids":["eth","btc"],"prices":[350012000000,9500000000000]}"#
        );
    }

    #[test]
    fn usd_scale() {
        assert_eq!(usd_to_fixed(3500.12), 350_012_000_000);
        assert_eq!(usd_to_fixed(1.0), 100_000_000);
    }
}
