use alloy::primitives::{Address, address};

use super::erc20::{RawTransfer, RawTx};
use super::{
    Action, ContractDecoder, KnownContract, ProtocolKind, inbound_assets, native_sent,
    outbound_assets,
};

// Across V3 spoke pools (a small representative set; one per chain we support).
const SPOKE_MAINNET: Address = address!("0x5c7BCd6E7De5423a257D81B442095A1a6ced35C5");
const SPOKE_ARBITRUM: Address = address!("0xe35e9842fceaCA96570B734083f4a58e8F7C5f2A");
const SPOKE_OPTIMISM: Address = address!("0x6f26Bf09B1C792e3228e5467807a900A503c0281");
const SPOKE_BASE: Address = address!("0x09aea4b2242abC8bb4BB78D537A67a245A7bEC64");

const KNOWN: &[KnownContract] = &[
    KnownContract {
        chain_id: 1,
        address: SPOKE_MAINNET,
        label: "Across Spoke Pool",
    },
    KnownContract {
        chain_id: 42161,
        address: SPOKE_ARBITRUM,
        label: "Across Spoke Pool",
    },
    KnownContract {
        chain_id: 10,
        address: SPOKE_OPTIMISM,
        label: "Across Spoke Pool",
    },
    KnownContract {
        chain_id: 8453,
        address: SPOKE_BASE,
        label: "Across Spoke Pool",
    },
];

pub struct Across;

impl Across {
    fn spoke_for(chain_id: u64) -> Option<Address> {
        match chain_id {
            1 => Some(SPOKE_MAINNET),
            42161 => Some(SPOKE_ARBITRUM),
            10 => Some(SPOKE_OPTIMISM),
            8453 => Some(SPOKE_BASE),
            _ => None,
        }
    }
}

impl ContractDecoder for Across {
    fn known_contracts(&self) -> &[KnownContract] {
        KNOWN
    }

    fn decode(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> Option<Vec<Action>> {
        let spoke = Self::spoke_for(tx.chain_id)?;
        let touches_spoke = tx.to == Some(spoke)
            || transfers
                .iter()
                .any(|t| (t.from == us && t.to == spoke) || (t.to == us && t.from == spoke));
        if !touches_spoke {
            return None;
        }

        let mut sent = outbound_assets(us, transfers);
        if let Some(eth) = native_sent(us, tx) {
            sent.insert(0, eth);
        }
        let received = inbound_assets(us, transfers);
        if sent.is_empty() && received.is_empty() {
            return None;
        }

        Some(vec![Action::Protocol {
            protocol: "Across",
            kind: ProtocolKind::Bridge,
            contract: spoke,
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
    fn detects_deposit_to_spoke() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x1".into(),
            timestamp: 1,
            from: us(),
            to: Some(SPOKE_MAINNET),
            value_wei: U256::ZERO,
            input_len: 200,
            success: true,
        };
        let send = RawTransfer {
            token: address!("0x000000000000000000000000000000000000beed"),
            from: us(),
            to: SPOKE_MAINNET,
            amount: U256::from(1_000_000_000u64),
            symbol: "USDC".into(),
            decimals: 6,
        };
        let actions = Across.decode(us(), &tx, &[send]).expect("decode");
        assert!(matches!(
            &actions[..],
            [Action::Protocol {
                protocol: "Across",
                kind: ProtocolKind::Bridge,
                ..
            }]
        ));
    }
}
