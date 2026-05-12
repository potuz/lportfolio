//! Android egui frontend. The entire module is gated on
//! `target_os = "android"` (see `lib.rs`); these submodules are only built
//! when cross-compiling to an Android target.

pub mod app;
pub mod data_dir;
pub mod entry;
pub mod screens;
