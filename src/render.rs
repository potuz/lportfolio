use std::collections::BTreeMap;

use alloy::primitives::{Address, U256};
use comfy_table::Table;

use crate::chain::Chain;
use crate::decode::{Action, DecodedTx, Direction};

pub fn render_history(
    decoded: &[DecodedTx],
    aliases: &BTreeMap<Address, String>,
    labels: &BTreeMap<(u64, Address), String>,
) -> Table {
    let mut table = Table::new();
    table.set_header(vec!["Time (UTC)", "Chain", "Tx", "Action"]);
    for d in decoded {
        let chain = Chain::from_id(d.chain_id)
            .map(|c| c.name().to_string())
            .unwrap_or_else(|| d.chain_id.to_string());
        let when = format_unix_utc(d.timestamp);
        let hash_short = short_hash(&d.hash);

        if d.actions.is_empty() {
            let action_text = if d.success {
                "(no decoded actions)".to_string()
            } else {
                "(reverted)".to_string()
            };
            table.add_row(vec![
                when.clone(),
                chain.clone(),
                hash_short.clone(),
                action_text,
            ]);
            continue;
        }

        for action in &d.actions {
            let action_text = format_action(action, d.chain_id, aliases, labels);
            let prefixed = if d.success {
                action_text
            } else {
                format!("(reverted) {action_text}")
            };
            table.add_row(vec![
                when.clone(),
                chain.clone(),
                hash_short.clone(),
                prefixed,
            ]);
        }
    }
    table
}

fn format_action(
    action: &Action,
    chain_id: u64,
    aliases: &BTreeMap<Address, String>,
    labels: &BTreeMap<(u64, Address), String>,
) -> String {
    match action {
        Action::NativeTransfer {
            direction,
            counterparty,
            amount_wei,
        } => {
            let amount = format_eth_amount(*amount_wei);
            let cp = display_address(*counterparty, chain_id, aliases, labels);
            match direction {
                Direction::Out => format!("Sent {amount} ETH to {cp}"),
                Direction::In => format!("Received {amount} ETH from {cp}"),
                Direction::SelfTransfer => format!("Self-transfer {amount} ETH"),
            }
        }
        Action::TokenTransfer {
            direction,
            counterparty,
            token,
            symbol,
            decimals,
            amount,
        } => {
            let display_amount = format_token_amount(*amount, *decimals);
            let display_symbol = if symbol.is_empty() {
                short_addr(*token)
            } else {
                symbol.clone()
            };
            let cp = display_address(*counterparty, chain_id, aliases, labels);
            match direction {
                Direction::Out => format!("Sent {display_amount} {display_symbol} to {cp}"),
                Direction::In => format!("Received {display_amount} {display_symbol} from {cp}"),
                Direction::SelfTransfer => {
                    format!("Self-transfer {display_amount} {display_symbol}")
                }
            }
        }
        Action::ContractCall { contract } => {
            let cp = display_address(*contract, chain_id, aliases, labels);
            format!("Called contract {cp}")
        }
    }
}

fn display_address(
    addr: Address,
    chain_id: u64,
    aliases: &BTreeMap<Address, String>,
    labels: &BTreeMap<(u64, Address), String>,
) -> String {
    if let Some(alias) = aliases.get(&addr) {
        alias.clone()
    } else if let Some(label) = labels.get(&(chain_id, addr)) {
        label.clone()
    } else {
        short_addr(addr)
    }
}

fn short_addr(addr: Address) -> String {
    let s = format!("{addr:#x}");
    if s.len() <= 12 {
        s
    } else {
        format!("{}…{}", &s[..6], &s[s.len() - 4..])
    }
}

fn short_hash(hash: &str) -> String {
    if hash.len() <= 12 {
        hash.to_string()
    } else {
        format!("{}…{}", &hash[..6], &hash[hash.len() - 4..])
    }
}

pub fn format_eth_amount(wei: U256) -> String {
    format_with_decimals(wei, 18, 6)
}

pub fn format_token_amount(amount: U256, decimals: u32) -> String {
    let max_frac = decimals.min(6);
    format_with_decimals(amount, decimals, max_frac)
}

fn format_with_decimals(amount: U256, decimals: u32, max_frac: u32) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let raw = amount.to_string();
    let need_len = decimals as usize + 1;
    let padded = if raw.len() < need_len {
        format!("{:0>width$}", raw, width = need_len)
    } else {
        raw
    };
    let len = padded.len();
    let int_part = &padded[..len - decimals as usize];
    let frac_full = &padded[len - decimals as usize..];
    let frac_keep = &frac_full[..max_frac as usize];
    let mut combined = format!("{int_part}.{frac_keep}");
    while combined.ends_with('0') {
        combined.pop();
    }
    if combined.ends_with('.') {
        combined.pop();
    }
    combined
}

fn format_unix_utc(ts: u64) -> String {
    // Minimal UTC formatter: YYYY-MM-DD HH:MM:SS, no leap seconds, Gregorian.
    let (y, mo, d, h, mi, s) = unix_to_ymdhms_utc(ts);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

fn unix_to_ymdhms_utc(ts: u64) -> (i64, u32, u32, u32, u32, u32) {
    let secs_per_day: u64 = 86_400;
    let days = (ts / secs_per_day) as i64;
    let mut secs_of_day = ts % secs_per_day;
    let h = (secs_of_day / 3600) as u32;
    secs_of_day %= 3600;
    let mi = (secs_of_day / 60) as u32;
    let s = (secs_of_day % 60) as u32;

    // Days since 1970-01-01 → Gregorian Y/M/D using Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_epoch_renders_correctly() {
        assert_eq!(format_unix_utc(0), "1970-01-01 00:00:00");
    }

    #[test]
    fn known_timestamp_renders_correctly() {
        // 2024-01-01 00:00:00 UTC = 1_704_067_200
        assert_eq!(format_unix_utc(1_704_067_200), "2024-01-01 00:00:00");
    }

    #[test]
    fn token_amount_formatting() {
        // 1 USDC = 10^6 base units, 6 decimals, formatted as "1"
        assert_eq!(format_token_amount(U256::from(1_000_000u64), 6), "1");
        assert_eq!(format_token_amount(U256::from(1_500_000u64), 6), "1.5");
        assert_eq!(format_token_amount(U256::from(123_456u64), 6), "0.123456");
    }

    #[test]
    fn eth_amount_formatting() {
        let one = U256::from(10u64).pow(U256::from(18));
        assert_eq!(format_eth_amount(one), "1");
        assert_eq!(format_eth_amount(U256::ZERO), "0");
    }

    #[test]
    fn short_hash_truncation() {
        assert_eq!(short_hash("0x1234"), "0x1234");
        assert_eq!(
            short_hash("0x1234567890abcdef1234567890abcdef"),
            "0x1234…cdef"
        );
    }
}
