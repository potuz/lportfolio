# lportfolio

> [!WARNING]
> Completely vibecoded — not a single line of code was read by a human.

A local Ethereum portfolio tracker. Ingests a list of user-owned addresses
across mainnet + a handful of L2s, queries JSON-RPC + a Beacon node + a
block explorer, persists everything to a local SQLite cache, and renders
the result as terminal tables on the desktop or as an egui-based GUI on
Android. Single-user, runs entirely against your own RPCs.

## Why

The hosted "portfolio tracker" sites need to see your addresses to work, and
some of them want a wallet connection. lportfolio reads the same on-chain
data via your own (or any) RPC endpoints and stores nothing remotely.

## What it tracks

- Native ETH balances on mainnet, Arbitrum, Optimism, Base
- Whitelisted ERC-20 balances on those chains (USDC, USDT, ARB, DAI, …)
- Beacon-chain validators (via a local Beacon API node)
- Lido CSM bonds (operator IDs you control)
- Splits V2 claimable balances on every supported chain
- USD totals via CoinGecko
- Decoded transaction history for tagged protocols (Lido, Aave, Uniswap,
  CowSwap, Across, Splits, generic ERC-20)

## Hard constraints

These are non-negotiable in this repo:

- **Dependencies kept minimal and exact-pinned.** Every crate uses
  `= "x.y.z"` in `Cargo.toml`. `Cargo.lock` is committed. New deps need
  justification — the goal is to keep the supply-chain attack surface small.
- **No `unsafe`.** The library uses `#![deny(unsafe_code)]` at its root;
  the CLI binary uses `#![forbid(unsafe_code)]`. The single Android-only
  carve-out is the `#[unsafe(no_mangle)]` on `android_main`, scoped to one
  file. No `unsafe` blocks anywhere.
- **Secrets stay in `.env`** (gitignored). `.env.example` is the template.
- **Always build with `--locked`.** CI must fail if `Cargo.lock` would change.

## Building

### Desktop CLI

```bash
cargo build  --locked --release
cargo test   --locked
cargo clippy --locked --all-targets -- -D warnings
```

The binary is at `target/release/lportfolio`.

### Android cdylib + APK

Prerequisites:

- Android SDK + NDK (NDK r27 verified). Install with `sdkmanager
  "ndk;27.1.12297006" "platforms;android-33" "build-tools;33.0.0"`.
- `aarch64-linux-android` Rust target: `rustup target add aarch64-linux-android`.
- xbuild for APK packaging: `cargo install --locked xbuild`.
- `lld`, `llvm` (toolchain helpers), `android-tools` (`adb`),
  `squashfs-tools` (xbuild needs `mksquashfs`).

Set environment for the NDK compiler so `rusqlite-bundled` builds:

```bash
export ANDROID_NDK_ROOT=/opt/android-sdk/ndk/27.1.12297006
export NDK_BIN=$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin
export CC_aarch64_linux_android=$NDK_BIN/aarch64-linux-android26-clang
export CXX_aarch64_linux_android=$NDK_BIN/aarch64-linux-android26-clang++
export AR_aarch64_linux_android=$NDK_BIN/llvm-ar
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$CC_aarch64_linux_android
```

Then:

```bash
# Cross-compile sanity check
cargo build --locked --release --no-default-features \
    --target aarch64-linux-android

# Package an APK
x build --platform android --arch arm64
```

The APK lands at `target/x/debug/android/lportfolio.apk` (~260 MB debug, ~30 MB
release). Sideload to a wired-or-wireless `adb`-paired device:

```bash
adb install -r target/x/debug/android/lportfolio.apk
```

The package id is `com.lportfolio.android`.

## Configuration

The CLI reads `.env` (or the process environment). The Android app reads a
TOML file written to its app-private storage. The two are equivalent.

### `.env` (CLI)

```
LPORTFOLIO_ADDRESSES=alias1=0x...,alias2=0x...
LPORTFOLIO_RPC_MAINNET=http://your-node:8545
LPORTFOLIO_RPC_ARBITRUM=https://arb1.arbitrum.io/rpc/...
LPORTFOLIO_RPC_OPTIMISM=
LPORTFOLIO_RPC_BASE=
LPORTFOLIO_ETHERSCAN_API_KEY=...
LPORTFOLIO_BEACON_URL=http://your-node:3500
LPORTFOLIO_VALIDATOR_INDICES=12345,12346
LPORTFOLIO_LIDO_CSM_OPERATOR_IDS=42
LPORTFOLIO_TOKEN_WHITELIST=USDC,USDT,ARB,DAI
LPORTFOLIO_SAFES=cold_wallet_alias
LPORTFOLIO_DB_PATH=                # optional override
```

Empty/unset → the corresponding section is omitted. Only `LPORTFOLIO_ADDRESSES`
is strictly required.

### `config.toml` (Android)

Lives at `/data/data/com.lportfolio.android/files/config.toml` on the device.
The Settings screen in the app writes it; you can also seed it manually:

```toml
safes = ["cold"]

[[addresses]]
alias = "hot"
address = "0x..."

[[addresses]]
alias = "cold"
address = "0x..."

[chains.mainnet]
rpc_url = "http://192.168.1.157:8545"

[chains.arbitrum]
rpc_url = "https://arb1.arbitrum.io/rpc/..."

[beacon]
url = "http://192.168.1.157:3500"
validator_indices = [12345, 12346]

[csm]
operator_ids = [42]

[tokens]
whitelist = ["USDC", "USDT", "ARB", "DAI"]
```

Push from desktop:

```bash
adb push my-config.toml /data/local/tmp/config.toml
adb shell run-as com.lportfolio.android cp /data/local/tmp/config.toml files/config.toml
adb shell am force-stop com.lportfolio.android
adb shell am start -n com.lportfolio.android/android.app.NativeActivity
```

The phone needs to reach whatever URLs you configure. LAN-only RPCs and
beacon nodes require a Tailscale/WireGuard tunnel or a route from the
phone's Wi-Fi to your home network. Public RPCs and CoinGecko work over
any internet connection.

## CLI usage

```
lportfolio chains                                   # configured chains + status
lportfolio sync     [--chain <id>] [--address <alias>]
lportfolio holdings [--refresh]                     # native + ERC-20 + staking + CSM + USD total
lportfolio history  [--address <alias>] [--chain <id>] [--since <date>]
lportfolio tag      <address> <label> [--chain <c>] [--kind <eoa|contract|protocol>]
lportfolio unknowns [--chain <id>]                  # interactive tagging in a TTY
lportfolio completions [bash]
```

Install shell completions:

```bash
lportfolio completions bash > ~/.local/share/bash-completion/completions/lportfolio
```

## ENS reverse-resolution

`lportfolio sync` runs a best-effort ENS reverse-resolution pass after the
Etherscan pulls finish, using the configured `LPORTFOLIO_RPC_MAINNET`
endpoint and the Universal Resolver contract. Results — both hits and
confirmed misses — land in the `ens_cache` table and are reused for every
subsequent `lportfolio history` and `lportfolio unknowns`, so each address
is queried at most once.

- Canonical ENS reverse lives on mainnet only, so the cache has no
  `chain_id` column. The same name renders for an address regardless of
  which chain its transactions appeared on.
- ENS names show in `history` only when nothing higher-priority is
  available: alias > manual `lportfolio tag` label > decoder-supplied
  registry label > **ENS name** > short-hex fallback.
- `unknowns` hides counterparties that successfully reverse-resolved; only
  manually-meaningful ones surface.
- No TTL — entries stay cached forever. To force a re-resolution, delete
  the row:

  ```bash
  sqlite3 ~/.local/share/lportfolio/db.sqlite \
      "DELETE FROM ens_cache WHERE address = '0xdeadbeef...';"
  lportfolio sync
  ```

## Android app

Only the `holdings` view is ported. The app has two screens:

- **Settings** — forms for every config field. Save writes `config.toml`
  to app-private storage; Reload re-reads from disk.
- **Holdings** — Refresh / Force refresh buttons; results render as
  scrollable egui grids with the same per-chain pivot as the CLI tables.

Logs are routed to `logcat`:

```bash
adb logcat -s lportfolio:V RustStdoutStderr:V
```

## Architecture

```
src/
  lib.rs              re-exports the portable core
  bin/
    lportfolio.rs     CLI: clap entry, subcommand wiring, panic hook
  config.rs           .env loader + TOML codec
  chain.rs            Chain enum
  rpc.rs              JSON-RPC reads (alloy)
  explorer.rs         Etherscan v2 client
  staking.rs          Beacon API client
  csm.rs              Lido CSM bond reader
  splits.rs           Splits V2 claimable-balance reader
  tokens.rs           Hardcoded ERC-20 whitelist registry
  holdings.rs         PortfolioSnapshot aggregator
  db.rs               rusqlite schema + queries
  sync.rs             Incremental transaction history sync
  portfolio_view.rs   Cell formatting + pivot helpers
                      shared by CLI render and Android UI
  render.rs           comfy-table renderer (cli feature only)
  interactive.rs      Unknown-counterparty tagging prompt
  decode/             Protocol decoders (Lido, Aave, Uniswap, ...)
  android/            egui frontend, cdylib-only
crates/
  rustls-platform-verifier-webpki/
                      Local replacement for the upstream crate; uses
                      Mozilla's webpki-roots on every platform so the
                      Android build does not need the Kotlin
                      CertificateVerifier shim
manifest.yaml         Android package id + permissions for xbuild
```

The portable core (everything outside `android/` and `bin/` excluding
`render.rs`/`interactive.rs`) compiles on both the desktop CLI and the
Android cdylib.

## License

MIT — see [LICENSE](LICENSE).
