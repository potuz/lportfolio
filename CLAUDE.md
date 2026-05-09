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
  main.rs            clap entry, subcommand wiring, pipe-friendly panic hook
  config.rs          .env loader (addresses, chains, RPCs, beacon, validators,
                     CSM op IDs, token whitelist)
  chain.rs           Chain enum + chain metadata
  rpc.rs             ChainClient: native balance, ERC-20 balanceOf, retries
  explorer.rs        Etherscan v2 client (throttled + retries)
  staking.rs         BeaconNodeClient — direct Beacon API call to local node
  csm.rs             Lido CSM bond reader (alloy::sol! ABI on CSAccounting)
  splits.rs          Splits V2 claimable balance reader
                     (warehouse.balanceOf for native + whitelisted ERC-20s)
  tokens.rs          Hardcoded ERC-20 whitelist registry: (whitelist_id,
                     chain, address, display_symbol, decimals)
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

- **All network I/O is behind a small struct/trait** (`ChainClient`,
  `Explorer`, `BeaconNodeClient`, `CsmReader`, `SplitsReader`). Tests use
  recorded fixtures, never live network.
- **`decode/splits.rs` vs `splits.rs` are intentionally separate.** The
  former is a transaction-history decoder (matches by contract address in
  past txs); the latter is a current-state reader (`eth_call` on
  SplitsWarehouse for claimable balances). Same protocol, two read paths.
- **Decoders are pluggable.** `ContractDecoder::matches(chain_id, to)` +
  `decode(tx, logs)`. The `Registry` walks decoders in order; the first match
  wins. Add a new protocol by adding a file under `src/decode/` and registering
  it.
- **Local node vs explorer split.** Current-state reads (native balances,
  ERC-20 `balanceOf`, CSM bond) go through `ChainClient` and can hit a user's
  local node — non-archive is fine. Historical reads (tx lists, logs older
  than the node's pruning window) go through `Explorer`. Don't collapse this
  distinction.
- **anyhow at boundaries, thiserror in libs.** `main.rs` and subcommand
  handlers return `anyhow::Result`. Internal modules expose typed errors.
  Log anyhow errors with `{:#}` (Display alternate) so the full cause chain
  surfaces; piping to `error = %e` only shows the top-level context.
- **RPC retries are centralized** in `ChainClient::with_retry` —
  exponential backoff (1s/2s/4s) on any transport error, then propagated.
  Holdings degrades gracefully: if one (alias, chain) ultimately fails its
  row is omitted rather than aborting the whole table.

## Environment variables

Loaded by `config.rs` from `.env` (gitignored).

- `LPORTFOLIO_ADDRESSES` — required; comma-separated `alias=0xaddr` pairs.
- `LPORTFOLIO_RPC_{MAINNET,ARBITRUM,OPTIMISM,BASE}` — RPC URLs per chain.
  Non-archive is fine for `holdings`; `sync` doesn't use these (Etherscan
  does the historical lookup).
- `LPORTFOLIO_ETHERSCAN_API_KEY` — required for `sync` / `history`. One key
  works across all chains via Etherscan v2 unified.
- `LPORTFOLIO_BEACON_URL` — local Beacon API endpoint (e.g.
  `http://localhost:5052`). Empty/unset → staking section omitted.
- `LPORTFOLIO_VALIDATOR_INDICES` — comma-separated u64s. Required when
  `BEACON_URL` is set. We hit
  `GET {BEACON_URL}/eth/v1/beacon/states/head/validators?id=…` and sum the
  `balance` field. Result is cached 5 min in `staking_snapshot`;
  `holdings --refresh` bypasses.
- `LPORTFOLIO_LIDO_CSM_OPERATOR_IDS` — comma-separated u64s. Empty/unset
  → CSM section omitted. Reads `getBond(operatorId)` from
  CSAccounting (`0x4d72…e5Da`) on mainnet.
- `LPORTFOLIO_TOKEN_WHITELIST` — comma-separated logical token IDs from
  `tokens::REGISTRY`. Empty/unset → only native ETH in `holdings`.
  Currently registered: `USDC, USDT, ARB, DAI` (plus per-chain
  variants like `USDT0` and `USDC.e` on Arbitrum).
- `LPORTFOLIO_SAFES` — comma-separated subset of aliases (from
  `LPORTFOLIO_ADDRESSES`) that are Gnosis Safe contracts. Listed aliases
  get a `(Safe)` badge in the holdings table. Balance fetching is
  identical to EOAs — `eth_getBalance` and `balanceOf` work for any
  address. The flag is purely for visual labeling.

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
lportfolio holdings     [--refresh]                     # native + ERC-20 + staking + CSM + total
lportfolio history      [--address <alias>] [--chain <id>] [--since <date>]
lportfolio tag          <address> <label> [--chain <c>] [--kind <eoa|contract|protocol>]
lportfolio unknowns     [--chain <id>]                  # interactive tagging in TTY
lportfolio completions  [bash]                          # print shell completions
```

Install completions:
```
lportfolio completions bash > ~/.local/share/bash-completion/completions/lportfolio
```

### Holdings layout

The native balances table pivots one row per (alias, address) with one
column per configured chain plus a Total column, ending in a `Total` row.
Each cell is multi-line: native ETH plus any whitelisted ERC-20 the user
holds on that chain. ETH renders at 4 decimals; tokens at 2 with values
below 0.01 omitted. Integer parts get thousands separators (`1,234.56`),
and amounts are right-aligned within each cell so the symbol column lines
up. Beacon staking and Lido CSM bonds are separate tables; the bottom
"Grand total" line is printed below the tables (ANSI-painted, outside the
table so cell-width math doesn't break).

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

`render::paint` exposes `bold`, `bold_green`, and `header` helpers. Each
emits raw ANSI escapes only when `stdout().is_terminal()` is true (cached
via `OnceLock`); piped output stays escape-free. **Do not add a color
crate** — the hand-rolled escapes are intentional to keep the dep set
minimal. **Do not put ANSI escapes inside table cells** — comfy-table sizes
columns by byte count and escape bytes break the layout. Color section
headers and post-table lines instead.

## Adding a new whitelisted ERC-20

1. Find the contract address(es) on each supported chain. Note any
   per-chain symbol differences (e.g. Arbitrum's bridged Tether displays
   `USDT0`, not `USDT`).
2. Add one `WhitelistedToken` entry per (chain, deployment) to
   `src/tokens.rs::REGISTRY` with the on-chain `display_symbol` and
   `decimals`.
3. The user adds the `whitelist_id` to `LPORTFOLIO_TOKEN_WHITELIST`.
   Multiple deployments per chain (e.g. `USDC` + `USDC.e` on Arbitrum)
   are supported — each becomes its own line in the cell.

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
