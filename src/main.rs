#![forbid(unsafe_code)]

mod chain;
mod config;
mod db;
mod decode;
mod explorer;
mod interactive;
mod render;
mod rpc;
mod sync;

use std::collections::BTreeMap;

use alloy::primitives::Address;
use anyhow::Result;
use clap::{Parser, Subcommand};
use comfy_table::Table;

use crate::chain::Chain;
use crate::config::Config;
use crate::db::Db;
use crate::decode::Registry;
use crate::explorer::Explorer;
use crate::rpc::{ChainClient, format_eth};

#[derive(Parser)]
#[command(
    name = "lportfolio",
    version,
    about = "Local Ethereum portfolio tracker"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List configured chains and whether they have an RPC URL set.
    Chains,
    /// Pull new transaction history into the local cache.
    Sync {
        #[arg(long)]
        chain: Option<Chain>,
        #[arg(long)]
        address: Option<String>,
    },
    /// Show current balances.
    Holdings {
        #[arg(long)]
        chain: Option<Chain>,
    },
    /// Show decoded transaction history.
    History {
        #[arg(long)]
        address: Option<String>,
        #[arg(long)]
        chain: Option<Chain>,
        #[arg(long)]
        since: Option<String>,
    },
    /// Tag an address with a label.
    Tag {
        address: String,
        label: String,
        #[arg(long)]
        chain: Option<Chain>,
        #[arg(long, default_value = "contract")]
        kind: String,
    },
    /// List counterparties with no label and no decoder coverage.
    /// In a TTY, prompts to label each one.
    Unknowns {
        #[arg(long)]
        chain: Option<Chain>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Chains => chains_cmd(),
        Cmd::Holdings { chain } => holdings_cmd(chain).await,
        Cmd::Sync { chain, address } => sync_cmd(chain, address).await,
        Cmd::History {
            address,
            chain,
            since,
        } => history_cmd(address, chain, since),
        Cmd::Tag {
            address,
            label,
            chain,
            kind,
        } => tag_cmd(address, label, chain, kind),
        Cmd::Unknowns { chain } => unknowns_cmd(chain),
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn chains_cmd() -> Result<()> {
    let cfg = Config::load()?;

    println!(
        "{} address(es) configured, etherscan API key: {}",
        cfg.addresses.len(),
        if cfg.etherscan_api_key.is_some() {
            "set"
        } else {
            "not set"
        },
    );

    let mut addr_table = Table::new();
    addr_table.set_header(vec!["Alias", "Address"]);
    for (alias, addr) in &cfg.addresses {
        addr_table.add_row(vec![alias.clone(), format!("{addr:#x}")]);
    }
    println!("{addr_table}");

    let mut chain_table = Table::new();
    chain_table.set_header(vec!["Chain", "Chain ID", "Status", "RPC URL"]);
    for chain in Chain::ALL {
        let (status, url) = match cfg.chains.get(chain) {
            Some(cc) => ("configured", cc.rpc_url.as_str()),
            None => ("(not configured)", ""),
        };
        chain_table.add_row(vec![
            chain.name().to_string(),
            chain.id().to_string(),
            status.to_string(),
            url.to_string(),
        ]);
    }
    println!("{chain_table}");
    Ok(())
}

async fn holdings_cmd(chain_filter: Option<Chain>) -> Result<()> {
    let cfg = Config::load()?;
    let _db = Db::open()?;

    let selected: Vec<(Chain, String)> = cfg
        .chains
        .iter()
        .filter(|(c, _)| chain_filter.is_none_or(|f| f == **c))
        .map(|(c, cc)| (*c, cc.rpc_url.clone()))
        .collect();

    if selected.is_empty() {
        anyhow::bail!("no chains configured (or none match --chain filter)");
    }

    let mut clients = Vec::with_capacity(selected.len());
    for (chain, url) in selected {
        let client = ChainClient::connect(chain, &url)?;
        client.verify_chain_id().await?;
        clients.push(client);
    }

    let mut table = Table::new();
    table.set_header(vec!["Alias", "Address", "Chain", "Native Balance"]);
    for (alias, addr) in &cfg.addresses {
        for client in &clients {
            let balance = client.balance(*addr).await?;
            table.add_row(vec![
                alias.clone(),
                format!("{addr:#x}"),
                client.chain().name().to_string(),
                format_eth(balance),
            ]);
        }
    }
    println!("{table}");
    Ok(())
}

async fn sync_cmd(chain_filter: Option<Chain>, alias_filter: Option<String>) -> Result<()> {
    let cfg = Config::load()?;
    let api_key = cfg
        .etherscan_api_key
        .clone()
        .ok_or_else(|| anyhow::anyhow!("LPORTFOLIO_ETHERSCAN_API_KEY is required for sync"))?;
    let explorer = Explorer::new(api_key)?;
    let mut db = Db::open()?;

    let chains: Vec<Chain> = cfg
        .chains
        .keys()
        .copied()
        .filter(|c| chain_filter.is_none_or(|f| f == *c))
        .collect();
    if chains.is_empty() {
        anyhow::bail!("no chains configured (or none match --chain filter)");
    }

    let addresses: Vec<(String, Address)> = cfg
        .addresses
        .iter()
        .filter(|(alias, _)| alias_filter.as_deref().is_none_or(|f| *alias == f))
        .map(|(a, addr)| (a.clone(), *addr))
        .collect();
    if addresses.is_empty() {
        anyhow::bail!("no addresses match --address filter");
    }

    let mut table = Table::new();
    table.set_header(vec![
        "Alias",
        "Chain",
        "New txs",
        "New transfers",
        "Highest block",
    ]);
    for (alias, addr) in &addresses {
        for chain in &chains {
            let summary = sync::sync_address(&mut db, &explorer, *chain, alias, *addr).await?;
            table.add_row(vec![
                alias.clone(),
                chain.name().to_string(),
                summary.tx_count.to_string(),
                summary.transfer_count.to_string(),
                if summary.highest_block == 0 {
                    "-".to_string()
                } else {
                    summary.highest_block.to_string()
                },
            ]);
        }
    }
    println!("{table}");
    Ok(())
}

fn history_cmd(
    alias_filter: Option<String>,
    chain_filter: Option<Chain>,
    since: Option<String>,
) -> Result<()> {
    if since.is_some() {
        anyhow::bail!("--since is not yet implemented");
    }
    let cfg = Config::load()?;
    let db = Db::open()?;

    let addresses: Vec<Address> = cfg
        .addresses
        .iter()
        .filter(|(alias, _)| alias_filter.as_deref().is_none_or(|f| *alias == f))
        .map(|(_, addr)| *addr)
        .collect();
    if addresses.is_empty() {
        anyhow::bail!("no addresses match --address filter");
    }

    let aliases: BTreeMap<Address, String> = cfg
        .addresses
        .iter()
        .map(|(alias, addr)| (*addr, alias.clone()))
        .collect();
    let registry = Registry::default_set();
    let mut labels = db.list_labels()?;
    for known in registry.known_labels() {
        labels
            .entry((known.chain_id, known.address))
            .or_insert_with(|| known.label.to_string());
    }
    let history = db.query_history(&registry, &addresses, chain_filter)?;

    if history.is_empty() {
        println!("(no history; run `lportfolio sync` first)");
        return Ok(());
    }

    let table = render::render_history(&history, &aliases, &labels);
    println!("{table}");
    Ok(())
}

fn tag_cmd(address: String, label: String, chain: Option<Chain>, kind: String) -> Result<()> {
    let chain = chain.unwrap_or(Chain::Mainnet);
    let addr: Address = address
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid address {address:?}: {e}"))?;
    let mut db = Db::open()?;
    db.upsert_label(chain, addr, &label, &kind)?;
    println!("tagged {addr:#x} on {} as {label:?} ({kind})", chain.name());
    Ok(())
}

fn unknowns_cmd(chain_filter: Option<Chain>) -> Result<()> {
    let cfg = Config::load()?;
    let mut db = Db::open()?;
    let registry = Registry::default_set();

    let owned: Vec<Address> = cfg.addresses.values().copied().collect();
    if owned.is_empty() {
        anyhow::bail!("no addresses configured");
    }

    let counterparties = db.unknown_counterparties(&owned, chain_filter)?;
    let known_labels = db.list_labels()?;
    let registry_known: std::collections::HashSet<(u64, Address)> = registry
        .known_labels()
        .into_iter()
        .map(|k| (k.chain_id, k.address))
        .collect();

    let unlabeled: Vec<_> = counterparties
        .into_iter()
        .filter(|c| {
            !known_labels.contains_key(&(c.chain_id, c.address))
                && !registry_known.contains(&(c.chain_id, c.address))
        })
        .collect();

    if unlabeled.is_empty() {
        println!("No unlabeled counterparties.");
        return Ok(());
    }

    match interactive::prompt_unknowns(&mut db, &unlabeled)? {
        Some(outcome) => {
            println!(
                "\ndone: {} tagged, {} skipped, {} remaining",
                outcome.tagged,
                outcome.skipped,
                unlabeled.len() - outcome.tagged - outcome.skipped,
            );
        }
        None => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["Chain", "Address", "Interactions"]);
            for u in &unlabeled {
                let chain = Chain::from_id(u.chain_id)
                    .map(|c| c.name().to_string())
                    .unwrap_or_else(|| u.chain_id.to_string());
                table.add_row(vec![
                    chain,
                    format!("{:#x}", u.address),
                    u.interactions.to_string(),
                ]);
            }
            println!("{table}");
            println!(
                "\n{} unlabeled counterparties. Run `lportfolio tag <address> <label>` to label.",
                unlabeled.len()
            );
        }
    }
    Ok(())
}
