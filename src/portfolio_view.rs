//! Portable cell-formatting helpers used by both the CLI renderer
//! (`render.rs`) and the Android egui Holdings screen. No terminal or table
//! dependencies live here — purely numeric + string formatting.

use std::collections::BTreeMap;

use alloy::primitives::U256;

use crate::prices::{PriceTable, u256_to_f64};

/// Aggregate of one holdings cell (or a row/column total): native ETH plus a
/// symbol → (amount, decimals) map of ERC-20 holdings.
#[derive(Default, Clone, Debug)]
pub struct CellAgg {
    pub native_wei: U256,
    pub tokens: BTreeMap<String, (U256, u8)>,
}

impl CellAgg {
    pub fn add_native(&mut self, wei: U256) {
        self.native_wei += wei;
    }

    pub fn add_token(&mut self, symbol: &str, amount: U256, decimals: u8) {
        let entry = self
            .tokens
            .entry(symbol.to_string())
            .or_insert((U256::ZERO, decimals));
        entry.0 += amount;
    }

    pub fn merge(&mut self, other: &CellAgg) {
        self.native_wei += other.native_wei;
        for (sym, (amt, dec)) in &other.tokens {
            let entry = self.tokens.entry(sym.clone()).or_insert((U256::ZERO, *dec));
            entry.0 += *amt;
        }
    }

    /// Build (amount_str, symbol) pairs for one cell. Skips ERC-20 amounts
    /// below the display threshold but always includes native ETH.
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = Vec::new();
        entries.push((format_amount_compact(self.native_wei, 18, 4), "ETH".into()));
        for (sym, (amt, dec)) in &self.tokens {
            if !meets_token_threshold(*amt, *dec) {
                continue;
            }
            entries.push((format_amount_compact(*amt, *dec, 2), sym.clone()));
        }
        entries
    }

    /// Multi-line render with amounts right-aligned so symbols line up.
    /// Used by both the CLI table cell and the egui label.
    pub fn render(&self) -> String {
        let entries = self.entries();
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

    /// USD valuation given an ETH price and a price table for ERC-20 symbols.
    pub fn usd(&self, eth_usd: f64, prices: &PriceTable) -> f64 {
        let mut total = u256_to_f64(self.native_wei, 18) * eth_usd;
        for (sym, (amt, dec)) in &self.tokens {
            if let Some(p) = prices.lookup(sym) {
                total += u256_to_f64(*amt, *dec) * p;
            }
        }
        total
    }
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

pub fn format_usd(usd: f64) -> String {
    let cents = (usd * 100.0).round() as i64;
    let abs_cents = cents.abs();
    let dollars = abs_cents / 100;
    let frac = abs_cents % 100;
    let sign = if cents < 0 { "-" } else { "" };
    format!(
        "{sign}${}.{frac:02}",
        add_thousands_separators(&dollars.to_string())
    )
}

pub fn add_thousands_separators(int_str: &str) -> String {
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
