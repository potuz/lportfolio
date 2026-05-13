use std::time::Duration;

use alloy::primitives::{Address, Bytes, address};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::sol;
use anyhow::{Context, Result};
use tracing::{info, warn};

use crate::chain::Chain;
use crate::config::Config;
use crate::db::Db;

/// L2-aware Universal Resolver deployment on Ethereum mainnet. Its
/// `reverse(bytes)` method performs forward-verification internally — the
/// returned name is empty when no record exists or when forward
/// verification fails.
const UNIVERSAL_RESOLVER: Address = address!("0xce01f8eee7E479C928F8919abD53E553a36CeF67");

const MAX_RETRIES: u32 = 2;
const INITIAL_BACKOFF: Duration = Duration::from_millis(400);
const PER_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const CONCURRENCY: usize = 4;

sol! {
    #[sol(rpc)]
    interface IUniversalResolver {
        /// `reverseName` is DNS-wire-encoded `<addr-no-0x-lowercase>.addr.reverse`.
        function reverse(bytes calldata reverseName)
            external
            view
            returns (
                string memory name,
                address resolvedAddress,
                address reverseResolver,
                address resolver
            );
    }
}

#[derive(Clone)]
pub struct EnsResolver {
    provider: DynProvider,
}

#[derive(Debug, Default)]
pub struct EnsBatchStats {
    pub resolved: usize,
    pub miss: usize,
    pub errored: usize,
}

impl EnsResolver {
    pub fn connect(mainnet_rpc_url: &str) -> Result<Self> {
        let url = mainnet_rpc_url
            .parse()
            .with_context(|| format!("invalid mainnet RPC URL: {mainnet_rpc_url}"))?;
        let provider = ProviderBuilder::new().connect_http(url).erased();
        Ok(Self { provider })
    }

    /// Reverse-resolve an address via the Universal Resolver.
    ///
    /// `Ok(None)` is a confirmed miss (no reverse record, forward
    /// verification failed, or any deterministic contract revert) and
    /// should be cached. `Err` indicates a transient transport/timeout
    /// failure and must NOT be cached.
    pub async fn reverse(&self, addr: Address) -> Result<Option<String>> {
        let mut attempt: u32 = 0;
        let wire = encode_reverse_name(addr);
        let contract = IUniversalResolver::new(UNIVERSAL_RESOLVER, &self.provider);
        loop {
            let builder = contract.reverse(Bytes::from(wire.clone()));
            let call = builder.call();
            match tokio::time::timeout(PER_CALL_TIMEOUT, call).await {
                Ok(Ok(ret)) => {
                    if ret.name.is_empty() {
                        return Ok(None);
                    }
                    return Ok(Some(ret.name));
                }
                Ok(Err(e)) if is_contract_revert(&e) => {
                    // Deterministic — same input will revert the same way next
                    // sync. Treat as a confirmed miss so we cache it and stop
                    // hammering. Covers ResolverNotFound, UnsupportedResolverProfile,
                    // ReverseAddressMismatch, etc.
                    return Ok(None);
                }
                Ok(Err(e)) if attempt < MAX_RETRIES => {
                    attempt += 1;
                    let backoff = INITIAL_BACKOFF * (1u32 << attempt);
                    warn!(
                        addr = %format!("{addr:#x}"),
                        attempt,
                        backoff_ms = backoff.as_millis() as u64,
                        error = %e,
                        "ENS reverse error; retrying",
                    );
                    tokio::time::sleep(backoff).await;
                }
                Ok(Err(e)) => {
                    return Err(e).with_context(|| format!("UR.reverse({addr:#x})"));
                }
                Err(_elapsed) if attempt < MAX_RETRIES => {
                    attempt += 1;
                    let backoff = INITIAL_BACKOFF * (1u32 << attempt);
                    warn!(
                        addr = %format!("{addr:#x}"),
                        attempt,
                        backoff_ms = backoff.as_millis() as u64,
                        "ENS reverse timeout; retrying",
                    );
                    tokio::time::sleep(backoff).await;
                }
                Err(_elapsed) => {
                    anyhow::bail!("ENS reverse timeout for {addr:#x}");
                }
            }
        }
    }
}

/// Detect contract-level reverts (deterministic) vs. transport errors
/// (transient). The Universal Resolver returns several custom errors
/// (ResolverNotFound, UnsupportedResolverProfile, ReverseAddressMismatch,
/// …) for addresses with no usable reverse record — these manifest as
/// `execution reverted` from the RPC. Heuristic match on the message
/// because alloy::contract::Error variants are version-sensitive.
fn is_contract_revert(e: &alloy::contract::Error) -> bool {
    let s = format!("{e}");
    s.contains("execution reverted")
}

/// DNS-wire encoding of `<addr-no-0x-lowercase>.addr.reverse`.
///
/// Layout (55 bytes total): `[40][<40 hex bytes>][4]"addr"[7]"reverse"[0]`.
fn encode_reverse_name(addr: Address) -> Vec<u8> {
    let hex = format!("{addr:x}"); // lowercase, no 0x, 40 chars
    let mut out = Vec::with_capacity(1 + 40 + 1 + 4 + 1 + 7 + 1);
    out.push(40);
    out.extend_from_slice(hex.as_bytes());
    out.push(4);
    out.extend_from_slice(b"addr");
    out.push(7);
    out.extend_from_slice(b"reverse");
    out.push(0);
    out
}

/// Best-effort batch ENS resolution against the configured mainnet RPC.
/// Silently skips (returns zero stats) when no mainnet RPC is configured.
/// Per-address transport errors are logged but do not abort the pass.
pub async fn resolve_pending(
    db: &mut Db,
    cfg: &Config,
    owned: &[Address],
) -> Result<EnsBatchStats> {
    let Some(cc) = cfg.chains.get(&Chain::Mainnet) else {
        info!("ENS resolution skipped: no mainnet RPC configured");
        return Ok(EnsBatchStats::default());
    };
    let resolver = EnsResolver::connect(&cc.rpc_url)?;
    let pending = db.ens_pending(owned)?;
    if pending.is_empty() {
        return Ok(EnsBatchStats::default());
    }

    let mut stats = EnsBatchStats::default();
    for chunk in pending.chunks(CONCURRENCY) {
        let mut set = tokio::task::JoinSet::new();
        for &addr in chunk {
            let r = resolver.clone();
            set.spawn(async move { (addr, r.reverse(addr).await) });
        }
        while let Some(joined) = set.join_next().await {
            let (addr, outcome) = joined.context("ENS task panicked")?;
            match outcome {
                Ok(Some(name)) => {
                    db.ens_upsert(addr, Some(&name))?;
                    stats.resolved += 1;
                }
                Ok(None) => {
                    db.ens_upsert(addr, None)?;
                    stats.miss += 1;
                }
                Err(e) => {
                    warn!(
                        addr = %format!("{addr:#x}"),
                        err = %format!("{e:#}"),
                        "ENS reverse failed",
                    );
                    stats.errored += 1;
                }
            }
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn universal_resolver_constant_decodes() {
        assert_eq!(UNIVERSAL_RESOLVER.0.0.len(), 20);
    }

    #[test]
    fn encode_reverse_name_for_known_address() {
        // vitalik.eth's address.
        let addr: Address = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let wire = encode_reverse_name(addr);

        assert_eq!(wire.len(), 1 + 40 + 1 + 4 + 1 + 7 + 1);
        assert_eq!(wire[0], 40);
        assert_eq!(&wire[1..41], b"d8da6bf26964af9d7eed9e03e53415d37aa96045");
        assert_eq!(wire[41], 4);
        assert_eq!(&wire[42..46], b"addr");
        assert_eq!(wire[46], 7);
        assert_eq!(&wire[47..54], b"reverse");
        assert_eq!(wire[54], 0);
    }

    #[test]
    fn encode_reverse_name_lowercases() {
        let mixed: Address = "0xD8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
            .parse()
            .unwrap();
        let wire = encode_reverse_name(mixed);
        assert!(wire[1..41].iter().all(|b| !b.is_ascii_uppercase()));
    }
}
