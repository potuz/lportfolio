use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::sync::OnceLock;

use alloy::primitives::{Address, U256};
use comfy_table::Table;
use comfy_table::presets::UTF8_FULL;

use crate::chain::Chain;
use crate::decode::{Action, AssetAmount, DecodedTx, Direction};
use crate::holdings::{CsmRow, NativeRow, PortfolioSnapshot, SplitsRow, StakingRow, gwei_to_wei};
use crate::portfolio_view::{CellAgg, format_usd};
use crate::prices::PriceTable;

pub mod paint {
    use super::*;

    static USE_COLOR: OnceLock<bool> = OnceLock::new();

    /// Should we emit ANSI escapes? Decided once per process based on stdout TTY.
    pub fn enabled() -> bool {
        *USE_COLOR.get_or_init(|| std::io::stdout().is_terminal())
    }

    fn wrap(text: &str, code: &str) -> String {
        if enabled() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    pub fn bold_green(s: &str) -> String {
        wrap(s, "1;32")
    }

    pub fn header(text: &str) -> String {
        if enabled() {
            format!("\x1b[1;4m{text}\x1b[0m")
        } else {
            format!("== {text} ==")
        }
    }
}

fn new_table() -> Table {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL);
    t
}

fn header_row(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

pub fn render_history(
    decoded: &[DecodedTx],
    aliases: &BTreeMap<Address, String>,
    labels: &BTreeMap<(u64, Address), String>,
) -> Table {
    let mut table = new_table();
    table.set_header(header_row(&["Time (UTC)", "Chain", "Tx", "Action"]));
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
        Action::Protocol {
            protocol,
            kind,
            contract,
            sent,
            received,
        } => {
            let header = format!(
                "[{protocol}] {kind} ({})",
                display_address(*contract, chain_id, aliases, labels),
                kind = kind.label(),
            );
            let sent_part = if sent.is_empty() {
                String::new()
            } else {
                format!(" sent {}", join_assets(sent))
            };
            let recv_part = if received.is_empty() {
                String::new()
            } else {
                format!(" received {}", join_assets(received))
            };
            format!("{header}{sent_part}{recv_part}")
        }
    }
}

fn join_assets(items: &[AssetAmount]) -> String {
    items
        .iter()
        .map(|a| {
            let display_amount = match a.token {
                None => format_eth_amount(a.amount),
                Some(_) => format_token_amount(a.amount, a.decimals),
            };
            let symbol = if a.symbol.is_empty() {
                match a.token {
                    None => "ETH".to_string(),
                    Some(addr) => short_addr(addr),
                }
            } else {
                a.symbol.clone()
            };
            format!("{display_amount} {symbol}")
        })
        .collect::<Vec<_>>()
        .join(", ")
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
        return add_thousands_separators(&amount.to_string());
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
    let mut combined = format!("{}.{}", add_thousands_separators(int_part), frac_keep);
    while combined.ends_with('0') {
        combined.pop();
    }
    if combined.ends_with('.') {
        combined.pop();
    }
    combined
}

fn add_thousands_separators(int_str: &str) -> String {
    crate::portfolio_view::add_thousands_separators(int_str)
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

pub fn print_section(heading: &str) {
    println!("\n{}", paint::header(heading));
}

pub fn render_native(rows: &[NativeRow], safes: &BTreeSet<String>, prices: &PriceTable) -> Table {
    let mut t = new_table();

    let present_chains: Vec<Chain> = Chain::ALL
        .iter()
        .copied()
        .filter(|c| rows.iter().any(|r| r.chain == *c))
        .collect();

    let mut header_cells: Vec<&str> = vec!["Alias", "Address"];
    for chain in &present_chains {
        header_cells.push(chain.name());
    }
    header_cells.push("Total");
    t.set_header(header_row(&header_cells));

    let mut grouped: BTreeMap<(String, Address), BTreeMap<Chain, CellAgg>> = BTreeMap::new();
    for r in rows {
        let cell = grouped
            .entry((r.alias.clone(), r.address))
            .or_default()
            .entry(r.chain)
            .or_default();
        cell.add_native(r.balance_wei);
        for tok in &r.tokens {
            cell.add_token(&tok.display_symbol, tok.amount, tok.decimals);
        }
    }

    let mut col_totals: BTreeMap<Chain, CellAgg> = BTreeMap::new();
    let mut grand = CellAgg::default();

    for ((alias, address), per_chain) in &grouped {
        let alias_display = if safes.contains(alias) {
            format!("{alias} (Safe)")
        } else {
            alias.clone()
        };
        let mut cells: Vec<String> = vec![alias_display, format!("{address:#x}")];
        let mut row_total = CellAgg::default();
        for chain in &present_chains {
            let cell = per_chain.get(chain).cloned().unwrap_or_default();
            row_total.merge(&cell);
            col_totals.entry(*chain).or_default().merge(&cell);
            cells.push(cell.render());
        }
        grand.merge(&row_total);
        cells.push(row_total.render());
        t.add_row(cells);
    }

    let mut total_cells: Vec<String> = vec!["Total".into(), String::new()];
    for chain in &present_chains {
        let agg = col_totals.get(chain).cloned().unwrap_or_default();
        total_cells.push(agg.render());
    }
    total_cells.push(grand.render());
    t.add_row(total_cells);

    if !prices.is_empty()
        && let Some(eth_usd) = prices.lookup("ETH")
    {
        let mut usd_cells: Vec<String> = vec!["Total (USD)".into(), String::new()];
        let mut grand_usd = 0.0;
        for chain in &present_chains {
            let agg = col_totals.get(chain).cloned().unwrap_or_default();
            let usd = agg.usd(eth_usd, prices);
            grand_usd += usd;
            usd_cells.push(format_usd(usd));
        }
        usd_cells.push(format_usd(grand_usd));
        t.add_row(usd_cells);
    }

    t
}

pub fn render_staking(rows: &[StakingRow]) -> Table {
    let mut t = new_table();
    t.set_header(header_row(&[
        "Alias",
        "Validators",
        "Beacon balance",
        "Source",
    ]));
    for r in rows {
        let balance_wei = gwei_to_wei(r.total_balance_gwei);
        let amount = format_eth_amount(balance_wei);
        let source = if r.from_cache { "(cached)" } else { "(fresh)" };
        t.add_row(vec![
            r.alias.clone(),
            r.validator_count.to_string(),
            format!("{amount} ETH"),
            source.to_string(),
        ]);
    }
    t
}

pub fn render_csm(rows: &[CsmRow]) -> Table {
    let mut t = new_table();
    t.set_header(header_row(&["Operator ID", "Bond"]));
    for r in rows {
        t.add_row(vec![
            r.operator_id.to_string(),
            format!("{} stETH", format_eth_amount(r.bond_steth_wei)),
        ]);
    }
    t
}

pub fn render_splits(rows: &[SplitsRow], prices: &PriceTable) -> Table {
    let mut t = new_table();

    let present_chains: Vec<Chain> = Chain::ALL
        .iter()
        .copied()
        .filter(|c| rows.iter().any(|r| r.chain == *c))
        .collect();

    let mut header_cells: Vec<&str> = vec!["Alias", "Address"];
    for chain in &present_chains {
        header_cells.push(chain.name());
    }
    header_cells.push("Total");
    t.set_header(header_row(&header_cells));

    let mut grouped: BTreeMap<(String, Address), BTreeMap<Chain, CellAgg>> = BTreeMap::new();
    for r in rows {
        let cell = grouped
            .entry((r.alias.clone(), r.address))
            .or_default()
            .entry(r.chain)
            .or_default();
        match r.token {
            None => cell.add_native(r.amount),
            Some(_) => cell.add_token(&r.display_symbol, r.amount, r.decimals),
        }
    }

    let mut col_totals: BTreeMap<Chain, CellAgg> = BTreeMap::new();
    let mut grand = CellAgg::default();

    for ((alias, address), per_chain) in &grouped {
        let mut cells: Vec<String> = vec![alias.clone(), format!("{address:#x}")];
        let mut row_total = CellAgg::default();
        for chain in &present_chains {
            let cell = per_chain.get(chain).cloned().unwrap_or_default();
            row_total.merge(&cell);
            col_totals.entry(*chain).or_default().merge(&cell);
            cells.push(cell.render());
        }
        grand.merge(&row_total);
        cells.push(row_total.render());
        t.add_row(cells);
    }

    let mut total_cells: Vec<String> = vec!["Total".into(), String::new()];
    for chain in &present_chains {
        let agg = col_totals.get(chain).cloned().unwrap_or_default();
        total_cells.push(agg.render());
    }
    total_cells.push(grand.render());
    t.add_row(total_cells);

    if !prices.is_empty()
        && let Some(eth_usd) = prices.lookup("ETH")
    {
        let mut usd_cells: Vec<String> = vec!["Total (USD)".into(), String::new()];
        let mut grand_usd = 0.0;
        for chain in &present_chains {
            let agg = col_totals.get(chain).cloned().unwrap_or_default();
            let usd = agg.usd(eth_usd, prices);
            grand_usd += usd;
            usd_cells.push(format_usd(usd));
        }
        usd_cells.push(format_usd(grand_usd));
        t.add_row(usd_cells);
    }

    t
}

/// Print the grand total in USD after the holdings tables. Includes native +
/// ERC-20 + staking + CSM. Renders inside a unicode-bordered box for
/// emphasis. ANSI escapes live outside any table cell so cell-width math
/// isn't fooled by escape bytes.
pub fn print_grand_total(snap: &PortfolioSnapshot) {
    let body = match snap.grand_total_usd() {
        Some(usd) => format!("Grand total:  {}", format_usd(usd)),
        None => "Grand total:  (USD prices unavailable)".into(),
    };
    let inner_width = body.chars().count() + 4; // 2 spaces of padding on each side
    let top = format!("╭{}╮", "─".repeat(inner_width));
    let bot = format!("╰{}╯", "─".repeat(inner_width));
    let middle = format!("│  {}  │", paint::bold_green(&body));
    println!();
    println!("{top}");
    println!("{middle}");
    println!("{bot}");
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
