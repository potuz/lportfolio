//! Holdings view: refresh button dispatches `build_snapshot` on a tokio task
//! and renders the resulting `PortfolioSnapshot` as egui grids.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use std::time::Instant;

use alloy::primitives::Address;
use eframe::egui;

use crate::android::app::{LportfolioApp, SnapshotMsg};
use crate::android::data_dir;
use crate::chain::Chain;
use crate::db::Db;
use crate::holdings::{self, NativeRow, PortfolioSnapshot, SplitsRow, gwei_to_wei};
use crate::portfolio_view::{CellAgg, format_amount_compact, format_usd};

#[derive(Default)]
pub struct HoldingsState {
    pub last: Option<PortfolioSnapshot>,
    pub in_flight: bool,
    pub last_error: Option<String>,
    pub last_refreshed_at: Option<Instant>,
}

pub fn ui(app: &mut LportfolioApp, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui.heading("Holdings");

    if app.cfg.is_none() {
        ui.label("No config loaded. Switch to Settings to set up addresses and RPCs.");
        return;
    }

    if let Some(err) = &app.holdings.last_error {
        egui::Frame::group(ui.style())
            .fill(egui::Color32::from_rgb(80, 0, 0))
            .show(ui, |ui| {
                ui.colored_label(egui::Color32::WHITE, err);
            });
    }

    ui.horizontal(|ui| {
        let refresh_btn = ui.add_enabled(!app.holdings.in_flight, egui::Button::new("Refresh"));
        if refresh_btn.clicked() {
            dispatch_refresh(app, false, ctx);
        }
        let force_btn = ui.add_enabled(!app.holdings.in_flight, egui::Button::new("Force refresh"));
        if force_btn.clicked() {
            dispatch_refresh(app, true, ctx);
        }
        if app.holdings.in_flight {
            ui.spinner();
            ui.label("Fetching...");
        }
    });

    let safes: BTreeSet<String> = app
        .cfg
        .as_ref()
        .map(|c| c.safes.clone())
        .unwrap_or_default();

    if let Some(snap) = app.holdings.last.clone() {
        ui.add_space(8.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            if !snap.native.is_empty() {
                ui.label(egui::RichText::new("Native + ERC-20").strong());
                render_native_grid(ui, &snap, &safes);
                ui.add_space(8.0);
            }
            if !snap.staking.is_empty() {
                ui.label(egui::RichText::new("Beacon staking").strong());
                render_staking_grid(ui, &snap);
                ui.add_space(8.0);
            }
            if !snap.csm.is_empty() {
                ui.label(egui::RichText::new("Lido CSM bonds").strong());
                render_csm_grid(ui, &snap);
                ui.add_space(8.0);
            }
            if !snap.splits.is_empty() {
                ui.label(egui::RichText::new("Splits claims").strong());
                render_splits_grid(ui, &snap);
                ui.add_space(8.0);
            }
            render_grand_total(ui, &snap);
        });
    } else if !app.holdings.in_flight {
        ui.add_space(8.0);
        ui.label("Tap Refresh to fetch your current holdings.");
    }
}

fn dispatch_refresh(app: &mut LportfolioApp, force: bool, ctx: &egui::Context) {
    let cfg = match app.cfg.clone() {
        Some(c) => c,
        None => {
            app.holdings.last_error = Some("no config loaded".to_string());
            return;
        }
    };
    let db_path = cfg
        .db_path_override
        .clone()
        .unwrap_or_else(data_dir::default_db_path);

    let (tx, rx) = mpsc::channel();
    app.snapshot_rx = Some(rx);
    app.holdings.in_flight = true;
    app.holdings.last_error = None;
    let ctx_clone = ctx.clone();

    app.runtime.spawn(async move {
        let result = async {
            let mut db = Db::open_at(&db_path)?;
            holdings::build_snapshot(&cfg, &mut db, force).await
        }
        .await;

        let msg = match result {
            Ok(snap) => SnapshotMsg::Ok(snap),
            Err(e) => SnapshotMsg::Err(format!("{e:#}")),
        };
        let _ = tx.send(msg);
        ctx_clone.request_repaint();
    });

    app.holdings.last_refreshed_at = Some(Instant::now());
}

fn render_native_grid(ui: &mut egui::Ui, snap: &PortfolioSnapshot, safes: &BTreeSet<String>) {
    let present_chains: Vec<Chain> = Chain::ALL
        .iter()
        .copied()
        .filter(|c| snap.native.iter().any(|r| r.chain == *c))
        .collect();

    let (grouped, col_totals, grand) = aggregate_native(&snap.native);

    egui::Grid::new("native").striped(true).show(ui, |ui| {
        ui.label(egui::RichText::new("Alias").strong());
        ui.label(egui::RichText::new("Address").strong());
        for chain in &present_chains {
            ui.label(egui::RichText::new(chain.name()).strong());
        }
        ui.label(egui::RichText::new("Total").strong());
        ui.end_row();

        for ((alias, address), per_chain) in &grouped {
            let alias_display = if safes.contains(alias) {
                format!("{alias} (Safe)")
            } else {
                alias.clone()
            };
            ui.label(alias_display);
            ui.label(short_addr(*address));
            let mut row_total = CellAgg::default();
            for chain in &present_chains {
                let cell = per_chain.get(chain).cloned().unwrap_or_default();
                row_total.merge(&cell);
                ui.label(cell.render());
            }
            ui.label(row_total.render());
            ui.end_row();
        }

        ui.label(egui::RichText::new("Total").strong());
        ui.label("");
        for chain in &present_chains {
            let agg = col_totals.get(chain).cloned().unwrap_or_default();
            ui.label(agg.render());
        }
        ui.label(grand.render());
        ui.end_row();

        if let Some(eth_usd) = snap.prices.lookup("ETH") {
            ui.label(egui::RichText::new("Total (USD)").strong());
            ui.label("");
            let mut grand_usd = 0.0;
            for chain in &present_chains {
                let agg = col_totals.get(chain).cloned().unwrap_or_default();
                let usd = agg.usd(eth_usd, &snap.prices);
                grand_usd += usd;
                ui.label(format_usd(usd));
            }
            ui.label(format_usd(grand_usd));
            ui.end_row();
        }
    });
}

fn aggregate_native(
    rows: &[NativeRow],
) -> (
    BTreeMap<(String, Address), BTreeMap<Chain, CellAgg>>,
    BTreeMap<Chain, CellAgg>,
    CellAgg,
) {
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
    for per_chain in grouped.values() {
        for (chain, cell) in per_chain {
            col_totals.entry(*chain).or_default().merge(cell);
            grand.merge(cell);
        }
    }
    (grouped, col_totals, grand)
}

fn render_staking_grid(ui: &mut egui::Ui, snap: &PortfolioSnapshot) {
    egui::Grid::new("staking").striped(true).show(ui, |ui| {
        ui.label(egui::RichText::new("Alias").strong());
        ui.label(egui::RichText::new("Validators").strong());
        ui.label(egui::RichText::new("Beacon balance").strong());
        ui.label(egui::RichText::new("Source").strong());
        ui.end_row();
        for r in &snap.staking {
            let balance_wei = gwei_to_wei(r.total_balance_gwei);
            let amount = format_amount_compact(balance_wei, 18, 6);
            ui.label(r.alias.clone());
            ui.label(r.validator_count.to_string());
            ui.label(format!("{amount} ETH"));
            ui.label(if r.from_cache { "(cached)" } else { "(fresh)" });
            ui.end_row();
        }
    });
}

fn render_csm_grid(ui: &mut egui::Ui, snap: &PortfolioSnapshot) {
    egui::Grid::new("csm").striped(true).show(ui, |ui| {
        ui.label(egui::RichText::new("Operator ID").strong());
        ui.label(egui::RichText::new("Bond").strong());
        ui.end_row();
        for r in &snap.csm {
            ui.label(r.operator_id.to_string());
            ui.label(format!(
                "{} stETH",
                format_amount_compact(r.bond_steth_wei, 18, 6)
            ));
            ui.end_row();
        }
    });
}

fn render_splits_grid(ui: &mut egui::Ui, snap: &PortfolioSnapshot) {
    let present_chains: Vec<Chain> = Chain::ALL
        .iter()
        .copied()
        .filter(|c| snap.splits.iter().any(|r| r.chain == *c))
        .collect();

    let (grouped, col_totals, grand) = aggregate_splits(&snap.splits);

    egui::Grid::new("splits").striped(true).show(ui, |ui| {
        ui.label(egui::RichText::new("Alias").strong());
        ui.label(egui::RichText::new("Address").strong());
        for chain in &present_chains {
            ui.label(egui::RichText::new(chain.name()).strong());
        }
        ui.label(egui::RichText::new("Total").strong());
        ui.end_row();

        for ((alias, address), per_chain) in &grouped {
            ui.label(alias.clone());
            ui.label(short_addr(*address));
            let mut row_total = CellAgg::default();
            for chain in &present_chains {
                let cell = per_chain.get(chain).cloned().unwrap_or_default();
                row_total.merge(&cell);
                ui.label(cell.render());
            }
            ui.label(row_total.render());
            ui.end_row();
        }

        ui.label(egui::RichText::new("Total").strong());
        ui.label("");
        for chain in &present_chains {
            let agg = col_totals.get(chain).cloned().unwrap_or_default();
            ui.label(agg.render());
        }
        ui.label(grand.render());
        ui.end_row();

        if let Some(eth_usd) = snap.prices.lookup("ETH") {
            ui.label(egui::RichText::new("Total (USD)").strong());
            ui.label("");
            let mut grand_usd = 0.0;
            for chain in &present_chains {
                let agg = col_totals.get(chain).cloned().unwrap_or_default();
                let usd = agg.usd(eth_usd, &snap.prices);
                grand_usd += usd;
                ui.label(format_usd(usd));
            }
            ui.label(format_usd(grand_usd));
            ui.end_row();
        }
    });
}

fn aggregate_splits(
    rows: &[SplitsRow],
) -> (
    BTreeMap<(String, Address), BTreeMap<Chain, CellAgg>>,
    BTreeMap<Chain, CellAgg>,
    CellAgg,
) {
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
    for per_chain in grouped.values() {
        for (chain, cell) in per_chain {
            col_totals.entry(*chain).or_default().merge(cell);
            grand.merge(cell);
        }
    }
    (grouped, col_totals, grand)
}

fn render_grand_total(ui: &mut egui::Ui, snap: &PortfolioSnapshot) {
    let body = match snap.grand_total_usd() {
        Some(usd) => format!("Grand total: {}", format_usd(usd)),
        None => "Grand total: (USD prices unavailable)".to_string(),
    };
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(body)
            .strong()
            .color(egui::Color32::from_rgb(0, 200, 0)),
    );
}

fn short_addr(addr: Address) -> String {
    let s = format!("{addr:#x}");
    if s.len() <= 12 {
        s
    } else {
        format!("{}…{}", &s[..6], &s[s.len() - 4..])
    }
}
