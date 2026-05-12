//! Process-global handle to the Android app-private files directory.
//!
//! Set once from `android_main` (via `AndroidApp::internal_data_path`) and read
//! by everything else: TOML config location, SQLite database path, etc.

use std::path::PathBuf;
use std::sync::OnceLock;

static FILES_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Record the Android `internalDataPath`. Idempotent — first call wins.
pub fn set(path: PathBuf) {
    let _ = FILES_DIR.set(path);
}

/// The directory where this app may read/write private files. Panics if not
/// initialised; only callable after `android_main` has run.
pub fn get() -> PathBuf {
    FILES_DIR
        .get()
        .cloned()
        .expect("android files dir not initialised; android_main must call set() first")
}

pub fn config_path() -> PathBuf {
    get().join("config.toml")
}

pub fn default_db_path() -> PathBuf {
    get().join("db.sqlite")
}
