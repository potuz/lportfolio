use alloy::primitives::{Address, address};

use super::erc20::{RawTransfer, RawTx};
use super::{
    Action, ContractDecoder, KnownContract, ProtocolKind, inbound_assets, outbound_assets,
};

const POOL_V3: Address = address!("0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2");

const KNOWN: &[KnownContract] = &[KnownContract {
    chain_id: 1,
    address: POOL_V3,
    label: "AAVE v3 Pool",
}];

pub struct AaveV3;

impl ContractDecoder for AaveV3 {
    fn known_contracts(&self) -> &[KnownContract] {
        KNOWN
    }

    fn decode(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> Option<Vec<Action>> {
        if tx.chain_id != 1 || tx.to != Some(POOL_V3) {
            return None;
        }
        let sent = outbound_assets(us, transfers);
        let received = inbound_assets(us, transfers);

        // Heuristic: aToken / debtToken symbols start with `a` or `variableDebt`.
        let recv_atoken = received
            .iter()
            .any(|a| starts_with_a(&a.symbol) && !a.symbol.eq_ignore_ascii_case("ATOM"));
        let recv_debt = received
            .iter()
            .any(|a| a.symbol.starts_with("variableDebt") || a.symbol.starts_with("stableDebt"));
        let sent_atoken = sent.iter().any(|a| starts_with_a(&a.symbol));
        let sent_debt = sent
            .iter()
            .any(|a| a.symbol.starts_with("variableDebt") || a.symbol.starts_with("stableDebt"));

        let kind = if recv_atoken && !sent.is_empty() {
            ProtocolKind::Supply
        } else if sent_atoken && !received.is_empty() {
            ProtocolKind::Withdraw
        } else if recv_debt {
            ProtocolKind::Borrow
        } else if sent_debt {
            ProtocolKind::Repay
        } else {
            ProtocolKind::Other
        };

        Some(vec![Action::Protocol {
            protocol: "AAVE v3",
            kind,
            contract: POOL_V3,
            sent,
            received,
        }])
    }
}

fn starts_with_a(symbol: &str) -> bool {
    symbol.starts_with('a') || symbol.starts_with('A')
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::U256;

    fn us() -> Address {
        address!("0x000000000000000000000000000000000000beef")
    }

    #[test]
    fn supply_when_sent_token_and_received_atoken() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x1".into(),
            timestamp: 1,
            from: us(),
            to: Some(POOL_V3),
            value_wei: U256::ZERO,
            input_len: 100,
            success: true,
            internals: Vec::new(),
        };
        let send_usdc = RawTransfer {
            token: address!("0x000000000000000000000000000000000000beed"),
            from: us(),
            to: POOL_V3,
            amount: U256::from(1_000_000_000u64),
            symbol: "USDC".into(),
            decimals: 6,
        };
        let recv_ausdc = RawTransfer {
            token: address!("0x000000000000000000000000000000000000feed"),
            from: address!("0x0000000000000000000000000000000000000000"),
            to: us(),
            amount: U256::from(1_000_000_000u64),
            symbol: "aEthUSDC".into(),
            decimals: 6,
        };
        let actions = AaveV3
            .decode(us(), &tx, &[send_usdc, recv_ausdc])
            .expect("decode");
        assert!(matches!(
            &actions[..],
            [Action::Protocol {
                kind: ProtocolKind::Supply,
                ..
            }]
        ));
    }

    #[test]
    fn ignores_off_chain() {
        let tx = RawTx {
            chain_id: 42161,
            hash: "0x1".into(),
            timestamp: 1,
            from: us(),
            to: Some(POOL_V3),
            value_wei: U256::ZERO,
            input_len: 100,
            success: true,
            internals: Vec::new(),
        };
        assert!(AaveV3.decode(us(), &tx, &[]).is_none());
    }
}
