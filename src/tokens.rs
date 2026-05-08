use alloy::primitives::{Address, address};

/// One ERC-20 deployment we may want to read balances from.
///
/// `whitelist_id` is the logical name the user puts in
/// `LPORTFOLIO_TOKEN_WHITELIST`; `display_symbol` is what we render in tables
/// (some tokens have different on-chain symbols across chains, e.g. bridged
/// USDT on Arbitrum is `USDT0`).
#[derive(Debug, Clone, Copy)]
pub struct WhitelistedToken {
    pub whitelist_id: &'static str,
    pub chain_id: u64,
    pub address: Address,
    pub display_symbol: &'static str,
    pub decimals: u8,
}

pub const REGISTRY: &[WhitelistedToken] = &[
    // -- Mainnet --
    WhitelistedToken {
        whitelist_id: "USDC",
        chain_id: 1,
        address: address!("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"),
        display_symbol: "USDC",
        decimals: 6,
    },
    WhitelistedToken {
        whitelist_id: "USDT",
        chain_id: 1,
        address: address!("0xdAC17F958D2ee523a2206206994597C13D831ec7"),
        display_symbol: "USDT",
        decimals: 6,
    },
    WhitelistedToken {
        whitelist_id: "ARB",
        chain_id: 1,
        address: address!("0xB50721BcF8d664c30412Cfbc6cF7a15145234ad1"),
        display_symbol: "ARB",
        decimals: 18,
    },
    WhitelistedToken {
        whitelist_id: "DAI",
        chain_id: 1,
        address: address!("0x6B175474E89094C44Da98b954EedeAC495271d0F"),
        display_symbol: "DAI",
        decimals: 18,
    },
    // -- Arbitrum One --
    WhitelistedToken {
        whitelist_id: "USDC",
        chain_id: 42161,
        address: address!("0xaf88d065e77c8cC2239327C5EDb3A432268e5831"),
        display_symbol: "USDC",
        decimals: 6,
    },
    WhitelistedToken {
        whitelist_id: "USDC",
        chain_id: 42161,
        address: address!("0xFF970A61A04b1cA14834A43f5dE4533eBDDB5CC8"),
        display_symbol: "USDC.e",
        decimals: 6,
    },
    WhitelistedToken {
        whitelist_id: "USDT",
        chain_id: 42161,
        address: address!("0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9"),
        display_symbol: "USDT0",
        decimals: 6,
    },
    WhitelistedToken {
        whitelist_id: "ARB",
        chain_id: 42161,
        address: address!("0x912CE59144191C1204E64559FE8253a0e49E6548"),
        display_symbol: "ARB",
        decimals: 18,
    },
    WhitelistedToken {
        whitelist_id: "DAI",
        chain_id: 42161,
        address: address!("0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1"),
        display_symbol: "DAI",
        decimals: 18,
    },
];

/// Returns the deployments matching `(chain_id, whitelist_id ∈ whitelist)`.
pub fn deployments_for(chain_id: u64, whitelist: &[String]) -> Vec<&'static WhitelistedToken> {
    REGISTRY
        .iter()
        .filter(|t| t.chain_id == chain_id && whitelist.iter().any(|w| w == t.whitelist_id))
        .collect()
}
