use alloy::primitives::{Address, address};

use super::erc20::{RawTransfer, RawTx};
use super::{
    Action, ContractDecoder, KnownContract, ProtocolKind, inbound_assets, outbound_assets,
};

const SPLIT_MAIN_V1: Address = address!("0x2ed6c4B5dA6378c7897AC67Ba9e43102Feb694EE");
const SPLITS_WAREHOUSE: Address = address!("0x8fb66F38cF86A3d5e8768f8F1754A24A6c661Fb8");

const KNOWN: &[KnownContract] = &[
    KnownContract {
        chain_id: 1,
        address: SPLIT_MAIN_V1,
        label: "0xSplits SplitMain",
    },
    KnownContract {
        chain_id: 1,
        address: SPLITS_WAREHOUSE,
        label: "0xSplits Warehouse",
    },
];

pub struct Splits;

impl ContractDecoder for Splits {
    fn known_contracts(&self) -> &[KnownContract] {
        KNOWN
    }

    fn decode(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> Option<Vec<Action>> {
        if tx.chain_id != 1 {
            return None;
        }
        let touches = tx.to == Some(SPLIT_MAIN_V1)
            || tx.to == Some(SPLITS_WAREHOUSE)
            || transfers.iter().any(|t| {
                let touches_split = t.from == SPLIT_MAIN_V1
                    || t.to == SPLIT_MAIN_V1
                    || t.from == SPLITS_WAREHOUSE
                    || t.to == SPLITS_WAREHOUSE;
                touches_split && (t.from == us || t.to == us)
            });
        if !touches {
            return None;
        }

        let sent = outbound_assets(us, transfers);
        let received = inbound_assets(us, transfers);
        if sent.is_empty() && received.is_empty() {
            return None;
        }

        let kind = if !received.is_empty() && sent.is_empty() {
            ProtocolKind::Distribute
        } else {
            ProtocolKind::Other
        };
        let contract = if tx.to == Some(SPLITS_WAREHOUSE) {
            SPLITS_WAREHOUSE
        } else {
            SPLIT_MAIN_V1
        };

        Some(vec![Action::Protocol {
            protocol: "0xSplits",
            kind,
            contract,
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
    fn distribute_when_user_only_receives_from_split() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x1".into(),
            timestamp: 1,
            from: address!("0x000000000000000000000000000000000000ee11"),
            to: Some(SPLIT_MAIN_V1),
            value_wei: U256::ZERO,
            input_len: 100,
            success: true,
        };
        let recv = RawTransfer {
            token: address!("0x000000000000000000000000000000000000beed"),
            from: SPLIT_MAIN_V1,
            to: us(),
            amount: U256::from(1_000_000_000u64),
            symbol: "USDC".into(),
            decimals: 6,
        };
        let actions = Splits.decode(us(), &tx, &[recv]).expect("decode");
        assert!(matches!(
            &actions[..],
            [Action::Protocol {
                protocol: "0xSplits",
                kind: ProtocolKind::Distribute,
                ..
            }]
        ));
    }
}
