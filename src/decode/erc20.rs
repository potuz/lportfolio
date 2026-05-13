use alloy::primitives::{Address, U256};

use super::{Action, DecodedTx, Direction, classify_direction};

#[derive(Debug, Clone)]
pub struct RawTx {
    pub chain_id: u64,
    pub hash: String,
    pub timestamp: u64,
    pub from: Address,
    pub to: Option<Address>,
    pub value_wei: U256,
    pub input_len: usize,
    pub success: bool,
    /// Internal calls (CALL/CALLCODE/CREATE/SELFDESTRUCT) made during this
    /// tx's execution. Only entries with `value_wei > 0` are interesting
    /// for native-ETH transfer detection.
    pub internals: Vec<InternalTx>,
}

#[derive(Debug, Clone)]
pub struct InternalTx {
    pub from: Address,
    pub to: Address,
    pub value_wei: U256,
    pub success: bool,
}

#[derive(Debug, Clone)]
pub struct RawTransfer {
    pub token: Address,
    pub from: Address,
    pub to: Address,
    pub amount: U256,
    pub symbol: String,
    pub decimals: u32,
}

pub fn synthesize(us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> DecodedTx {
    let mut actions = Vec::new();

    if let Some(to) = tx.to
        && !tx.value_wei.is_zero()
        && let Some(direction) = classify_direction(us, tx.from, to)
    {
        let counterparty = match direction {
            Direction::Out | Direction::SelfTransfer => to,
            Direction::In => tx.from,
        };
        actions.push(Action::NativeTransfer {
            direction,
            counterparty,
            amount_wei: tx.value_wei,
        });
    }

    for t in transfers {
        if let Some(direction) = classify_direction(us, t.from, t.to) {
            let counterparty = match direction {
                Direction::Out | Direction::SelfTransfer => t.to,
                Direction::In => t.from,
            };
            actions.push(Action::TokenTransfer {
                direction,
                counterparty,
                token: t.token,
                symbol: t.symbol.clone(),
                decimals: t.decimals,
                amount: t.amount,
            });
        }
    }

    if actions.is_empty()
        && let Some(to) = tx.to
        && tx.from == us
        && tx.input_len > 0
    {
        actions.push(Action::ContractCall { contract: to });
    }

    DecodedTx {
        chain_id: tx.chain_id,
        hash: tx.hash.clone(),
        timestamp: tx.timestamp,
        success: tx.success,
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn us() -> Address {
        address!("0x0000000000000000000000000000000000000001")
    }

    fn other() -> Address {
        address!("0x0000000000000000000000000000000000000002")
    }

    fn tx(value_wei: U256, to: Option<Address>, from: Address, input_len: usize) -> RawTx {
        RawTx {
            chain_id: 1,
            hash: "0xhash".into(),
            timestamp: 1_700_000_000,
            from,
            to,
            value_wei,
            input_len,
            success: true,
            internals: Vec::new(),
        }
    }

    #[test]
    fn outgoing_native_transfer_creates_one_action() {
        let value = U256::from(10u64).pow(U256::from(18));
        let dec = synthesize(us(), &tx(value, Some(other()), us(), 0), &[]);
        assert_eq!(dec.actions.len(), 1);
        match &dec.actions[0] {
            Action::NativeTransfer {
                direction: Direction::Out,
                counterparty,
                amount_wei,
            } => {
                assert_eq!(*counterparty, other());
                assert_eq!(*amount_wei, value);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn unrelated_tx_yields_no_actions() {
        let value = U256::from(1u64);
        let third = address!("0x0000000000000000000000000000000000000003");
        let dec = synthesize(us(), &tx(value, Some(other()), third, 0), &[]);
        assert!(dec.actions.is_empty());
    }

    #[test]
    fn contract_call_with_no_value_emits_contract_call() {
        let dec = synthesize(us(), &tx(U256::ZERO, Some(other()), us(), 64), &[]);
        assert!(matches!(
            dec.actions.as_slice(),
            [Action::ContractCall { .. }]
        ));
    }

    #[test]
    fn token_transfer_adds_action() {
        let raw = tx(U256::ZERO, Some(other()), us(), 64);
        let xfer = RawTransfer {
            token: address!("0x000000000000000000000000000000000000abcd"),
            from: us(),
            to: other(),
            amount: U256::from(1_000_000u64),
            symbol: "USDC".into(),
            decimals: 6,
        };
        let dec = synthesize(us(), &raw, std::slice::from_ref(&xfer));
        assert_eq!(dec.actions.len(), 1);
        assert!(matches!(
            &dec.actions[0],
            Action::TokenTransfer {
                direction: Direction::Out,
                ..
            }
        ));
    }
}
