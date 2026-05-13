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

    // Internal txs are tracked under a separate watermark so existing users
    // (whose sync_state is already at HEAD for regular txs) still backfill
    // internals from genesis on the first run after this feature lands.
    let internal_start = db
        .last_internal_synced_block(address, chain)?
        .saturating_add(1);
    info!(
        chain = chain.name(),
        alias,
        start = internal_start,
        "fetching txlistinternal"
    );
    let internals = explorer
        .txlistinternal(chain, &address_hex, internal_start)
        .await?;
    let internal_inserted = db.record_internals(chain, address, &internals)?;

    info!(
        chain = chain.name(),
        alias,
        new_txs = summary.tx_count,
        new_transfers = summary.transfer_count,
        new_internals = internal_inserted,
        highest_block = summary.highest_block,
        "sync complete"
    );
    Ok(summary)
}
