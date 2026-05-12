use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "cli")]
use std::env;
use std::path::{Path, PathBuf};

use alloy::primitives::Address;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

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
    pub db_path_override: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub rpc_url: String,
}

impl Config {
    #[cfg(feature = "cli")]
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

        let db_path_override = env::var("LPORTFOLIO_DB_PATH")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);

        Ok(Self {
            addresses,
            chains,
            etherscan_api_key,
            beacon_url,
            validator_indices,
            csm_operator_ids,
            token_whitelist,
            safes,
            db_path_override,
        })
    }

    /// Construct an empty `Config` — used by the Android app on first launch
    /// before the user has saved any settings.
    pub fn empty() -> Self {
        Self {
            addresses: BTreeMap::new(),
            chains: BTreeMap::new(),
            etherscan_api_key: None,
            beacon_url: None,
            validator_indices: Vec::new(),
            csm_operator_ids: Vec::new(),
            token_whitelist: Vec::new(),
            safes: BTreeSet::new(),
            db_path_override: None,
        }
    }

    /// Read a TOML config file. Validates the same invariants as `Config::load`.
    pub fn from_toml(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let parsed: ConfigToml = toml::from_str(&text)
            .with_context(|| format!("parsing config file {}", path.display()))?;
        parsed.into_config()
    }

    /// Serialize this config to a TOML file. Parent directory is created if needed.
    pub fn to_toml(&self, path: &Path) -> Result<()> {
        let toml_repr = ConfigToml::from_config(self);
        let text = toml::to_string_pretty(&toml_repr).context("serializing config to TOML")?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        std::fs::write(path, text)
            .with_context(|| format!("writing config file {}", path.display()))?;
        Ok(())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConfigToml {
    #[serde(default)]
    addresses: Vec<AddressEntry>,
    #[serde(default)]
    chains: BTreeMap<String, ChainEntry>,
    #[serde(default)]
    beacon: Option<BeaconEntry>,
    #[serde(default)]
    csm: Option<CsmEntry>,
    #[serde(default)]
    tokens: Option<TokensEntry>,
    #[serde(default)]
    safes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    db_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AddressEntry {
    alias: String,
    address: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainEntry {
    rpc_url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BeaconEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    validator_indices: Vec<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CsmEntry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    operator_ids: Vec<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TokensEntry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    whitelist: Vec<String>,
}

impl ConfigToml {
    fn from_config(cfg: &Config) -> Self {
        let addresses = cfg
            .addresses
            .iter()
            .map(|(alias, addr)| AddressEntry {
                alias: alias.clone(),
                address: format!("{addr:#x}"),
            })
            .collect();
        let chains = cfg
            .chains
            .iter()
            .map(|(chain, cc)| {
                (
                    chain.name().to_string(),
                    ChainEntry {
                        rpc_url: cc.rpc_url.clone(),
                    },
                )
            })
            .collect();
        let beacon = if cfg.beacon_url.is_some() || !cfg.validator_indices.is_empty() {
            Some(BeaconEntry {
                url: cfg.beacon_url.clone(),
                validator_indices: cfg.validator_indices.clone(),
            })
        } else {
            None
        };
        let csm = if cfg.csm_operator_ids.is_empty() {
            None
        } else {
            Some(CsmEntry {
                operator_ids: cfg.csm_operator_ids.clone(),
            })
        };
        let tokens = if cfg.token_whitelist.is_empty() {
            None
        } else {
            Some(TokensEntry {
                whitelist: cfg.token_whitelist.clone(),
            })
        };
        Self {
            addresses,
            chains,
            beacon,
            csm,
            tokens,
            safes: cfg.safes.iter().cloned().collect(),
            db_path: cfg
                .db_path_override
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        }
    }

    fn into_config(self) -> Result<Config> {
        let mut addresses = BTreeMap::new();
        for entry in self.addresses {
            let alias = entry.alias.trim();
            if alias.is_empty() {
                bail!("address entry with empty alias");
            }
            let addr: Address = entry.address.trim().parse().with_context(|| {
                format!("invalid address for alias {alias:?}: {:?}", entry.address)
            })?;
            if addresses.insert(alias.to_string(), addr).is_some() {
                bail!("duplicate alias: {alias}");
            }
        }
        if addresses.is_empty() {
            bail!("config has no addresses");
        }

        let mut chains = BTreeMap::new();
        for (key, entry) in self.chains {
            let chain = Chain::ALL
                .iter()
                .copied()
                .find(|c| c.name() == key.as_str())
                .ok_or_else(|| anyhow::anyhow!("unknown chain key: {key:?}"))?;
            let rpc_url = entry.rpc_url.trim();
            if rpc_url.is_empty() {
                continue;
            }
            chains.insert(
                chain,
                ChainConfig {
                    rpc_url: rpc_url.to_string(),
                },
            );
        }

        let (beacon_url, validator_indices) = match self.beacon {
            Some(b) => (
                b.url
                    .as_ref()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                b.validator_indices,
            ),
            None => (None, Vec::new()),
        };

        let csm_operator_ids = self.csm.map(|c| c.operator_ids).unwrap_or_default();
        let token_whitelist = self.tokens.map(|t| t.whitelist).unwrap_or_default();

        let mut safes = BTreeSet::new();
        for alias in self.safes {
            let alias = alias.trim();
            if alias.is_empty() {
                continue;
            }
            if !addresses.contains_key(alias) {
                bail!("safes references unknown alias: {alias:?}");
            }
            safes.insert(alias.to_string());
        }

        let db_path_override = self
            .db_path
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);

        Ok(Config {
            addresses,
            chains,
            etherscan_api_key: None,
            beacon_url,
            validator_indices,
            csm_operator_ids,
            token_whitelist,
            safes,
            db_path_override,
        })
    }
}

#[cfg_attr(not(feature = "cli"), allow(dead_code))]
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

#[cfg_attr(not(feature = "cli"), allow(dead_code))]
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

    fn sample_config() -> Config {
        let mut addresses = BTreeMap::new();
        addresses.insert(
            "hot".to_string(),
            "0x0000000000000000000000000000000000000001"
                .parse()
                .unwrap(),
        );
        addresses.insert(
            "cold".to_string(),
            "0x0000000000000000000000000000000000000002"
                .parse()
                .unwrap(),
        );
        let mut chains = BTreeMap::new();
        chains.insert(
            Chain::Mainnet,
            ChainConfig {
                rpc_url: "https://eth.example".to_string(),
            },
        );
        chains.insert(
            Chain::Arbitrum,
            ChainConfig {
                rpc_url: "https://arb.example".to_string(),
            },
        );
        let mut safes = BTreeSet::new();
        safes.insert("cold".to_string());
        Config {
            addresses,
            chains,
            etherscan_api_key: None,
            beacon_url: Some("http://localhost:5052".to_string()),
            validator_indices: vec![1, 2, 3],
            csm_operator_ids: vec![42],
            token_whitelist: vec!["USDC".to_string(), "USDT".to_string()],
            safes,
            db_path_override: Some(PathBuf::from("/tmp/db.sqlite")),
        }
    }

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lportfolio-config-test-{}-{}-{}.toml",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn roundtrip_toml() {
        let cfg = sample_config();
        let path = tmp_path("roundtrip");
        cfg.to_toml(&path).unwrap();
        let loaded = Config::from_toml(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(cfg.addresses, loaded.addresses);
        assert_eq!(cfg.chains.len(), loaded.chains.len());
        for (c, cc) in &cfg.chains {
            assert_eq!(cc.rpc_url, loaded.chains.get(c).unwrap().rpc_url);
        }
        assert_eq!(cfg.beacon_url, loaded.beacon_url);
        assert_eq!(cfg.validator_indices, loaded.validator_indices);
        assert_eq!(cfg.csm_operator_ids, loaded.csm_operator_ids);
        assert_eq!(cfg.token_whitelist, loaded.token_whitelist);
        assert_eq!(cfg.safes, loaded.safes);
        assert_eq!(cfg.db_path_override, loaded.db_path_override);
    }

    #[test]
    fn from_toml_rejects_unknown_safe() {
        let text = r#"
            safes = ["ghost"]

            [[addresses]]
            alias = "hot"
            address = "0x0000000000000000000000000000000000000001"
        "#;
        let path = tmp_path("bad-safe");
        std::fs::write(&path, text).unwrap();
        let err = Config::from_toml(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(
            format!("{err:#}").contains("ghost"),
            "expected error to mention alias, got: {err:#}"
        );
    }

    #[test]
    fn from_toml_accepts_partial() {
        let text = r#"
            [[addresses]]
            alias = "hot"
            address = "0x0000000000000000000000000000000000000001"
        "#;
        let path = tmp_path("partial");
        std::fs::write(&path, text).unwrap();
        let cfg = Config::from_toml(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(cfg.addresses.len(), 1);
        assert!(cfg.chains.is_empty());
        assert_eq!(cfg.beacon_url, None);
        assert!(cfg.validator_indices.is_empty());
        assert!(cfg.csm_operator_ids.is_empty());
        assert!(cfg.token_whitelist.is_empty());
        assert!(cfg.safes.is_empty());
        assert_eq!(cfg.db_path_override, None);
    }

    #[test]
    fn from_toml_rejects_unknown_chain() {
        let text = r#"
            [[addresses]]
            alias = "hot"
            address = "0x0000000000000000000000000000000000000001"

            [chains.solana]
            rpc_url = "https://solana.example"
        "#;
        let path = tmp_path("bad-chain");
        std::fs::write(&path, text).unwrap();
        let err = Config::from_toml(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(format!("{err:#}").contains("solana"));
    }
}
