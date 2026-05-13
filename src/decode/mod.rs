pub mod aave;
pub mod across;
pub mod cowswap;
pub mod erc20;
pub mod lido;
pub mod splits;
pub mod uniswap;

use alloy::primitives::{Address, U256};

use crate::decode::erc20::{RawTransfer, RawTx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    SelfTransfer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolKind {
    Stake,
    Unstake,
    Wrap,
    Unwrap,
    Swap,
    Supply,
    Withdraw,
    Borrow,
    Repay,
    Bridge,
    Distribute,
    Claim,
    Other,
}

impl ProtocolKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stake => "Stake",
            Self::Unstake => "Unstake",
            Self::Wrap => "Wrap",
            Self::Unwrap => "Unwrap",
            Self::Swap => "Swap",
            Self::Supply => "Supply",
            Self::Withdraw => "Withdraw",
            Self::Borrow => "Borrow",
            Self::Repay => "Repay",
            Self::Bridge => "Bridge",
            Self::Distribute => "Distribute",
            Self::Claim => "Claim",
            Self::Other => "Interaction",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AssetAmount {
    /// `None` means native ETH; `Some` means an ERC-20 contract.
    pub token: Option<Address>,
    pub symbol: String,
    pub decimals: u32,
    pub amount: U256,
}

#[derive(Debug, Clone)]
pub enum Action {
    NativeTransfer {
        direction: Direction,
        counterparty: Address,
        amount_wei: U256,
    },
    TokenTransfer {
        direction: Direction,
        counterparty: Address,
        token: Address,
        symbol: String,
        decimals: u32,
        amount: U256,
    },
    ContractCall {
        contract: Address,
    },
    Protocol {
        protocol: &'static str,
        kind: ProtocolKind,
        contract: Address,
        sent: Vec<AssetAmount>,
        received: Vec<AssetAmount>,
    },
}

#[derive(Debug, Clone)]
pub struct DecodedTx {
    pub chain_id: u64,
    pub hash: String,
    pub timestamp: u64,
    pub success: bool,
    pub actions: Vec<Action>,
}

pub fn classify_direction(us: Address, from: Address, to: Address) -> Option<Direction> {
    let from_us = from == us;
    let to_us = to == us;
    match (from_us, to_us) {
        (true, true) => Some(Direction::SelfTransfer),
        (true, false) => Some(Direction::Out),
        (false, true) => Some(Direction::In),
        (false, false) => None,
    }
}

/// Static metadata: a contract address with a human label, scoped by chain.
#[derive(Debug, Clone, Copy)]
pub struct KnownContract {
    pub chain_id: u64,
    pub address: Address,
    pub label: &'static str,
}

pub trait ContractDecoder: Send + Sync {
    fn known_contracts(&self) -> &[KnownContract];
    fn decode(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> Option<Vec<Action>>;
}

pub struct Registry {
    decoders: Vec<Box<dyn ContractDecoder>>,
}

impl Registry {
    pub fn default_set() -> Self {
        let mut r = Self {
            decoders: Vec::new(),
        };
        r.register(Box::new(lido::Lido));
        r.register(Box::new(aave::AaveV3));
        r.register(Box::new(uniswap::Uniswap));
        r.register(Box::new(cowswap::Cowswap));
        r.register(Box::new(across::Across));
        r.register(Box::new(splits::Splits));
        r
    }

    pub fn register(&mut self, d: Box<dyn ContractDecoder>) {
        self.decoders.push(d);
    }

    pub fn known_labels(&self) -> Vec<KnownContract> {
        self.decoders
            .iter()
            .flat_map(|d| d.known_contracts().iter().copied())
            .collect()
    }

    pub fn decode_tx(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> DecodedTx {
        for d in &self.decoders {
            if let Some(actions) = d.decode(us, tx, transfers) {
                return DecodedTx {
                    chain_id: tx.chain_id,
                    hash: tx.hash.clone(),
                    timestamp: tx.timestamp,
                    success: tx.success,
                    actions,
                };
            }
        }
        erc20::synthesize(us, tx, transfers)
    }
}

/// Helper: gather inbound (`us` is `to`) transfers as `AssetAmount`s.
pub(crate) fn inbound_assets(us: Address, transfers: &[RawTransfer]) -> Vec<AssetAmount> {
    transfers
        .iter()
        .filter(|t| t.to == us && t.from != us)
        .map(|t| AssetAmount {
            token: Some(t.token),
            symbol: t.symbol.clone(),
            decimals: t.decimals,
            amount: t.amount,
        })
        .collect()
}

/// Helper: gather outbound (`us` is `from`) transfers as `AssetAmount`s.
pub(crate) fn outbound_assets(us: Address, transfers: &[RawTransfer]) -> Vec<AssetAmount> {
    transfers
        .iter()
        .filter(|t| t.from == us && t.to != us)
        .map(|t| AssetAmount {
            token: Some(t.token),
            symbol: t.symbol.clone(),
            decimals: t.decimals,
            amount: t.amount,
        })
        .collect()
}

pub(crate) fn native_sent(us: Address, tx: &RawTx) -> Option<AssetAmount> {
    if tx.from == us && !tx.value_wei.is_zero() {
        Some(AssetAmount {
            token: None,
            symbol: "ETH".into(),
            decimals: 18,
            amount: tx.value_wei,
        })
    } else {
        None
    }
}

/// Sum of ETH received by `us` via successful internal calls inside this
/// tx (e.g. Uniswap's `unwrapWETH9` forwarding native ETH after a swap).
/// Returns `None` if no internal transfer landed at `us`.
pub(crate) fn native_received(us: Address, tx: &RawTx) -> Option<AssetAmount> {
    use alloy::primitives::U256;
    let mut total = U256::ZERO;
    for it in &tx.internals {
        if it.success && it.to == us && it.from != us && !it.value_wei.is_zero() {
            total += it.value_wei;
        }
    }
    if total.is_zero() {
        None
    } else {
        Some(AssetAmount {
            token: None,
            symbol: "ETH".into(),
            decimals: 18,
            amount: total,
        })
    }
}
