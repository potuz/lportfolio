use alloy::primitives::{Address, address};

use super::erc20::{RawTransfer, RawTx};
use super::{
    Action, ContractDecoder, KnownContract, ProtocolKind, inbound_assets, native_received,
    outbound_assets,
};

const SETTLEMENT: Address = address!("0x9008D19f58AAbD9eD0D60971565AA8510560ab41");
const VAULT_RELAYER: Address = address!("0xC92E8bdf79f0507f65a392b0ab4667716BFE0110");

const KNOWN: &[KnownContract] = &[
    KnownContract {
        chain_id: 1,
        address: SETTLEMENT,
        label: "CoW Protocol Settlement",
    },
    KnownContract {
        chain_id: 1,
        address: VAULT_RELAYER,
        label: "CoW Protocol Vault Relayer",
    },
];

pub struct Cowswap;

impl ContractDecoder for Cowswap {
    fn known_contracts(&self) -> &[KnownContract] {
        KNOWN
    }

    fn decode(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> Option<Vec<Action>> {
        if tx.chain_id != 1 {
            return None;
        }

        // CoW orders are settled in batches: the user is rarely tx.from. Detect by
        // whether any of the user's transfers in this tx have settlement/relayer as
        // counterparty, or whether the Settlement contract sent the user ETH via
        // an internal call (the ETH-out side of a swap).
        let touches_via_transfer = transfers.iter().any(|t| {
            (t.from == us && (t.to == SETTLEMENT || t.to == VAULT_RELAYER))
                || (t.to == us && (t.from == SETTLEMENT || t.from == VAULT_RELAYER))
        });
        let touches_via_internal = tx.internals.iter().any(|it| {
            it.success && it.to == us && (it.from == SETTLEMENT || it.from == VAULT_RELAYER)
        });
        if !touches_via_transfer && !touches_via_internal {
            return None;
        }

        let sent = outbound_assets(us, transfers);
        let mut received = inbound_assets(us, transfers);
        if let Some(eth) = native_received(us, tx) {
            received.insert(0, eth);
        }
        if sent.is_empty() && received.is_empty() {
            return None;
        }

        Some(vec![Action::Protocol {
            protocol: "CoW Protocol",
            kind: ProtocolKind::Swap,
            contract: SETTLEMENT,
            sent,
            received,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::U256;

    fn us() -> Address {
        address!("0x000000000000000000000000000000000000beef")
    }

    #[test]
    fn detects_swap_when_settlement_is_counterparty() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x1".into(),
            timestamp: 1,
            from: address!("0x000000000000000000000000000000000000ee11"),
            to: Some(SETTLEMENT),
            value_wei: U256::ZERO,
            input_len: 200,
            success: true,
            internals: Vec::new(),
        };
        let send = RawTransfer {
            token: address!("0x000000000000000000000000000000000000aaaa"),
            from: us(),
            to: VAULT_RELAYER,
            amount: U256::from(1_000_000u64),
            symbol: "USDC".into(),
            decimals: 6,
        };
        let recv = RawTransfer {
            token: address!("0x000000000000000000000000000000000000bbbb"),
            from: SETTLEMENT,
            to: us(),
            amount: U256::from(1u64),
            symbol: "DAI".into(),
            decimals: 18,
        };
        let actions = Cowswap.decode(us(), &tx, &[send, recv]).expect("decode");
        assert!(matches!(
            &actions[..],
            [Action::Protocol {
                protocol: "CoW Protocol",
                kind: ProtocolKind::Swap,
                ..
            }]
        ));
    }
}
