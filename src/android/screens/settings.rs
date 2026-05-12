//! Settings form: addresses, RPC URLs, beacon node, CSM operators, ERC-20
//! whitelist, safes, optional DB path. Save writes `config.toml`; reload
//! discards in-memory buffers and re-reads from disk.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::android::app::{LportfolioApp, Screen};
use crate::chain::Chain;
use crate::config::{ChainConfig, Config};
use crate::tokens;

pub struct SettingsState {
    pub addresses: Vec<(String, String)>,
    pub rpc_mainnet: String,
    pub rpc_arbitrum: String,
    pub rpc_optimism: String,
    pub rpc_base: String,
    pub beacon_url: String,
    pub validator_indices: String,
    pub csm_operator_ids: String,
    pub token_whitelist: BTreeSet<String>,
    pub safes: BTreeSet<String>,
    pub db_path_override: String,
    pub error: Option<String>,
    pub saved_at: Option<Instant>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            addresses: vec![(String::new(), String::new())],
            rpc_mainnet: String::new(),
            rpc_arbitrum: String::new(),
            rpc_optimism: String::new(),
            rpc_base: String::new(),
            beacon_url: String::new(),
            validator_indices: String::new(),
            csm_operator_ids: String::new(),
            token_whitelist: BTreeSet::new(),
            safes: BTreeSet::new(),
            db_path_override: String::new(),
            error: None,
            saved_at: None,
        }
    }
}

impl SettingsState {
    pub fn from_config(cfg: &Config) -> Self {
        let addresses = cfg
            .addresses
            .iter()
            .map(|(alias, addr)| (alias.clone(), format!("{addr:#x}")))
            .collect::<Vec<_>>();
        let addresses = if addresses.is_empty() {
            vec![(String::new(), String::new())]
        } else {
            addresses
        };
        let rpc = |c: Chain| -> String {
            cfg.chains
                .get(&c)
                .map(|cc| cc.rpc_url.clone())
                .unwrap_or_default()
        };
        Self {
            addresses,
            rpc_mainnet: rpc(Chain::Mainnet),
            rpc_arbitrum: rpc(Chain::Arbitrum),
            rpc_optimism: rpc(Chain::Optimism),
            rpc_base: rpc(Chain::Base),
            beacon_url: cfg.beacon_url.clone().unwrap_or_default(),
            validator_indices: csv_from_u64s(&cfg.validator_indices),
            csm_operator_ids: csv_from_u64s(&cfg.csm_operator_ids),
            token_whitelist: cfg.token_whitelist.iter().cloned().collect(),
            safes: cfg.safes.clone(),
            db_path_override: cfg
                .db_path_override
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            error: None,
            saved_at: None,
        }
    }
}

fn csv_from_u64s(items: &[u64]) -> String {
    items
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_u64_csv(raw: &str, what: &str) -> Result<Vec<u64>, String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<u64>()
                .map_err(|_| format!("invalid {what}: {s:?}"))
        })
        .collect()
}

fn distinct_whitelist_ids() -> Vec<&'static str> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for tok in tokens::REGISTRY {
        if seen.insert(tok.whitelist_id) {
            out.push(tok.whitelist_id);
        }
    }
    out
}

pub fn ui(app: &mut LportfolioApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading("Settings");

        if let Some(err) = &app.settings.error {
            egui::Frame::group(ui.style())
                .fill(egui::Color32::from_rgb(80, 0, 0))
                .show(ui, |ui| {
                    ui.colored_label(egui::Color32::WHITE, err);
                });
        }
        if let Some(t) = app.settings.saved_at
            && t.elapsed() < Duration::from_secs(3)
        {
            ui.colored_label(egui::Color32::LIGHT_GREEN, "Saved.");
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Addresses").strong());
        ui.label(
            egui::RichText::new("Owned addresses to query. Alias is shown in the holdings table.")
                .small(),
        );
        addresses_grid(ui, &mut app.settings);

        ui.add_space(8.0);
        ui.label(egui::RichText::new("RPC URLs").strong());
        labeled_text_edit(ui, "Mainnet", &mut app.settings.rpc_mainnet);
        labeled_text_edit(ui, "Arbitrum", &mut app.settings.rpc_arbitrum);
        labeled_text_edit(ui, "Optimism", &mut app.settings.rpc_optimism);
        labeled_text_edit(ui, "Base", &mut app.settings.rpc_base);

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Beacon node").strong());
        labeled_text_edit(ui, "URL", &mut app.settings.beacon_url);
        labeled_text_edit(
            ui,
            "Validator indices (csv)",
            &mut app.settings.validator_indices,
        );

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Lido CSM").strong());
        labeled_text_edit(ui, "Operator IDs (csv)", &mut app.settings.csm_operator_ids);

        ui.add_space(8.0);
        ui.label(egui::RichText::new("ERC-20 whitelist").strong());
        for token_id in distinct_whitelist_ids() {
            let mut on = app.settings.token_whitelist.contains(token_id);
            if ui.checkbox(&mut on, token_id).changed() {
                if on {
                    app.settings.token_whitelist.insert(token_id.to_string());
                } else {
                    app.settings.token_whitelist.remove(token_id);
                }
            }
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Safes").strong());
        ui.label(
            egui::RichText::new("Aliases that are Gnosis Safe contracts (display tag only).")
                .small(),
        );
        let aliases: Vec<String> = app
            .settings
            .addresses
            .iter()
            .map(|(alias, _)| alias.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for alias in &aliases {
            let mut on = app.settings.safes.contains(alias);
            if ui.checkbox(&mut on, alias).changed() {
                if on {
                    app.settings.safes.insert(alias.clone());
                } else {
                    app.settings.safes.remove(alias);
                }
            }
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Advanced").strong());
        labeled_text_edit(
            ui,
            "DB path override (empty = default)",
            &mut app.settings.db_path_override,
        );

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                match save(app) {
                    Ok(()) => {
                        app.settings.error = None;
                        app.settings.saved_at = Some(Instant::now());
                    }
                    Err(e) => app.settings.error = Some(e),
                }
            }
            if ui.button("Reload from disk").clicked() {
                reload(app);
            }
            if app.cfg.is_some() && ui.button("Go to Holdings").clicked() {
                app.screen = Screen::Holdings;
            }
        });
    });
}

fn labeled_text_edit(ui: &mut egui::Ui, label: &str, buf: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(buf).desired_width(f32::INFINITY));
    });
}

fn addresses_grid(ui: &mut egui::Ui, s: &mut SettingsState) {
    let mut to_remove: Option<usize> = None;
    egui::Grid::new("addresses").striped(true).show(ui, |ui| {
        ui.label(egui::RichText::new("Alias").small());
        ui.label(egui::RichText::new("Address").small());
        ui.label("");
        ui.end_row();
        for (i, (alias, addr)) in s.addresses.iter_mut().enumerate() {
            ui.add(egui::TextEdit::singleline(alias).desired_width(80.0));
            ui.add(egui::TextEdit::singleline(addr).desired_width(f32::INFINITY));
            if ui.button("Remove").clicked() {
                to_remove = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = to_remove {
        s.addresses.remove(i);
    }
    if ui.button("+ Add address").clicked() {
        s.addresses.push((String::new(), String::new()));
    }
}

fn build_config(s: &SettingsState) -> Result<Config, String> {
    let mut cfg = Config::empty();

    for (alias, addr) in &s.addresses {
        let alias = alias.trim();
        let addr = addr.trim();
        if alias.is_empty() && addr.is_empty() {
            continue;
        }
        if alias.is_empty() {
            return Err(format!("address {addr:?} has no alias"));
        }
        if addr.is_empty() {
            return Err(format!("alias {alias:?} has no address"));
        }
        let parsed: alloy::primitives::Address = addr
            .parse()
            .map_err(|e| format!("invalid address for alias {alias:?}: {e}"))?;
        if cfg.addresses.insert(alias.to_string(), parsed).is_some() {
            return Err(format!("duplicate alias: {alias}"));
        }
    }
    if cfg.addresses.is_empty() {
        return Err("at least one address is required".to_string());
    }

    for (chain, url) in [
        (Chain::Mainnet, &s.rpc_mainnet),
        (Chain::Arbitrum, &s.rpc_arbitrum),
        (Chain::Optimism, &s.rpc_optimism),
        (Chain::Base, &s.rpc_base),
    ] {
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        cfg.chains.insert(
            chain,
            ChainConfig {
                rpc_url: url.to_string(),
            },
        );
    }

    cfg.beacon_url = {
        let url = s.beacon_url.trim();
        if url.is_empty() {
            None
        } else {
            Some(url.to_string())
        }
    };
    cfg.validator_indices = parse_u64_csv(&s.validator_indices, "validator index")?;
    cfg.csm_operator_ids = parse_u64_csv(&s.csm_operator_ids, "csm operator id")?;
    cfg.token_whitelist = s.token_whitelist.iter().cloned().collect();

    for alias in &s.safes {
        if !cfg.addresses.contains_key(alias) {
            return Err(format!("safes references unknown alias: {alias:?}"));
        }
    }
    cfg.safes = s.safes.clone();

    let db = s.db_path_override.trim();
    cfg.db_path_override = if db.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(db))
    };

    Ok(cfg)
}

fn save(app: &mut LportfolioApp) -> Result<(), String> {
    let cfg = build_config(&app.settings)?;
    cfg.to_toml(&app.cfg_path)
        .map_err(|e| format!("writing config.toml: {e:#}"))?;
    let reloaded =
        Config::from_toml(&app.cfg_path).map_err(|e| format!("re-reading config.toml: {e:#}"))?;
    app.settings = SettingsState::from_config(&reloaded);
    app.cfg = Some(reloaded);
    app.load_error = None;
    Ok(())
}

fn reload(app: &mut LportfolioApp) {
    if !app.cfg_path.exists() {
        app.settings = SettingsState::default();
        app.cfg = None;
        return;
    }
    match Config::from_toml(&app.cfg_path) {
        Ok(cfg) => {
            app.settings = SettingsState::from_config(&cfg);
            app.cfg = Some(cfg);
            app.settings.error = None;
        }
        Err(e) => {
            app.settings.error = Some(format!("failed to load config: {e:#}"));
        }
    }
}
