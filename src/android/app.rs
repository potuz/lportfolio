//! Top-level eframe app. Holds the loaded `Config`, a tokio runtime for
//! background refreshes, and dispatches to the Settings / Holdings screens.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui;

use crate::android::data_dir;
use crate::android::screens::{HoldingsState, SettingsState};
use crate::config::Config;
use crate::holdings::PortfolioSnapshot;

pub enum SnapshotMsg {
    Ok(PortfolioSnapshot),
    Err(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Settings,
    Holdings,
}

pub struct LportfolioApp {
    pub cfg: Option<Config>,
    pub cfg_path: PathBuf,
    pub screen: Screen,
    pub settings: SettingsState,
    pub holdings: HoldingsState,
    pub runtime: tokio::runtime::Runtime,
    pub snapshot_rx: Option<mpsc::Receiver<SnapshotMsg>>,
    pub load_error: Option<String>,
}

impl LportfolioApp {
    pub fn new() -> Self {
        let cfg_path = data_dir::config_path();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        let (cfg, load_error, screen, settings) = if cfg_path.exists() {
            match Config::from_toml(&cfg_path) {
                Ok(cfg) => {
                    let settings = SettingsState::from_config(&cfg);
                    (Some(cfg), None, Screen::Holdings, settings)
                }
                Err(e) => (
                    None,
                    Some(format!("failed to load config: {e:#}")),
                    Screen::Settings,
                    SettingsState::default(),
                ),
            }
        } else {
            (None, None, Screen::Settings, SettingsState::default())
        };

        Self {
            cfg,
            cfg_path,
            screen,
            settings,
            holdings: HoldingsState::default(),
            runtime,
            snapshot_rx: None,
            load_error,
        }
    }
}

impl Default for LportfolioApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for LportfolioApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.snapshot_rx {
            while let Ok(msg) = rx.try_recv() {
                self.holdings.in_flight = false;
                match msg {
                    SnapshotMsg::Ok(snap) => {
                        self.holdings.last = Some(snap);
                        self.holdings.last_error = None;
                    }
                    SnapshotMsg::Err(e) => {
                        self.holdings.last_error = Some(e);
                    }
                }
            }
        }

        let ctx = ui.ctx().clone();

        egui::Panel::top("nav").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.screen, Screen::Settings, "Settings");
                ui.selectable_value(&mut self.screen, Screen::Holdings, "Holdings");
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            if let Some(err) = &self.load_error {
                ui.colored_label(egui::Color32::RED, err);
                ui.separator();
            }
            match self.screen {
                Screen::Settings => crate::android::screens::settings::ui(self, ui),
                Screen::Holdings => crate::android::screens::holdings::ui(self, ui, &ctx),
            }
        });

        if self.holdings.in_flight {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}
