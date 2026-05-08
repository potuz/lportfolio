pub mod erc20;

use alloy::primitives::{Address, U256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    SelfTransfer,
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
