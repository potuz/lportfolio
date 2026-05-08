# CLAUDE.md

Guidance for Claude Code sessions working in this repo.

## What this project is

`lportfolio` — a local CLI Ethereum portfolio tracker, written in Rust. It
ingests a list of user-owned addresses and chains (mainnet + L2s) from a `.env`,
fetches current holdings via JSON-RPC and historical transactions via block
explorers (Etherscan v2 unified API), persists everything to a local SQLite
cache, and renders holdings + decoded transaction history as terminal tables.

## Hard constraints

These are user-imposed and non-negotiable without asking first:

- **Dependencies are kept to an absolute minimum and exact-pinned.** Use
  `= "x.y.z"` in `Cargo.toml` (not `^` or `~`). Commit `Cargo.lock`. The
  motivation is supply-chain attack surface — every new crate must be
  justified.
- **No `unsafe`.** `#![forbid(unsafe_code)]` lives at the crate root.
- **Secrets stay in `.env`** (gitignored). Never log addresses' API keys, never
  commit a real `.env`. `.env.example` is the template.
- **Build with `--locked`.** `cargo build --locked` and `cargo test --locked`
  are the canonical commands; CI must fail if `Cargo.lock` would change.

## Repository layout

```
src/
  main.rs            clap entry, subcommand wiring
  config.rs          .env loader (addresses, chains, RPCs, API keys, CSM op IDs)
  chain.rs           Chain enum + chain metadata
  rpc.rs             ChainClient + JSON-RPC implementation
  explorer.rs        Etherscan v2 client (throttled + retries)
  staking.rs         beaconcha.in client (throttled + retries)
  csm.rs             Lido CSM bond reader (alloy::sol! ABI)
  holdings.rs        PortfolioSnapshot aggregator + build_snapshot()
  db.rs              rusqlite schema, migrations, queries
  sync.rs            incremental sync orchestration
  render.rs          comfy-table rendering + paint module (ANSI colors)
  interactive.rs     unknown-contract tagging flow
  decode/
    mod.rs           ContractDecoder trait + Registry
    erc20.rs         generic ERC-20 transfers (default fallback)
    lido.rs aave.rs uniswap.rs cowswap.rs across.rs splits.rs
```

## Architectural conventions

- **All network I/O is behind a trait** (`ChainClient`, `Explorer`). Tests use
  recorded fixtures, never live network.
- **Decoders are pluggable.** `ContractDecoder::matches(chain_id, to)` +
  `decode(tx, logs)`. The `Registry` walks decoders in order; the first match
  wins. Add a new protocol by adding a file under `src/decode/` and registering
  it.
- **Local node vs explorer split.** Current-state reads (balances, contract
  views) go through `ChainClient` and can hit a user's local node. Historical
  reads (tx lists, logs older than the node's pruning window) go through
  `Explorer`. Don't collapse this distinction.
- **anyhow at boundaries, thiserror in libs.** `main.rs` and subcommand
  handlers return `anyhow::Result`. Internal modules expose typed errors.

## Environment variables

Loaded by `config.rs` from `.env` (gitignored).

- `LPORTFOLIO_ADDRESSES` — required; comma-separated `alias=0xaddr` pairs.
- `LPORTFOLIO_RPC_{MAINNET,ARBITRUM,OPTIMISM,BASE}` — RPC URLs per chain.
- `LPORTFOLIO_ETHERSCAN_API_KEY` — required for `sync` / `history`. One key
  works across all chains via Etherscan v2 unified.
- `LPORTFOLIO_BEACONCHAIN_API_KEY` — required to enable beacon staking
  in `holdings` (free tier requires a key as of 2026). Without it, the
  staking section is silently skipped.
- `LPORTFOLIO_LIDO_CSM_OPERATOR_IDS` — comma-separated u64s. Empty/unset
  → CSM section omitted.

## Database

SQLite, opened at an XDG path (`dirs::data_dir().join("lportfolio/db.sqlite")`).
Schema is applied at startup from embedded SQL strings — no migration framework.
Tables: `addresses`, `chains`, `sync_state`, `transactions`, `transfers`,
`labels`, `holdings_snapshot`, `staking_snapshot`.

When changing the schema, add a new `CREATE TABLE IF NOT EXISTS` / `ALTER TABLE`
block guarded by a `schema_version` row — never edit a previous migration in
place.

## CLI surface

```
lportfolio chains                                       # list configured chains
lportfolio sync         [--chain <id>] [--address <alias>]
lportfolio holdings     [--refresh]                     # native + staking + CSM + total
lportfolio history      [--address <alias>] [--chain <id>] [--since <date>]
lportfolio tag          <address> <label> [--chain <c>] [--kind <eoa|contract|protocol>]
lportfolio unknowns     [--chain <id>]                  # interactive tagging in TTY
lportfolio completions  [bash]                          # print shell completions
```

Install completions:
```
lportfolio completions bash > ~/.local/share/bash-completion/completions/lportfolio
```

## Common commands

```
cargo build --locked
cargo test  --locked
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo audit
cargo deny check
```

Run these before declaring a change complete. Clippy warnings are errors here.

## When adding a new protocol decoder

1. Create `src/decode/<protocol>.rs` implementing `ContractDecoder`.
2. Register it in `src/decode/mod.rs::Registry::default()`.
3. Add a fixture-based test: capture the relevant tx + receipt JSON into
   `tests/fixtures/<protocol>/`, write a test that loads it and asserts the
   decoded action.
4. Update `labels` defaults if the protocol has well-known contract addresses
   that should be pre-tagged.

## When the user runs into an unknown contract

The interactive flow (`interactive.rs`) prompts for a label and optional
protocol tag, persists to `labels`, and surfaces a hint about adding a decoder.
Non-interactive runs (`--no-prompt` / no TTY) skip the prompt and record the
address as `kind = "contract"` with no label.

## ANSI color helpers

`render::paint` exposes `bold`, `dim`, `cyan`, `bold_green`, and `header`
helpers. Each emits raw ANSI escapes only when `stdout().is_terminal()` is
true (cached via `OnceLock`); piped output stays escape-free. **Do not add a
color crate** — the hand-rolled escapes are intentional to keep the dep set
minimal.

## Things to avoid

- Adding a dep without justifying it against the minimal set above.
- Using `^` / `~` / unpinned versions in `Cargo.toml`.
- Live network calls in tests.
- Logging API keys, full `.env` contents, or unredacted RPC URLs (some include
  the key in the path).
- Schema changes that mutate existing migrations rather than adding new ones.
- Introducing async runtimes other than `tokio`.
- Using `println!` directly when output may be very large — consider that the
  pipe-friendly panic hook in `main.rs` only catches broken pipes, not other
  large-output issues.
