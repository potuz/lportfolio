use std::collections::{BTreeMap, BTreeSet};
use std::env;

use alloy::primitives::Address;
use anyhow::{Context, Result, bail};

use crate::chain::Chain;

#[derive(Debug, Clone)]
pub struct Config {
    pub addresses: BTreeMap<String, Address>,
    pub chains: BTreeMap<Chain, ChainConfig>,
    pub etherscan_api_key: Option<String>,
    pub beacon_url: Option<String>,
    pub validator_indices: Vec<u64>,
    pub csm_operator_ids: Vec<u64>,
    pub token_whitelist: Vec<String>,
    pub safes: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub rpc_url: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        let _ = dotenvy::dotenv();

        let addresses_raw = env::var("LPORTFOLIO_ADDRESSES")
            .context("LPORTFOLIO_ADDRESSES is not set; copy .env.example to .env and fill it in")?;
        let addresses = parse_addresses(&addresses_raw)?;

        let mut chains = BTreeMap::new();
        for chain in Chain::ALL {
            match env::var(chain.rpc_env_key()) {
                Ok(url) if !url.trim().is_empty() => {
                    chains.insert(*chain, ChainConfig { rpc_url: url });
                }
                _ => {}
            }
        }

        let etherscan_api_key = env::var("LPORTFOLIO_ETHERSCAN_API_KEY")
            .ok()
            .filter(|s| !s.is_empty());

        let beacon_url = env::var("LPORTFOLIO_BEACON_URL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let validator_indices = env::var("LPORTFOLIO_VALIDATOR_INDICES")
            .ok()
            .map(|s| parse_u64_list(&s, "validator index"))
            .transpose()?
            .unwrap_or_default();

        let csm_operator_ids = env::var("LPORTFOLIO_LIDO_CSM_OPERATOR_IDS")
            .ok()
            .map(|s| parse_u64_list(&s, "CSM operator id"))
            .transpose()?
            .unwrap_or_default();

        let token_whitelist = env::var("LPORTFOLIO_TOKEN_WHITELIST")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let safes: BTreeSet<String> = env::var("LPORTFOLIO_SAFES")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        for alias in &safes {
            if !addresses.contains_key(alias) {
                bail!("LPORTFOLIO_SAFES references unknown alias: {alias:?}");
            }
        }

        Ok(Self {
            addresses,
            chains,
            etherscan_api_key,
            beacon_url,
            validator_indices,
            csm_operator_ids,
            token_whitelist,
            safes,
        })
    }
}

fn parse_u64_list(raw: &str, what: &str) -> Result<Vec<u64>> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<u64>()
                .with_context(|| format!("invalid {what} {s:?}"))
        })
        .collect()
}

fn parse_addresses(raw: &str) -> Result<BTreeMap<String, Address>> {
    let mut out = BTreeMap::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (alias, addr) = entry
            .split_once('=')
            .with_context(|| format!("address entry missing `=`: {entry:?}"))?;
        let alias = alias.trim().to_string();
        if alias.is_empty() {
            bail!("empty alias in LPORTFOLIO_ADDRESSES");
        }
        let addr_str = addr.trim();
        let addr: Address = addr_str
            .parse()
            .with_context(|| format!("invalid address for alias {alias:?}: {addr_str:?}"))?;
        if out.insert(alias.clone(), addr).is_some() {
            bail!("duplicate alias: {alias}");
        }
    }
    if out.is_empty() {
        bail!("LPORTFOLIO_ADDRESSES is empty");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_entry() {
        let raw = "hot=0x0000000000000000000000000000000000000001";
        let out = parse_addresses(raw).unwrap();
        assert_eq!(out.len(), 1);
        let mut expected = [0u8; 20];
        expected[19] = 1;
        assert_eq!(out["hot"], Address::from(expected));
    }

    #[test]
    fn parses_multiple_entries_with_whitespace() {
        let raw = " hot = 0x0000000000000000000000000000000000000001 , cold=0x0000000000000000000000000000000000000002 ";
        let out = parse_addresses(raw).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains_key("hot"));
        assert!(out.contains_key("cold"));
    }

    #[test]
    fn rejects_duplicate_alias() {
        let raw = "a=0x0000000000000000000000000000000000000001,a=0x0000000000000000000000000000000000000002";
        assert!(parse_addresses(raw).is_err());
    }

    #[test]
    fn rejects_missing_equals() {
        let raw = "0x0000000000000000000000000000000000000001";
        assert!(parse_addresses(raw).is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_addresses("").is_err());
        assert!(parse_addresses("  ").is_err());
    }

    #[test]
    fn rejects_bad_address() {
        let raw = "alias=0xnothex";
        assert!(parse_addresses(raw).is_err());
    }
}
