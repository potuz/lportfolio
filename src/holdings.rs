use std::collections::BTreeSet;
use std::time::Duration;

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use tracing::warn;

use crate::chain::Chain;
use crate::config::Config;
use crate::csm::CsmReader;
use crate::db::Db;
use crate::prices::{PriceClient, PriceTable, u256_to_f64};
use crate::rpc::ChainClient;
use crate::splits::SplitsReader;
use crate::staking::BeaconNodeClient;
use crate::tokens;

const STAKING_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const GWEI_PER_ETH: u64 = 1_000_000_000;

#[derive(Debug, Clone)]
pub struct TokenBalance {
    pub display_symbol: String,
    pub decimals: u8,
    pub amount: U256,
}

#[derive(Debug, Clone)]
pub struct NativeRow {
    pub alias: String,
    pub address: Address,
    pub chain: Chain,
    pub balance_wei: U256,
    pub tokens: Vec<TokenBalance>,
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

#[derive(Debug, Clone)]
pub struct SplitsRow {
    pub alias: String,
    pub address: Address,
    pub chain: Chain,
    /// `None` ⇒ native ETH claim; `Some(addr)` ⇒ ERC-20 contract address.
    pub token: Option<Address>,
    pub display_symbol: String,
    pub decimals: u8,
    pub amount: U256,
}

#[derive(Debug, Clone, Default)]
pub struct PortfolioSnapshot {
    pub native: Vec<NativeRow>,
    pub staking: Vec<StakingRow>,
    pub csm: Vec<CsmRow>,
    pub splits: Vec<SplitsRow>,
    pub prices: PriceTable,
}

impl PortfolioSnapshot {
    /// Total USD value across native, staking (in ETH), and CSM (in stETH).
    /// Returns `None` if no ETH price was fetched (in which case all ETH-based
    /// totals are uncomputable).
    pub fn grand_total_usd(&self) -> Option<f64> {
        let eth_usd = self.prices.lookup("ETH")?;
        let mut total = 0.0;
        for row in &self.native {
            total += u256_to_f64(row.balance_wei, 18) * eth_usd;
            for tok in &row.tokens {
                if let Some(p) = self.prices.lookup(&tok.display_symbol) {
                    total += u256_to_f64(tok.amount, tok.decimals) * p;
                }
            }
        }
        for row in &self.staking {
            total += (row.total_balance_gwei as f64) / 1e9 * eth_usd;
        }
        if let Some(steth) = self.prices.lookup("stETH") {
            for row in &self.csm {
                total += u256_to_f64(row.bond_steth_wei, 18) * steth;
            }
        }
        for row in &self.splits {
            let price = match row.token {
                None => Some(eth_usd),
                Some(_) => self.prices.lookup(&row.display_symbol),
            };
            if let Some(p) = price {
                total += u256_to_f64(row.amount, row.decimals) * p;
            }
        }
        Some(total)
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
    let splits = collect_splits(cfg).await?;
    let prices = collect_prices(&native, &staking, &csm, &splits).await;
    Ok(PortfolioSnapshot {
        native,
        staking,
        csm,
        splits,
        prices,
    })
}

async fn collect_prices(
    native: &[NativeRow],
    staking: &[StakingRow],
    csm: &[CsmRow],
    splits: &[SplitsRow],
) -> PriceTable {
    let mut symbols: BTreeSet<&str> = BTreeSet::new();
    if !native.is_empty() || !staking.is_empty() {
        symbols.insert("ETH");
    }
    if !csm.is_empty() {
        symbols.insert("stETH");
    }
    for row in native {
        for tok in &row.tokens {
            symbols.insert(tok.display_symbol.as_str());
        }
    }
    for row in splits {
        if row.token.is_none() {
            symbols.insert("ETH");
        } else {
            symbols.insert(row.display_symbol.as_str());
        }
    }
    if symbols.is_empty() {
        return PriceTable::default();
    }
    let client = match PriceClient::new() {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "price client init failed; USD totals will be omitted: {:#}",
                e
            );
            return PriceTable::default();
        }
    };
    let owned: Vec<&str> = symbols.iter().copied().collect();
    match client.fetch_for_symbols(&owned).await {
        Ok(t) => t,
        Err(e) => {
            warn!("price fetch failed; USD totals will be omitted: {:#}", e);
            PriceTable::default()
        }
    }
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
            let balance_wei = match client.balance(*addr).await {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        alias,
                        chain = chain.name(),
                        "native balance fetch failed; omitting from table: {:#}",
                        e
                    );
                    continue;
                }
            };

            let mut tokens = Vec::new();
            for deployment in tokens::deployments_for(chain.id(), &cfg.token_whitelist) {
                match client.erc20_balance(deployment.address, *addr).await {
                    Ok(amount) if !amount.is_zero() => tokens.push(TokenBalance {
                        display_symbol: deployment.display_symbol.to_string(),
                        decimals: deployment.decimals,
                        amount,
                    }),
                    Ok(_) => {}
                    Err(e) => warn!(
                        alias,
                        chain = chain.name(),
                        token = deployment.display_symbol,
                        "ERC-20 balance fetch failed: {:#}",
                        e
                    ),
                }
            }

            rows.push(NativeRow {
                alias: alias.clone(),
                address: *addr,
                chain: *chain,
                balance_wei,
                tokens,
            });
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

async fn collect_splits(cfg: &Config) -> Result<Vec<SplitsRow>> {
    let mut rows = Vec::new();
    for (chain, cc) in &cfg.chains {
        let Some(reader) = SplitsReader::connect(chain.id(), &cc.rpc_url)? else {
            continue; // warehouse not deployed on this chain
        };

        for (alias, addr) in &cfg.addresses {
            // Native ETH claim.
            match reader.balance(*addr, None).await {
                Ok(amount) if !amount.is_zero() => rows.push(SplitsRow {
                    alias: alias.clone(),
                    address: *addr,
                    chain: *chain,
                    token: None,
                    display_symbol: "ETH".into(),
                    decimals: 18,
                    amount,
                }),
                Ok(_) => {}
                Err(e) => warn!(
                    alias,
                    chain = chain.name(),
                    "splits native claim fetch failed; skipping: {:#}",
                    e
                ),
            }

            // Whitelisted ERC-20 claims.
            for deployment in tokens::deployments_for(chain.id(), &cfg.token_whitelist) {
                match reader.balance(*addr, Some(deployment.address)).await {
                    Ok(amount) if !amount.is_zero() => rows.push(SplitsRow {
                        alias: alias.clone(),
                        address: *addr,
                        chain: *chain,
                        token: Some(deployment.address),
                        display_symbol: deployment.display_symbol.to_string(),
                        decimals: deployment.decimals,
                        amount,
                    }),
                    Ok(_) => {}
                    Err(e) => warn!(
                        alias,
                        chain = chain.name(),
                        token = deployment.display_symbol,
                        "splits ERC-20 claim fetch failed; skipping: {:#}",
                        e
                    ),
                }
            }
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grand_total_usd_sums_native_staking_and_csm() {
        let mut prices = PriceTable::default();
        // Inject prices for the test by going through fetch_for_symbols mocking;
        // simpler here: pre-populate via a small test-only constructor.
        prices.insert_for_test("ETH", 4_000.0);
        prices.insert_for_test("stETH", 4_010.0);

        let snap = PortfolioSnapshot {
            native: vec![NativeRow {
                alias: "a".into(),
                address: Address::default(),
                chain: Chain::Mainnet,
                balance_wei: U256::from(10u64).pow(U256::from(18)), // 1 ETH
                tokens: Vec::new(),
            }],
            staking: vec![StakingRow {
                alias: "a".into(),
                validator_count: 1,
                total_balance_gwei: 32_000_000_000, // 32 ETH
                from_cache: false,
            }],
            csm: vec![CsmRow {
                operator_id: 1,
                bond_steth_wei: U256::from(10u64).pow(U256::from(18)) * U256::from(2u64), // 2 stETH
            }],
            splits: Vec::new(),
            prices,
        };
        let usd = snap.grand_total_usd().unwrap();
        // 1 ETH * $4000 + 32 ETH * $4000 + 2 stETH * $4010 = $4000 + $128000 + $8020 = $140020
        assert!((usd - 140_020.0).abs() < 1e-6);
    }

    #[test]
    fn grand_total_usd_includes_splits_claims() {
        let mut prices = PriceTable::default();
        prices.insert_for_test("ETH", 4_000.0);
        prices.insert_for_test("USDC", 1.0);

        let snap = PortfolioSnapshot {
            native: Vec::new(),
            staking: Vec::new(),
            csm: Vec::new(),
            splits: vec![
                SplitsRow {
                    alias: "a".into(),
                    address: Address::default(),
                    chain: Chain::Mainnet,
                    token: None,
                    display_symbol: "ETH".into(),
                    decimals: 18,
                    amount: U256::from(10u64).pow(U256::from(18)) / U256::from(2u64), // 0.5 ETH
                },
                SplitsRow {
                    alias: "a".into(),
                    address: Address::default(),
                    chain: Chain::Mainnet,
                    token: Some(Address::default()),
                    display_symbol: "USDC".into(),
                    decimals: 6,
                    amount: U256::from(100_000_000u64), // 100 USDC
                },
            ],
            prices,
        };
        let usd = snap.grand_total_usd().unwrap();
        // 0.5 * $4000 + 100 * $1 = $2100
        assert!((usd - 2_100.0).abs() < 1e-6);
    }
}
