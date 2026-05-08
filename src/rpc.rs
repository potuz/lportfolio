use std::time::Duration;

use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::sol;
use anyhow::{Context, Result};
use tracing::warn;

use crate::chain::Chain;

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address) external view returns (uint256);
    }
}

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);

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

    pub async fn balance(&self, addr: Address) -> Result<U256> {
        self.with_retry("get_balance", || async {
            self.provider.get_balance(addr).await
        })
        .await
    }

    pub async fn erc20_balance(&self, token: Address, holder: Address) -> Result<U256> {
        let contract = IERC20::new(token, &self.provider);
        self.with_retry("erc20.balanceOf", || async {
            contract.balanceOf(holder).call().await
        })
        .await
    }

    pub async fn verify_chain_id(&self) -> Result<()> {
        let observed = self
            .with_retry("get_chain_id", || async {
                self.provider.get_chain_id().await
            })
            .await?;
        if observed != self.chain.id() {
            anyhow::bail!(
                "RPC for {} returned chain id {observed}, expected {}",
                self.chain.name(),
                self.chain.id(),
            );
        }
        Ok(())
    }

    async fn with_retry<F, Fut, T, E>(&self, label: &str, mut f: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = std::result::Result<T, E>>,
        E: std::fmt::Display + std::error::Error + Send + Sync + 'static,
    {
        let mut attempt: u32 = 0;
        loop {
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) if attempt < MAX_RETRIES => {
                    attempt += 1;
                    let backoff = INITIAL_BACKOFF * (1u32 << attempt);
                    warn!(
                        chain = self.chain.name(),
                        op = label,
                        attempt,
                        backoff_ms = backoff.as_millis() as u64,
                        error = %e,
                        "RPC error; retrying",
                    );
                    tokio::time::sleep(backoff).await;
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("{label} on {}", self.chain.name()));
                }
            }
        }
    }
}
