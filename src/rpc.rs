use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use anyhow::{Context, Result};

use crate::chain::Chain;

pub struct ChainClient {
    chain: Chain,
    provider: DynProvider,
}

impl ChainClient {
    pub fn connect(chain: Chain, rpc_url: &str) -> Result<Self> {
        let url = rpc_url
            .parse()
            .with_context(|| format!("invalid RPC URL for {}: {rpc_url}", chain.name()))?;
        let provider = ProviderBuilder::new().connect_http(url).erased();
        Ok(Self { chain, provider })
    }

    pub fn chain(&self) -> Chain {
        self.chain
    }

    pub async fn balance(&self, addr: Address) -> Result<U256> {
        self.provider
            .get_balance(addr)
            .await
            .with_context(|| format!("get_balance on {}", self.chain.name()))
    }

    pub async fn verify_chain_id(&self) -> Result<()> {
        let observed = self
            .provider
            .get_chain_id()
            .await
            .with_context(|| format!("get_chain_id on {}", self.chain.name()))?;
        if observed != self.chain.id() {
            anyhow::bail!(
                "RPC for {} returned chain id {observed}, expected {}",
                self.chain.name(),
                self.chain.id(),
            );
        }
        Ok(())
    }
}

pub fn format_eth(wei: U256) -> String {
    let raw = wei.to_string();
    let padded = if raw.len() < 19 {
        format!("{:0>19}", raw)
    } else {
        raw
    };
    let len = padded.len();
    let int_part = &padded[..len - 18];
    let frac6 = &padded[len - 18..len - 12];
    let mut combined = format!("{int_part}.{frac6}");
    while combined.ends_with('0') {
        combined.pop();
    }
    if combined.ends_with('.') {
        combined.pop();
    }
    format!("{combined} ETH")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_eth_zero() {
        assert_eq!(format_eth(U256::ZERO), "0 ETH");
    }

    #[test]
    fn format_eth_one() {
        let one_eth = U256::from(10u64).pow(U256::from(18));
        assert_eq!(format_eth(one_eth), "1 ETH");
    }

    #[test]
    fn format_eth_fractional() {
        let half_eth = U256::from(10u64).pow(U256::from(18)) / U256::from(2u64);
        assert_eq!(format_eth(half_eth), "0.5 ETH");
    }

    #[test]
    fn format_eth_truncates_to_six_decimals() {
        // 1.234567890123 ETH — should display 1.234567 (truncated, not rounded)
        let v = U256::from(1_234_567_890_123_000_000u128);
        assert_eq!(format_eth(v), "1.234567 ETH");
    }

    #[test]
    fn format_eth_below_six_decimals_shows_zero() {
        // 1 wei — below display precision
        assert_eq!(format_eth(U256::from(1u64)), "0 ETH");
    }
}
