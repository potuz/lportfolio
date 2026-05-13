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
    // -- Mainnet-only governance / yield tokens (often distributed via splits) --
    WhitelistedToken {
        whitelist_id: "EIGEN",
        chain_id: 1,
        address: address!("0xec53bF9167f50cDEB3Ae105f56099aaaB9061F83"),
        display_symbol: "EIGEN",
        decimals: 18,
    },
    WhitelistedToken {
        whitelist_id: "PUFFER",
        chain_id: 1,
        address: address!("0x4d1C297d39C5c1277964D0E3f8Aa901493664530"),
        display_symbol: "PUFFER",
        decimals: 18,
    },
    WhitelistedToken {
        whitelist_id: "SAFE",
        chain_id: 1,
        address: address!("0x5aFE3855358E112B5647B952709E6165e1c1eEEe"),
        display_symbol: "SAFE",
        decimals: 18,
    },
    WhitelistedToken {
        whitelist_id: "ETHFI",
        chain_id: 1,
        address: address!("0xFe0c30065B384F05761f15d0CC899D4F9F9Cc0eB"),
        display_symbol: "ETHFI",
        decimals: 18,
    },
    WhitelistedToken {
        whitelist_id: "STRK",
        chain_id: 1,
        address: address!("0xCa14007Eff0dB1f8135f4C25B34De49AB0d42766"),
        display_symbol: "STRK",
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

/// Yields `(chain_id, contract_address, display_symbol)` for every
/// `WhitelistedToken` whose `whitelist_id` appears in `whitelist`. Used by
/// the history view to auto-label token contracts the user already cares
/// about (since the whitelist already implies "render this as a known
/// thing").
pub fn whitelist_labels<'a>(
    whitelist: &'a [String],
) -> impl Iterator<Item = (u64, Address, &'static str)> + 'a {
    REGISTRY.iter().filter_map(move |t| {
        if whitelist.iter().any(|w| w == t.whitelist_id) {
            Some((t.chain_id, t.address, t.display_symbol))
        } else {
            None
        }
    })
}

/// Returns the canonical display symbol for a token if (and only if) the
/// `(chain_id, address)` pair is in our hardcoded REGISTRY. Otherwise
/// returns `None` — the on-chain `symbol()` is untrusted and may be a
/// scammer impersonating a well-known token.
pub fn trusted_symbol_for(chain_id: u64, addr: Address) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|t| t.chain_id == chain_id && t.address == addr)
        .map(|t| t.display_symbol)
}
