use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::chain::Chain;
use crate::decode::erc20::{RawTransfer, RawTx};
use crate::decode::{DecodedTx, Registry};
use crate::explorer::{EtherscanTokenTx, EtherscanTx};

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
    input_len    INTEGER NOT NULL,
    status       INTEGER NOT NULL,
    is_stub      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (chain_id, hash)
);

CREATE INDEX IF NOT EXISTS transactions_block_idx
    ON transactions(chain_id, block_number);

CREATE TABLE IF NOT EXISTS transfers (
    chain_id       INTEGER NOT NULL,
    tx_hash        TEXT NOT NULL,
    log_index      INTEGER NOT NULL,
    token          TEXT NOT NULL,
    from_addr      TEXT NOT NULL,
    to_addr        TEXT NOT NULL,
    amount         TEXT NOT NULL,
    token_symbol   TEXT NOT NULL DEFAULT '',
    token_decimals INTEGER NOT NULL DEFAULT 0,
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

CREATE TABLE IF NOT EXISTS staking_snapshot (
    address            TEXT PRIMARY KEY,
    validator_count    INTEGER NOT NULL,
    total_balance_gwei INTEGER NOT NULL,
    fetched_at         INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);
"#;

pub struct Db {
    conn: Connection,
}

pub struct SyncSummary {
    pub tx_count: usize,
    pub transfer_count: usize,
    pub highest_block: u64,
}

#[derive(Debug, Clone)]
pub struct UnknownCounterparty {
    pub chain_id: u64,
    pub address: Address,
    pub interactions: u64,
}

#[derive(Debug, Clone)]
pub struct CachedStakingSummary {
    pub validator_count: u64,
    pub total_balance_gwei: u64,
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

    pub fn upsert_address(&mut self, alias: &str, address: Address) -> Result<()> {
        self.conn.execute(
            "INSERT INTO addresses (address, alias) VALUES (?1, ?2)
             ON CONFLICT(address) DO UPDATE SET alias = excluded.alias",
            params![addr_to_db(address), alias],
        )?;
        Ok(())
    }

    pub fn last_synced_block(&self, address: Address, chain: Chain) -> Result<u64> {
        let block: Option<i64> = self
            .conn
            .query_row(
                "SELECT last_block FROM sync_state WHERE address = ?1 AND chain_id = ?2",
                params![addr_to_db(address), chain.id() as i64],
                |row| row.get(0),
            )
            .optional()?;
        Ok(block.map(|b| b.max(0) as u64).unwrap_or(0))
    }

    pub fn record_sync(
        &mut self,
        chain: Chain,
        address: Address,
        txs: &[EtherscanTx],
        token_txs: &[EtherscanTokenTx],
    ) -> Result<SyncSummary> {
        let tx_db = self.conn.transaction()?;
        let chain_id = chain.id() as i64;

        for t in txs {
            let success = t.is_error == "0" && t.receipt_status != "0";
            tx_db.execute(
                "INSERT OR REPLACE INTO transactions
                   (chain_id, hash, block_number, timestamp, from_addr, to_addr,
                    value_wei, input_len, status, is_stub)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
                params![
                    chain_id,
                    t.hash,
                    t.block_number as i64,
                    t.timestamp as i64,
                    t.from.to_lowercase(),
                    if t.to.is_empty() {
                        None
                    } else {
                        Some(t.to.to_lowercase())
                    },
                    t.value,
                    input_len(&t.input) as i64,
                    if success { 1 } else { 0 },
                ],
            )?;
        }

        let mut log_counter: BTreeMap<String, u64> = BTreeMap::new();
        for tt in token_txs {
            let log_index = {
                let entry = log_counter.entry(tt.hash.clone()).or_insert(0);
                let v = *entry;
                *entry += 1;
                v
            };

            // Stub the tx if txlist didn't already cover it.
            tx_db.execute(
                "INSERT OR IGNORE INTO transactions
                   (chain_id, hash, block_number, timestamp, from_addr, to_addr,
                    value_wei, input_len, status, is_stub)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, '0', 0, 1, 1)",
                params![
                    chain_id,
                    tt.hash,
                    tt.block_number as i64,
                    tt.timestamp as i64,
                    tt.from.to_lowercase(),
                    tt.to.to_lowercase(),
                ],
            )?;

            tx_db.execute(
                "INSERT OR REPLACE INTO transfers
                   (chain_id, tx_hash, log_index, token, from_addr, to_addr,
                    amount, token_symbol, token_decimals)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    chain_id,
                    tt.hash,
                    log_index as i64,
                    tt.contract_address.to_lowercase(),
                    tt.from.to_lowercase(),
                    tt.to.to_lowercase(),
                    tt.value,
                    tt.token_symbol,
                    tt.token_decimal as i64,
                ],
            )?;
        }

        let highest = txs
            .iter()
            .map(|t| t.block_number)
            .chain(token_txs.iter().map(|t| t.block_number))
            .max();

        if let Some(block) = highest {
            tx_db.execute(
                "INSERT INTO sync_state (address, chain_id, last_block, updated_at)
                 VALUES (?1, ?2, ?3, strftime('%s','now'))
                 ON CONFLICT(address, chain_id) DO UPDATE SET
                   last_block = max(last_block, excluded.last_block),
                   updated_at = strftime('%s','now')",
                params![addr_to_db(address), chain_id, block as i64],
            )?;
        }

        tx_db.commit()?;

        Ok(SyncSummary {
            tx_count: txs.len(),
            transfer_count: token_txs.len(),
            highest_block: highest.unwrap_or(0),
        })
    }

    pub fn read_staking_snapshot(
        &self,
        address: Address,
        max_age: std::time::Duration,
    ) -> Result<Option<CachedStakingSummary>> {
        let row: Option<(i64, i64, i64)> = self
            .conn
            .query_row(
                "SELECT validator_count, total_balance_gwei, fetched_at \
                   FROM staking_snapshot WHERE address = ?1",
                params![addr_to_db(address)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((count, total, fetched_at)) = row else {
            return Ok(None);
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let age_secs = (now - fetched_at).max(0) as u64;
        if age_secs > max_age.as_secs() {
            return Ok(None);
        }
        Ok(Some(CachedStakingSummary {
            validator_count: count.max(0) as u64,
            total_balance_gwei: total.max(0) as u64,
        }))
    }

    pub fn upsert_staking_snapshot(
        &mut self,
        address: Address,
        validator_count: u64,
        total_balance_gwei: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO staking_snapshot \
               (address, validator_count, total_balance_gwei, fetched_at) \
             VALUES (?1, ?2, ?3, strftime('%s','now')) \
             ON CONFLICT(address) DO UPDATE SET \
               validator_count    = excluded.validator_count, \
               total_balance_gwei = excluded.total_balance_gwei, \
               fetched_at         = excluded.fetched_at",
            params![
                addr_to_db(address),
                validator_count as i64,
                total_balance_gwei as i64,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_label(
        &mut self,
        chain: Chain,
        address: Address,
        label: &str,
        kind: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO labels (chain_id, address, label, kind)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chain_id, address) DO UPDATE SET
               label = excluded.label,
               kind  = excluded.kind",
            params![chain.id() as i64, addr_to_db(address), label, kind],
        )?;
        Ok(())
    }

    pub fn unknown_counterparties(
        &self,
        addresses: &[Address],
        chain_filter: Option<Chain>,
    ) -> Result<Vec<UnknownCounterparty>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let owned: HashSet<String> = addresses.iter().copied().map(addr_to_db).collect();
        let addr_strs: Vec<String> = owned.iter().cloned().collect();
        let n = addr_strs.len();
        let in_clause = vec!["?"; n].join(",");

        let mut sql = format!(
            "SELECT chain_id, to_addr AS counterparty FROM transactions
               WHERE to_addr IS NOT NULL
                 AND from_addr IN ({in_clause})
             UNION ALL
             SELECT chain_id,
                    CASE WHEN from_addr IN ({in_clause}) THEN to_addr ELSE from_addr END
                      AS counterparty
               FROM transfers
               WHERE from_addr IN ({in_clause}) OR to_addr IN ({in_clause})"
        );
        if let Some(c) = chain_filter {
            sql = format!("SELECT * FROM ({sql}) WHERE chain_id = {}", c.id() as i64);
        }
        sql = format!(
            "SELECT chain_id, counterparty, COUNT(*) FROM ({sql}) \
             WHERE counterparty IS NOT NULL \
             GROUP BY chain_id, counterparty"
        );

        let mut all_params: Vec<&String> = Vec::with_capacity(4 * n);
        for _ in 0..4 {
            all_params.extend(addr_strs.iter());
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(all_params.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (chain_id, addr_str, count) = row?;
            if owned.contains(&addr_str) {
                continue;
            }
            if let Ok(addr) = addr_str.parse::<Address>() {
                out.push(UnknownCounterparty {
                    chain_id: chain_id as u64,
                    address: addr,
                    interactions: count as u64,
                });
            }
        }
        out.sort_by(|a, b| {
            b.interactions
                .cmp(&a.interactions)
                .then(a.chain_id.cmp(&b.chain_id))
        });
        Ok(out)
    }

    pub fn list_labels(&self) -> Result<BTreeMap<(u64, Address), String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT chain_id, address, label FROM labels")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (chain_id, addr_str, label) = row?;
            if let Ok(addr) = addr_str.parse::<Address>() {
                out.insert((chain_id as u64, addr), label);
            }
        }
        Ok(out)
    }

    pub fn query_history(
        &self,
        registry: &Registry,
        addresses: &[Address],
        chain_filter: Option<Chain>,
    ) -> Result<Vec<DecodedTx>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let addr_strs: Vec<String> = addresses.iter().copied().map(addr_to_db).collect();
        let n = addr_strs.len();
        let in_clause = vec!["?"; n].join(",");

        let mut sql = format!(
            "SELECT chain_id, hash FROM transactions
               WHERE from_addr IN ({in_clause}) OR to_addr IN ({in_clause})
             UNION
             SELECT chain_id, tx_hash AS hash FROM transfers
               WHERE from_addr IN ({in_clause}) OR to_addr IN ({in_clause})"
        );
        if let Some(c) = chain_filter {
            sql = format!("SELECT * FROM ({sql}) WHERE chain_id = {}", c.id() as i64);
        }

        let mut all_params: Vec<&String> = Vec::with_capacity(4 * n);
        for _ in 0..4 {
            all_params.extend(addr_strs.iter());
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(all_params.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;

        let mut hashes: HashSet<(i64, String)> = HashSet::new();
        for row in rows {
            hashes.insert(row?);
        }

        let mut decoded: Vec<DecodedTx> = Vec::with_capacity(hashes.len());
        for (chain_id, hash) in &hashes {
            let raw_tx = self.fetch_tx(*chain_id, hash)?;
            let transfers = self.fetch_transfers(*chain_id, hash)?;
            let us_for_this_tx = pick_us(addresses, &raw_tx, &transfers).unwrap_or(addresses[0]);
            let dec = registry.decode_tx(us_for_this_tx, &raw_tx, &transfers);
            decoded.push(dec);
        }

        decoded.sort_by_key(|d| (d.timestamp, d.chain_id, d.hash.clone()));
        Ok(decoded)
    }

    fn fetch_tx(&self, chain_id: i64, hash: &str) -> Result<RawTx> {
        let row = self.conn.query_row(
            "SELECT timestamp, from_addr, to_addr, value_wei, input_len, status
               FROM transactions WHERE chain_id = ?1 AND hash = ?2",
            params![chain_id, hash],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )?;
        let (timestamp, from_s, to_s, value_s, input_len, status) = row;
        let from = from_s.parse::<Address>().context("parsing tx from")?;
        let to = to_s
            .map(|s| s.parse::<Address>().context("parsing tx to"))
            .transpose()?;
        let value_wei = value_s.parse::<U256>().context("parsing tx value")?;
        Ok(RawTx {
            chain_id: chain_id as u64,
            hash: hash.to_string(),
            timestamp: timestamp as u64,
            from,
            to,
            value_wei,
            input_len: input_len as usize,
            success: status != 0,
        })
    }

    fn fetch_transfers(&self, chain_id: i64, tx_hash: &str) -> Result<Vec<RawTransfer>> {
        let mut stmt = self.conn.prepare(
            "SELECT token, from_addr, to_addr, amount, token_symbol, token_decimals
               FROM transfers WHERE chain_id = ?1 AND tx_hash = ?2 ORDER BY log_index",
        )?;
        let rows = stmt.query_map(params![chain_id, tx_hash], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (token_s, from_s, to_s, amount_s, symbol, decimals) = row?;
            out.push(RawTransfer {
                token: token_s.parse().context("parsing transfer token")?,
                from: from_s.parse().context("parsing transfer from")?,
                to: to_s.parse().context("parsing transfer to")?,
                amount: amount_s.parse().context("parsing transfer amount")?,
                symbol,
                decimals: decimals as u32,
            });
        }
        Ok(out)
    }
}

fn addr_to_db(a: Address) -> String {
    format!("{a:#x}")
}

fn input_len(input_hex: &str) -> usize {
    let h = input_hex.strip_prefix("0x").unwrap_or(input_hex);
    h.len() / 2
}

fn pick_us(addresses: &[Address], tx: &RawTx, transfers: &[RawTransfer]) -> Option<Address> {
    for &a in addresses {
        if tx.from == a || tx.to == Some(a) {
            return Some(a);
        }
        for t in transfers {
            if t.from == a || t.to == a {
                return Some(a);
            }
        }
    }
    None
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

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "lportfolio-test-{}-{}.sqlite",
            std::process::id(),
            tag
        ))
    }

    #[test]
    fn opens_and_applies_schema_in_tempfile() {
        let tmp = tmp_path("schema");
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

    #[test]
    fn last_synced_block_returns_zero_when_unset() {
        let tmp = tmp_path("lastblock");
        let _ = std::fs::remove_file(&tmp);
        let db = Db::open_at(&tmp).unwrap();
        let addr: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        assert_eq!(db.last_synced_block(addr, Chain::Mainnet).unwrap(), 0);
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn record_sync_inserts_rows_and_advances_block() {
        let tmp = tmp_path("recordsync");
        let _ = std::fs::remove_file(&tmp);
        let mut db = Db::open_at(&tmp).unwrap();

        let addr: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        db.upsert_address("alice", addr).unwrap();

        let txs = vec![EtherscanTx {
            block_number: 100,
            timestamp: 1_700_000_000,
            hash: "0xaaa".into(),
            from: "0x0000000000000000000000000000000000000001".into(),
            to: "0x0000000000000000000000000000000000000002".into(),
            value: "1000000000000000000".into(),
            input: "0x".into(),
            receipt_status: "1".into(),
            is_error: "0".into(),
        }];
        let token_txs = vec![EtherscanTokenTx {
            block_number: 110,
            timestamp: 1_700_000_100,
            hash: "0xbbb".into(),
            contract_address: "0x000000000000000000000000000000000000abcd".into(),
            from: "0x0000000000000000000000000000000000000003".into(),
            to: "0x0000000000000000000000000000000000000001".into(),
            value: "1000000".into(),
            token_symbol: "USDC".into(),
            token_decimal: 6,
        }];

        let summary = db
            .record_sync(Chain::Mainnet, addr, &txs, &token_txs)
            .unwrap();
        assert_eq!(summary.tx_count, 1);
        assert_eq!(summary.transfer_count, 1);
        assert_eq!(summary.highest_block, 110);
        assert_eq!(db.last_synced_block(addr, Chain::Mainnet).unwrap(), 110);

        let registry = Registry::default_set();
        let history = db
            .query_history(&registry, &[addr], Some(Chain::Mainnet))
            .unwrap();
        assert_eq!(history.len(), 2);
        std::fs::remove_file(&tmp).ok();
    }
}
