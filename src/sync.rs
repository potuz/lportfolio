use alloy::primitives::Address;
use anyhow::Result;
use tracing::info;

use crate::chain::Chain;
use crate::db::{Db, SyncSummary};
use crate::explorer::Explorer;

pub async fn sync_address(
    db: &mut Db,
    explorer: &Explorer,
    chain: Chain,
    alias: &str,
    address: Address,
) -> Result<SyncSummary> {
    db.upsert_address(alias, address)?;

    let last_block = db.last_synced_block(address, chain)?;
    let start = last_block.saturating_add(1);
    let address_hex = format!("{address:#x}");

    info!(chain = chain.name(), alias, start, "fetching txlist");
    let txs = explorer.txlist(chain, &address_hex, start).await?;
    info!(chain = chain.name(), alias, start, "fetching tokentx");
    let token_txs = explorer.tokentx(chain, &address_hex, start).await?;

    let summary = db.record_sync(chain, address, &txs, &token_txs)?;
    info!(
        chain = chain.name(),
        alias,
        new_txs = summary.tx_count,
        new_transfers = summary.transfer_count,
        highest_block = summary.highest_block,
        "sync complete"
    );
    Ok(summary)
}
