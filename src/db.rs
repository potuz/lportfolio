use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

const SCHEMA_SQL: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY
);
INSERT OR IGNORE INTO schema_version (version) VALUES (1);

CREATE TABLE IF NOT EXISTS addresses (
    address  TEXT PRIMARY KEY,
    alias    TEXT NOT NULL UNIQUE,
    added_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);

CREATE TABLE IF NOT EXISTS chains (
    chain_id     INTEGER PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    explorer_url TEXT
);

CREATE TABLE IF NOT EXISTS sync_state (
    address    TEXT NOT NULL,
    chain_id   INTEGER NOT NULL,
    last_block INTEGER NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    PRIMARY KEY (address, chain_id)
);

CREATE TABLE IF NOT EXISTS transactions (
    chain_id     INTEGER NOT NULL,
    hash         TEXT NOT NULL,
    block_number INTEGER NOT NULL,
    timestamp    INTEGER NOT NULL,
    from_addr    TEXT NOT NULL,
    to_addr      TEXT,
    value_wei    TEXT NOT NULL,
    input        BLOB NOT NULL,
    status       INTEGER NOT NULL,
    PRIMARY KEY (chain_id, hash)
);

CREATE INDEX IF NOT EXISTS transactions_block_idx
    ON transactions(chain_id, block_number);

CREATE TABLE IF NOT EXISTS transfers (
    chain_id  INTEGER NOT NULL,
    tx_hash   TEXT NOT NULL,
    log_index INTEGER NOT NULL,
    token     TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    to_addr   TEXT NOT NULL,
    amount    TEXT NOT NULL,
    PRIMARY KEY (chain_id, tx_hash, log_index)
);

CREATE INDEX IF NOT EXISTS transfers_addr_idx
    ON transfers(chain_id, from_addr, to_addr);

CREATE TABLE IF NOT EXISTS labels (
    chain_id INTEGER NOT NULL,
    address  TEXT NOT NULL,
    label    TEXT NOT NULL,
    kind     TEXT NOT NULL,
    PRIMARY KEY (chain_id, address)
);

CREATE TABLE IF NOT EXISTS holdings_snapshot (
    address    TEXT NOT NULL,
    chain_id   INTEGER NOT NULL,
    token      TEXT NOT NULL,
    balance    TEXT NOT NULL,
    fetched_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    PRIMARY KEY (address, chain_id, token)
);
"#;

pub struct Db {
    #[allow(dead_code)]
    conn: Connection,
}

impl Db {
    pub fn open() -> Result<Self> {
        let path = default_db_path()?;
        Self::open_at(&path)
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db dir {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite at {}", path.display()))?;
        conn.execute_batch(SCHEMA_SQL).context("applying schema")?;
        Ok(Self { conn })
    }
}

fn default_db_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("LPORTFOLIO_DB_PATH") {
        return Ok(PathBuf::from(p));
    }
    let dir = dirs::data_dir().context("could not determine data directory")?;
    Ok(dir.join("lportfolio").join("db.sqlite"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_applies_schema_in_tempfile() {
        let tmp =
            std::env::temp_dir().join(format!("lportfolio-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let db = Db::open_at(&tmp).expect("open db");

        let count: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                ["addresses"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let version: i64 = db
            .conn
            .query_row("SELECT version FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);

        std::fs::remove_file(&tmp).ok();
    }
}
