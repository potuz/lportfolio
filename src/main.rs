#![forbid(unsafe_code)]

mod chain;
mod config;
mod csm;
mod db;
mod decode;
mod explorer;
mod holdings;
mod interactive;
mod render;
mod rpc;
mod staking;
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
    /// Show current balances and overall portfolio total.
    Holdings {
        /// Bypass the staking-snapshot cache and re-fetch from beaconcha.in.
        #[arg(long)]
        refresh: bool,
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
    /// Print a shell-completion script. Usage:
    ///   lportfolio completions bash > ~/.local/share/bash-completion/completions/lportfolio
    Completions {
        #[arg(value_enum, default_value = "bash")]
        shell: clap_complete::Shell,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    install_pipe_friendly_panic_hook();
    init_tracing();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Chains => chains_cmd(),
        Cmd::Holdings { refresh } => holdings_cmd(refresh).await,
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
        Cmd::Completions { shell } => completions_cmd(shell),
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

/// `println!` panics on EPIPE; when output is piped to `head`/`less` and the
/// reader exits early, that becomes a noisy backtrace. Translate "broken pipe"
/// panics into a clean exit while leaving other panics alone.
fn install_pipe_friendly_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&'static str>()
            .copied()
            .map(str::to_owned)
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        if msg.contains("Broken pipe") || msg.contains("os error 32") {
            std::process::exit(0);
        }
        default(info);
    }));
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

async fn holdings_cmd(refresh: bool) -> Result<()> {
    let cfg = Config::load()?;
    let mut db = Db::open()?;
    let snap = holdings::build_snapshot(&cfg, &mut db, refresh).await?;

    if !snap.native.is_empty() {
        render::print_section("Native balances");
        println!("{}", render::render_native(&snap.native));
    }

    if !snap.staking.is_empty() {
        render::print_section("Beacon staking");
        println!("{}", render::render_staking(&snap.staking));
    }

    if !snap.csm.is_empty() {
        render::print_section("Lido CSM bonds");
        println!("{}", render::render_csm(&snap.csm));
    }

    render::print_section("Summary");
    println!("{}", render::render_summary(&snap));
    render::print_grand_total(&snap);
    Ok(())
}

fn completions_cmd(shell: clap_complete::Shell) -> Result<()> {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
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
