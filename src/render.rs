use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::sync::OnceLock;

use alloy::primitives::{Address, U256};
use comfy_table::Table;
use comfy_table::presets::UTF8_FULL;

use crate::chain::Chain;
use crate::decode::{Action, AssetAmount, DecodedTx, Direction};
use crate::holdings::{CsmRow, NativeRow, PortfolioSnapshot, StakingRow, gwei_to_wei};

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

    pub fn bold(s: &str) -> String {
        wrap(s, "1")
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

/// Compact fixed-decimal format used inside holdings cells. Always shows
/// `frac_digits` decimal places, with thousands separators on the integer
/// part. Truncates (does not round) precision below the chosen step.
pub fn format_amount_compact(amount: U256, decimals: u8, frac_digits: u8) -> String {
    let int_part_str: String;
    let frac_part_str: String;
    let target = frac_digits as usize;
    if decimals == 0 {
        int_part_str = amount.to_string();
        frac_part_str = "0".repeat(target);
    } else {
        let raw = amount.to_string();
        let need_len = decimals as usize + 1;
        let padded = if raw.len() < need_len {
            format!("{:0>width$}", raw, width = need_len)
        } else {
            raw
        };
        let len = padded.len();
        int_part_str = padded[..len - decimals as usize].to_string();
        let frac_full = &padded[len - decimals as usize..];
        frac_part_str = if frac_full.len() >= target {
            frac_full[..target].to_string()
        } else {
            format!("{frac_full:0<target$}")
        };
    }
    if target == 0 {
        return add_thousands_separators(&int_part_str);
    }
    format!(
        "{}.{}",
        add_thousands_separators(&int_part_str),
        frac_part_str
    )
}

fn add_thousands_separators(int_str: &str) -> String {
    let len = int_str.len();
    if len <= 3 {
        return int_str.to_string();
    }
    let mut out = String::with_capacity(len + (len - 1) / 3);
    for (i, c) in int_str.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
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

/// Aggregate of one cell (or a row/column total): native ETH plus a
/// symbol → (amount, decimals) map of ERC-20 holdings.
#[derive(Default, Clone)]
struct CellAgg {
    native_wei: U256,
    tokens: BTreeMap<String, (U256, u8)>,
}

impl CellAgg {
    fn add_native(&mut self, wei: U256) {
        self.native_wei += wei;
    }
    fn add_token(&mut self, symbol: &str, amount: U256, decimals: u8) {
        let entry = self
            .tokens
            .entry(symbol.to_string())
            .or_insert((U256::ZERO, decimals));
        entry.0 += amount;
    }
    fn merge(&mut self, other: &CellAgg) {
        self.native_wei += other.native_wei;
        for (sym, (amt, dec)) in &other.tokens {
            let entry = self.tokens.entry(sym.clone()).or_insert((U256::ZERO, *dec));
            entry.0 += *amt;
        }
    }
    fn render(&self) -> String {
        // Build (amount_str, symbol) pairs first, then right-align amounts so
        // the symbol column lines up regardless of how wide each number is.
        let mut entries: Vec<(String, &str)> = Vec::new();
        entries.push((format_amount_compact(self.native_wei, 18, 4), "ETH"));
        for (sym, (amt, dec)) in &self.tokens {
            if !meets_token_threshold(*amt, *dec) {
                continue;
            }
            entries.push((format_amount_compact(*amt, *dec, 2), sym.as_str()));
        }
        let max_amt_width = entries
            .iter()
            .map(|(a, _)| a.chars().count())
            .max()
            .unwrap_or(0);
        entries
            .iter()
            .map(|(amt, sym)| format!("{amt:>max_amt_width$} {sym}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 0.01 of `decimals`-precision token in raw units, or 1 wei for very
/// low-decimal tokens.
fn meets_token_threshold(amount: U256, decimals: u8) -> bool {
    if amount.is_zero() {
        return false;
    }
    if decimals < 2 {
        return true;
    }
    let scale = U256::from(10u64).pow(U256::from(u64::from(decimals - 2)));
    amount >= scale
}

pub fn render_native(rows: &[NativeRow]) -> Table {
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

/// Print the grand total after the holdings tables. ANSI escapes live here
/// (outside any table) so cell-width math isn't fooled by escape bytes.
pub fn print_grand_total(snap: &PortfolioSnapshot) {
    let total = snap.grand_total_eth_wei();
    println!(
        "{} {}",
        paint::bold("Grand total:"),
        paint::bold_green(&format!("{} ETH", format_eth_amount(total))),
    );
    let csm = snap.grand_total_steth_wei();
    if !csm.is_zero() {
        println!(
            "{} {}",
            paint::bold("CSM bond total:"),
            paint::bold_green(&format!("{} stETH", format_eth_amount(csm))),
        );
    }
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
