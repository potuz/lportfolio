use alloy::primitives::{Address, address};

use super::erc20::{RawTransfer, RawTx};
use super::{
    Action, AssetAmount, ContractDecoder, KnownContract, ProtocolKind, inbound_assets, native_sent,
    outbound_assets,
};

const STETH: Address = address!("0xae7ab96520de3a18e5e111b5eaab095312d7fe84");
const WSTETH: Address = address!("0x7f39c581f595b53c5cb19bd0b3f8da6c935e2ca0");
const WITHDRAWAL_QUEUE: Address = address!("0x889edC2eDab5f40e902b864aD4d7AdE8E412F9B1");
const CSM: Address = address!("0xdA7dE2EcdDfccC6c3aF10108Db212ACBBf9EA83F");

const KNOWN: &[KnownContract] = &[
    KnownContract {
        chain_id: 1,
        address: STETH,
        label: "Lido stETH",
    },
    KnownContract {
        chain_id: 1,
        address: WSTETH,
        label: "Lido wstETH",
    },
    KnownContract {
        chain_id: 1,
        address: WITHDRAWAL_QUEUE,
        label: "Lido Withdrawal Queue",
    },
    KnownContract {
        chain_id: 1,
        address: CSM,
        label: "Lido CSM",
    },
];

pub struct Lido;

impl ContractDecoder for Lido {
    fn known_contracts(&self) -> &[KnownContract] {
        KNOWN
    }

    fn decode(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> Option<Vec<Action>> {
        if tx.chain_id != 1 {
            return None;
        }
        let to = tx.to?;
        let inbound = inbound_assets(us, transfers);
        let outbound = outbound_assets(us, transfers);

        match to {
            STETH => {
                // Submit (stake): user sends ETH; receives stETH (mint from 0x0).
                if let Some(eth_in) = native_sent(us, tx)
                    && inbound.iter().any(|a| a.token == Some(STETH))
                {
                    return Some(vec![Action::Protocol {
                        protocol: "Lido",
                        kind: ProtocolKind::Stake,
                        contract: STETH,
                        sent: vec![eth_in],
                        received: inbound,
                    }]);
                }
                None
            }
            WSTETH => {
                // Wrap: send stETH, receive wstETH. Unwrap: opposite.
                let sent_steth = outbound.iter().any(|a| a.token == Some(STETH));
                let recv_wsteth = inbound.iter().any(|a| a.token == Some(WSTETH));
                let sent_wsteth = outbound.iter().any(|a| a.token == Some(WSTETH));
                let recv_steth = inbound.iter().any(|a| a.token == Some(STETH));
                if sent_steth && recv_wsteth {
                    return Some(vec![Action::Protocol {
                        protocol: "Lido",
                        kind: ProtocolKind::Wrap,
                        contract: WSTETH,
                        sent: outbound,
                        received: inbound,
                    }]);
                }
                if sent_wsteth && recv_steth {
                    return Some(vec![Action::Protocol {
                        protocol: "Lido",
                        kind: ProtocolKind::Unwrap,
                        contract: WSTETH,
                        sent: outbound,
                        received: inbound,
                    }]);
                }
                None
            }
            WITHDRAWAL_QUEUE => {
                // Request: send stETH (or wstETH); receives an unstETH NFT (also a transfer).
                let sent_lido = outbound
                    .iter()
                    .any(|a| a.token == Some(STETH) || a.token == Some(WSTETH));
                if sent_lido {
                    return Some(vec![Action::Protocol {
                        protocol: "Lido",
                        kind: ProtocolKind::Unstake,
                        contract: WITHDRAWAL_QUEUE,
                        sent: outbound,
                        received: inbound,
                    }]);
                }
                // Claim: receive ETH, NFT burned (no inbound on us as `from` of transfer).
                if !inbound.is_empty() || !tx.value_wei.is_zero() {
                    let received: Vec<AssetAmount> = if let Some(amt) = native_sent_inverted(us, tx)
                    {
                        let mut v = inbound;
                        v.push(amt);
                        v
                    } else {
                        inbound
                    };
                    return Some(vec![Action::Protocol {
                        protocol: "Lido",
                        kind: ProtocolKind::Claim,
                        contract: WITHDRAWAL_QUEUE,
                        sent: outbound,
                        received,
                    }]);
                }
                None
            }
            CSM => Some(vec![Action::Protocol {
                protocol: "Lido CSM",
                kind: ProtocolKind::Other,
                contract: CSM,
                sent: native_sent(us, tx).map(|a| vec![a]).unwrap_or_default(),
                received: inbound,
            }]),
            _ => None,
        }
    }
}

/// Mirror of `native_sent` but for the inverse direction (we received native).
/// Etherscan `txlist` records the tx-level value; for claims, value flows back to us.
fn native_sent_inverted(us: Address, tx: &RawTx) -> Option<AssetAmount> {
    if tx.from != us && tx.to == Some(us) && !tx.value_wei.is_zero() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::U256;

    fn us() -> Address {
        address!("0x000000000000000000000000000000000000beef")
    }

    fn one_eth() -> U256 {
        U256::from(10u64).pow(U256::from(18))
    }

    #[test]
    fn detects_steth_submit_as_stake() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x1".into(),
            timestamp: 1,
            from: us(),
            to: Some(STETH),
            value_wei: one_eth(),
            input_len: 4,
            success: true,
            internals: Vec::new(),
        };
        let xfer = RawTransfer {
            token: STETH,
            from: address!("0x0000000000000000000000000000000000000000"),
            to: us(),
            amount: one_eth(),
            symbol: "stETH".into(),
            decimals: 18,
        };
        let lido = Lido;
        let actions = lido
            .decode(us(), &tx, std::slice::from_ref(&xfer))
            .expect("decode");
        assert!(matches!(
            &actions[..],
            [Action::Protocol {
                protocol: "Lido",
                kind: ProtocolKind::Stake,
                ..
            }]
        ));
    }

    #[test]
    fn detects_wsteth_wrap() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x2".into(),
            timestamp: 1,
            from: us(),
            to: Some(WSTETH),
            value_wei: U256::ZERO,
            input_len: 64,
            success: true,
            internals: Vec::new(),
        };
        let send_steth = RawTransfer {
            token: STETH,
            from: us(),
            to: WSTETH,
            amount: one_eth(),
            symbol: "stETH".into(),
            decimals: 18,
        };
        let recv_wsteth = RawTransfer {
            token: WSTETH,
            from: WSTETH,
            to: us(),
            amount: U256::from(900_000_000_000_000_000u128),
            symbol: "wstETH".into(),
            decimals: 18,
        };
        let lido = Lido;
        let actions = lido
            .decode(us(), &tx, &[send_steth, recv_wsteth])
            .expect("decode");
        assert!(matches!(
            &actions[..],
            [Action::Protocol {
                kind: ProtocolKind::Wrap,
                ..
            }]
        ));
    }

    #[test]
    fn ignores_non_lido_tx() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x3".into(),
            timestamp: 1,
            from: us(),
            to: Some(address!("0x000000000000000000000000000000000000dead")),
            value_wei: one_eth(),
            input_len: 0,
            success: true,
            internals: Vec::new(),
        };
        let lido = Lido;
        assert!(lido.decode(us(), &tx, &[]).is_none());
    }
}
