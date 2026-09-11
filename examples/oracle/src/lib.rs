//! Price oracle: fetch token USD prices from CoinGecko inside the enclave.

use runtime::http::{Client, Request};
use runtime::Error;
use serde::Deserialize;
use serde_json::{json, Value};
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
async fn handler(req: Request) -> Result<Value, Error> {
    let body: InvokeBody = req.json()?;
    if body.ids.is_empty() {
        return Ok(json!({ "error": "ids must not be empty" }));
    }

    let mut cg_ids = Vec::new();
    let mut aliases = HashMap::new();
    for id in &body.ids {
        let cg = map_id(id);
        aliases.insert(cg.clone(), id.clone());
        cg_ids.push(cg);
    }

    let ids_param = cg_ids.join(",");
    let url = format!(
        "https://api.coingecko.com/api/v3/simple/price?ids={ids_param}&vs_currencies=usd"
    );

    let upstream = Client::new().get(&url).send().await?;
    if upstream.status() != 200 {
        return Ok(json!({
            "source": url,
            "error": format!("upstream status {}", upstream.status()),
        }));
    }

    let raw: HashMap<String, CoinGeckoPrice> = upstream.json()?;
    let mut prices = serde_json::Map::new();
    for (cg_id, alias) in aliases {
        if let Some(entry) = raw.get(&cg_id) {
            prices.insert(alias.clone(), json!(entry.usd));
        }
    }

    Ok(json!({
        "source": url,
        "prices": prices,
    }))
}

fn map_id(id: &str) -> String {
    match id.to_ascii_lowercase().as_str() {
        "btc" => "bitcoin".into(),
        "eth" => "ethereum".into(),
        other => other.to_string(),
    }
}
