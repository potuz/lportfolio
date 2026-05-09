use alloy::primitives::{Address, U256, address};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::sol;
use anyhow::{Context, Result};

/// SplitsWarehouse singleton (Splits V2). Deployed deterministically at the
/// same address on mainnet, arbitrum, optimism, and base.
const SPLITS_WAREHOUSE: Address = address!("0x8fb66F38cF86A3d5e8768f8F1754A24A6c661Fb8");

/// Splits V2 native-token sentinel — `0xeeee…eeee` per the Splits docs.
const NATIVE_TOKEN_SENTINEL: Address = address!("0xeeeeeEeEeEEEeeEEeEeeeeeeeeeeeeeeeeeeEEEE");

sol! {
    #[sol(rpc)]
    interface ISplitsWarehouse {
        /// ERC-6909 balance: per-recipient claimable amount of `id`.
        function balanceOf(address owner, uint256 id) external view returns (uint256);
    }
}

pub struct SplitsReader {
    provider: DynProvider,
    chain_id: u64,
}

impl SplitsReader {
    /// Returns `None` for chains where the warehouse isn't deployed.
    pub fn connect(chain_id: u64, rpc_url: &str) -> Result<Option<Self>> {
        if !is_supported_chain(chain_id) {
            return Ok(None);
        }
        let url = rpc_url
            .parse()
            .with_context(|| format!("invalid RPC URL for chain {chain_id}: {rpc_url}"))?;
        let provider = ProviderBuilder::new().connect_http(url).erased();
        Ok(Some(Self { provider, chain_id }))
    }

    /// `token = None` reads the user's claimable native ETH balance; `Some(addr)`
    /// reads the claimable balance of that ERC-20.
    pub async fn balance(&self, recipient: Address, token: Option<Address>) -> Result<U256> {
        let token_addr = token.unwrap_or(NATIVE_TOKEN_SENTINEL);
        let id = token_id_for(token_addr);
        let contract = ISplitsWarehouse::new(SPLITS_WAREHOUSE, &self.provider);
        contract
            .balanceOf(recipient, id)
            .call()
            .await
            .with_context(|| {
                format!(
                    "SplitsWarehouse.balanceOf({recipient:#x}, {token_addr:#x}) on chain {}",
                    self.chain_id
                )
            })
    }
}

/// ERC-6909 token id derivation: `uint256(uint160(address))`. `from_be_slice`
/// of a 20-byte address right-aligns into a U256, leaving the upper 96 bits 0.
fn token_id_for(token: Address) -> U256 {
    U256::from_be_slice(token.as_slice())
}

fn is_supported_chain(chain_id: u64) -> bool {
    matches!(chain_id, 1 | 10 | 8453 | 42161)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warehouse_address_constant_decodes() {
        assert_eq!(SPLITS_WAREHOUSE.0.0.len(), 20);
        assert_eq!(NATIVE_TOKEN_SENTINEL.0.0.len(), 20);
    }

    #[test]
    fn token_id_lower_160_bits_match_address() {
        let addr = address!("0x1234567890aBcDeF1234567890ABCDEF12345678");
        let id = token_id_for(addr);
        // Round-trip: lower 20 bytes of the U256 should equal the address.
        let bytes: [u8; 32] = id.to_be_bytes();
        assert_eq!(&bytes[..12], &[0u8; 12]);
        assert_eq!(&bytes[12..], addr.as_slice());
    }

    #[test]
    fn supported_chains_match_known_deployments() {
        assert!(is_supported_chain(1));
        assert!(is_supported_chain(10));
        assert!(is_supported_chain(8453));
        assert!(is_supported_chain(42161));
        assert!(!is_supported_chain(5)); // goerli — not supported
    }
}
