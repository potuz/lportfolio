use std::collections::BTreeMap;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use tracing::warn;

use crate::chain::Chain;
use crate::config::Config;
use crate::csm::CsmReader;
use crate::db::Db;
use crate::rpc::ChainClient;
use crate::staking::BeaconNodeClient;

const STAKING_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const GWEI_PER_ETH: u64 = 1_000_000_000;

#[derive(Debug, Clone)]
pub struct NativeRow {
    pub alias: String,
    pub address: Address,
    pub chain: Chain,
    pub balance_wei: U256,
}

#[derive(Debug, Clone)]
pub struct StakingRow {
    pub alias: String,
    pub validator_count: u64,
    pub total_balance_gwei: u64,
    pub from_cache: bool,
}

#[derive(Debug, Clone)]
pub struct CsmRow {
    pub operator_id: u64,
    pub bond_steth_wei: U256,
}

#[derive(Debug, Clone, Default)]
pub struct PortfolioSnapshot {
    pub native: Vec<NativeRow>,
    pub staking: Vec<StakingRow>,
    pub csm: Vec<CsmRow>,
}

impl PortfolioSnapshot {
    pub fn grand_total_eth_wei(&self) -> U256 {
        let mut total = U256::ZERO;
        for r in &self.native {
            total += r.balance_wei;
        }
        for r in &self.staking {
            total += gwei_to_wei(r.total_balance_gwei);
        }
        total
    }

    pub fn grand_total_steth_wei(&self) -> U256 {
        let mut total = U256::ZERO;
        for r in &self.csm {
            total += r.bond_steth_wei;
        }
        total
    }

    pub fn native_total_by_chain(&self) -> BTreeMap<Chain, U256> {
        let mut out: BTreeMap<Chain, U256> = BTreeMap::new();
        for r in &self.native {
            *out.entry(r.chain).or_insert(U256::ZERO) += r.balance_wei;
        }
        out
    }

    /// Per-alias native total (chains summed). Staking is reported separately
    /// in the summary under its own row, since validator indices aren't tied
    /// to a specific eth1-address alias.
    pub fn native_total_by_alias(&self) -> BTreeMap<String, U256> {
        let mut out: BTreeMap<String, U256> = BTreeMap::new();
        for r in &self.native {
            *out.entry(r.alias.clone()).or_insert(U256::ZERO) += r.balance_wei;
        }
        out
    }
}

pub fn gwei_to_wei(gwei: u64) -> U256 {
    U256::from(gwei) * U256::from(GWEI_PER_ETH)
}

/// Build the full snapshot. Uses the staking cache unless `refresh` is true.
pub async fn build_snapshot(cfg: &Config, db: &mut Db, refresh: bool) -> Result<PortfolioSnapshot> {
    let native = collect_native(cfg).await?;
    let staking = collect_staking(cfg, db, refresh).await?;
    let csm = collect_csm(cfg).await?;
    Ok(PortfolioSnapshot {
        native,
        staking,
        csm,
    })
}

async fn collect_native(cfg: &Config) -> Result<Vec<NativeRow>> {
    let mut clients: Vec<(Chain, ChainClient)> = Vec::new();
    for (chain, cc) in &cfg.chains {
        let client = ChainClient::connect(*chain, &cc.rpc_url)?;
        client
            .verify_chain_id()
            .await
            .with_context(|| format!("verifying RPC for {}", chain.name()))?;
        clients.push((*chain, client));
    }

    let mut rows = Vec::with_capacity(cfg.addresses.len() * clients.len());
    for (alias, addr) in &cfg.addresses {
        for (chain, client) in &clients {
            match client.balance(*addr).await {
                Ok(balance) => rows.push(NativeRow {
                    alias: alias.clone(),
                    address: *addr,
                    chain: *chain,
                    balance_wei: balance,
                }),
                Err(e) => {
                    warn!(
                        alias,
                        chain = chain.name(),
                        error = %e,
                        "balance fetch failed; omitting from table"
                    );
                }
            }
        }
    }
    Ok(rows)
}

/// Synthetic cache key for the aggregate validator-balance row.
/// Uses `Address::ZERO` to avoid colliding with any real address.
const STAKING_CACHE_KEY: Address = Address::ZERO;

async fn collect_staking(cfg: &Config, db: &mut Db, refresh: bool) -> Result<Vec<StakingRow>> {
    let Some(beacon_url) = cfg.beacon_url.as_deref() else {
        return Ok(Vec::new());
    };
    if cfg.validator_indices.is_empty() {
        return Ok(Vec::new());
    }

    let max_age = if refresh {
        Duration::ZERO
    } else {
        STAKING_CACHE_TTL
    };

    if let Some(cached) = db.read_staking_snapshot(STAKING_CACHE_KEY, max_age)?
        && cached.validator_count > 0
    {
        return Ok(vec![StakingRow {
            alias: "validators".into(),
            validator_count: cached.validator_count,
            total_balance_gwei: cached.total_balance_gwei,
            from_cache: true,
        }]);
    }

    let beacon = BeaconNodeClient::new(beacon_url.to_string())?;
    let summary = match beacon.validator_balances(&cfg.validator_indices).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "beacon node lookup failed; skipping staking section");
            return Ok(Vec::new());
        }
    };
    db.upsert_staking_snapshot(
        STAKING_CACHE_KEY,
        summary.validator_count,
        summary.total_balance_gwei,
    )?;

    if summary.validator_count == 0 {
        return Ok(Vec::new());
    }
    Ok(vec![StakingRow {
        alias: "validators".into(),
        validator_count: summary.validator_count,
        total_balance_gwei: summary.total_balance_gwei,
        from_cache: false,
    }])
}

async fn collect_csm(cfg: &Config) -> Result<Vec<CsmRow>> {
    if cfg.csm_operator_ids.is_empty() {
        return Ok(Vec::new());
    }
    let Some(mainnet_cfg) = cfg.chains.get(&Chain::Mainnet) else {
        warn!("CSM operator IDs configured but no mainnet RPC; skipping CSM section");
        return Ok(Vec::new());
    };
    let reader = CsmReader::connect(&mainnet_cfg.rpc_url)?;
    let bonds = reader.read_bonds(&cfg.csm_operator_ids).await?;
    Ok(bonds
        .into_iter()
        .map(|b| CsmRow {
            operator_id: b.operator_id,
            bond_steth_wei: b.bond_steth_wei,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grand_total_sums_native_and_staking() {
        let snap = PortfolioSnapshot {
            native: vec![NativeRow {
                alias: "a".into(),
                address: Address::default(),
                chain: Chain::Mainnet,
                balance_wei: U256::from(10u64).pow(U256::from(18)),
            }],
            staking: vec![StakingRow {
                alias: "a".into(),
                validator_count: 1,
                total_balance_gwei: 32_000_000_000,
                from_cache: false,
            }],
            csm: vec![],
        };
        let total = snap.grand_total_eth_wei();
        let expected = U256::from(33u64) * U256::from(10u64).pow(U256::from(18));
        assert_eq!(total, expected);
    }

    #[test]
    fn native_by_chain_aggregates() {
        let snap = PortfolioSnapshot {
            native: vec![
                NativeRow {
                    alias: "a".into(),
                    address: Address::default(),
                    chain: Chain::Mainnet,
                    balance_wei: U256::from(1u64),
                },
                NativeRow {
                    alias: "b".into(),
                    address: Address::default(),
                    chain: Chain::Mainnet,
                    balance_wei: U256::from(2u64),
                },
                NativeRow {
                    alias: "a".into(),
                    address: Address::default(),
                    chain: Chain::Arbitrum,
                    balance_wei: U256::from(5u64),
                },
            ],
            staking: vec![],
            csm: vec![],
        };
        let by_chain = snap.native_total_by_chain();
        assert_eq!(by_chain[&Chain::Mainnet], U256::from(3u64));
        assert_eq!(by_chain[&Chain::Arbitrum], U256::from(5u64));
    }
}
