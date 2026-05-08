use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;

const ENDPOINT: &str = "https://api.coingecko.com/api/v3/simple/price";

/// Map our display symbols to CoinGecko coin IDs. Multiple symbols can share
/// the same CoinGecko id (e.g. USDC.e on Arbitrum tracks the same usd-coin
/// price as native USDC).
const SYMBOL_TO_ID: &[(&str, &str)] = &[
    ("ETH", "ethereum"),
    ("stETH", "staked-ether"),
    ("USDC", "usd-coin"),
    ("USDC.e", "usd-coin"),
    ("USDT", "tether"),
    ("USDT0", "tether"),
    ("ARB", "arbitrum"),
    ("DAI", "dai"),
];

#[derive(Debug, Clone, Default)]
pub struct PriceTable {
    /// display_symbol → USD price per unit.
    usd: HashMap<String, f64>,
}

impl PriceTable {
    pub fn lookup(&self, symbol: &str) -> Option<f64> {
        self.usd.get(symbol).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.usd.is_empty()
    }

    #[cfg(test)]
    pub fn insert_for_test(&mut self, symbol: &str, usd: f64) {
        self.usd.insert(symbol.to_string(), usd);
    }
}

pub struct PriceClient {
    http: Client,
}

impl PriceClient {
    pub fn new() -> Result<Self> {
        // CoinGecko's WAF rejects requests without a recognizable user-agent.
        let http = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("lportfolio/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("building http client")?;
        Ok(Self { http })
    }

    pub async fn fetch_for_symbols(&self, symbols: &[&str]) -> Result<PriceTable> {
        let mut needed_ids: BTreeSet<&'static str> = BTreeSet::new();
        let mut wanted: Vec<(&str, &'static str)> = Vec::new();
        for sym in symbols {
            if let Some((_, id)) = SYMBOL_TO_ID.iter().find(|(s, _)| s == sym) {
                needed_ids.insert(id);
                wanted.push((sym, id));
            }
        }
        if needed_ids.is_empty() {
            return Ok(PriceTable::default());
        }

        let id_param = needed_ids.iter().copied().collect::<Vec<_>>().join(",");
        let url = format!("{ENDPOINT}?ids={id_param}&vs_currencies=usd");

        let resp: HashMap<String, HashMap<String, f64>> = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("CoinGecko GET {url} failed"))?
            .error_for_status()
            .with_context(|| format!("CoinGecko {url} returned non-2xx"))?
            .json()
            .await
            .with_context(|| format!("parsing CoinGecko JSON from {url}"))?;

        let mut usd = HashMap::new();
        for (sym, id) in &wanted {
            if let Some(p) = resp.get(*id).and_then(|m| m.get("usd")) {
                usd.insert((*sym).to_string(), *p);
            }
        }
        Ok(PriceTable { usd })
    }
}

/// Convert a U256 amount (with `decimals` precision) to a native f64 number
/// of units. Loses precision below ~15 significant decimal digits, which is
/// far below the cent precision we render at.
pub fn u256_to_f64(amount: alloy::primitives::U256, decimals: u8) -> f64 {
    let raw: f64 = amount.to_string().parse().unwrap_or(0.0);
    raw / 10f64.powi(i32::from(decimals))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::U256;

    #[test]
    fn one_eth_converts_to_one() {
        let one = U256::from(10u64).pow(U256::from(18));
        assert_eq!(u256_to_f64(one, 18), 1.0);
    }

    #[test]
    fn fractional_eth_converts() {
        let half = U256::from(10u64).pow(U256::from(18)) / U256::from(2u64);
        assert_eq!(u256_to_f64(half, 18), 0.5);
    }

    #[test]
    fn empty_table_lookup_returns_none() {
        let t = PriceTable::default();
        assert!(t.lookup("ETH").is_none());
        assert!(t.is_empty());
    }
}
