use alloy::primitives::{Address, address};

use super::erc20::{RawTransfer, RawTx};
use super::{
    Action, AssetAmount, ContractDecoder, KnownContract, ProtocolKind, inbound_assets,
    native_received, native_sent, outbound_assets,
};

const V2_ROUTER: Address = address!("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D");
const V3_ROUTER_01: Address = address!("0xE592427A0AEce92De3Edee1F18E0157C05861564");
const V3_ROUTER_02: Address = address!("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45");
const UNIVERSAL_ROUTER: Address = address!("0x66a9893cC07D91D95644AEDD05D03f95e1dBA8Af");

const KNOWN: &[KnownContract] = &[
    KnownContract {
        chain_id: 1,
        address: V2_ROUTER,
        label: "Uniswap V2 Router",
    },
    KnownContract {
        chain_id: 1,
        address: V3_ROUTER_01,
        label: "Uniswap V3 SwapRouter",
    },
    KnownContract {
        chain_id: 1,
        address: V3_ROUTER_02,
        label: "Uniswap V3 SwapRouter02",
    },
    KnownContract {
        chain_id: 1,
        address: UNIVERSAL_ROUTER,
        label: "Uniswap Universal Router",
    },
];

pub struct Uniswap;

impl Uniswap {
    fn protocol_label(addr: Address) -> &'static str {
        match addr {
            V2_ROUTER => "Uniswap V2",
            V3_ROUTER_01 | V3_ROUTER_02 => "Uniswap V3",
            UNIVERSAL_ROUTER => "Uniswap",
            _ => "Uniswap",
        }
    }
}

impl ContractDecoder for Uniswap {
    fn known_contracts(&self) -> &[KnownContract] {
        KNOWN
    }

    fn decode(&self, us: Address, tx: &RawTx, transfers: &[RawTransfer]) -> Option<Vec<Action>> {
        if tx.chain_id != 1 {
            return None;
        }
        let to = tx.to?;
        let is_router = matches!(
            to,
            V2_ROUTER | V3_ROUTER_01 | V3_ROUTER_02 | UNIVERSAL_ROUTER
        );
        if !is_router {
            return None;
        }

        let mut sent = outbound_assets(us, transfers);
        if let Some(eth) = native_sent(us, tx) {
            sent.insert(0, eth);
        }
        let mut received: Vec<AssetAmount> = inbound_assets(us, transfers);
        if let Some(eth) = native_received(us, tx) {
            received.insert(0, eth);
        }

        if sent.is_empty() && received.is_empty() {
            return None;
        }

        Some(vec![Action::Protocol {
            protocol: Self::protocol_label(to),
            kind: ProtocolKind::Swap,
            contract: to,
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
    fn detects_v3_swap() {
        let tx = RawTx {
            chain_id: 1,
            hash: "0x1".into(),
            timestamp: 1,
            from: us(),
            to: Some(V3_ROUTER_02),
            value_wei: U256::ZERO,
            input_len: 132,
            success: true,
            internals: Vec::new(),
        };
        let send = RawTransfer {
            token: address!("0x000000000000000000000000000000000000beed"),
            from: us(),
            to: address!("0x0000000000000000000000000000000000001234"),
            amount: U256::from(1_000_000_000u64),
            symbol: "USDC".into(),
            decimals: 6,
        };
        let recv = RawTransfer {
            token: address!("0x000000000000000000000000000000000000feed"),
            from: address!("0x0000000000000000000000000000000000001234"),
            to: us(),
            amount: U256::from(500_000_000_000_000u64),
            symbol: "WETH".into(),
            decimals: 18,
        };
        let actions = Uniswap.decode(us(), &tx, &[send, recv]).expect("decode");
        match &actions[..] {
            [
                Action::Protocol {
                    protocol: "Uniswap V3",
                    kind: ProtocolKind::Swap,
                    ..
                },
            ] => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
}
