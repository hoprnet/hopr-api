# hopr-api Development Guidelines

## Project Overview

hopr-api is a trait-only library that defines the public API surface for the HOPR protocol. It provides high-level API traits across six domains: chain interactions, cover traffic, database, network graphs, network state, and node control.

**Key Modules** (feature-gated):

- `chain` — On-chain operation APIs (accounts, channels, tickets, Safe, events, keys, values)
- `ct` — Cover traffic and probing API traits
- `db` — Node database operation traits
- `graph` — Network graph topology, QoS, routing costs
- `network` — Network state, peer observations, connectivity
- `node` — High-level HOPR node API

## Build & Test (Run These Commands)

### Essential Commands (MUST run before committing)

```bash
nix fmt                    # Format all code (Rust, Nix)
nix run -L .#check        # Run clippy + all linters
```

### Build

```bash
nix develop -c cargo build              # Standard build
nix develop -c cargo check              # Quick type-check
```

### Test

```bash
cargo test --lib                        # Unit tests only
cargo test --test '*' -- --test-threads=1   # Integration tests (MUST be single-threaded)
```

### Setup

1. Install Nix with flakes: `~/.config/nix/nix.conf` → `experimental-features = nix-command flakes`
2. Enable direnv: `direnv allow .` (auto-loads environment + pre-commit hooks)

## Technology Stack

- **Rust**: Async traits (runtime-agnostic), libp2p types
- **Dependencies**: `hopr-types` (external, git), `hopr-crypto-packet` (external, git)
- **Testing**: Rust native tests, insta (snapshot testing)
- **Formatting**: rustfmt (nightly), nixfmt, prettier

## Architecture

### Workspace Structure

```text
api/          - The hopr-api trait library
  ├─ chain/   - On-chain operation traits (accounts, channels, tickets, Safe, keys)
  ├─ ct/      - Cover traffic & probing traits
  ├─ db/      - Database operation traits
  ├─ graph/   - Network graph & routing cost traits
  ├─ network/ - Network state & peer observation traits
  └─ node/    - High-level node API traits & state machine
```

### Key Design Patterns

**Trait-Only Library**: This crate defines only traits and types — no concrete implementations. Implementations live in the main hoprnet repository.

**Feature Gating**: Each module is behind a Cargo feature flag. The `full` feature enables everything.

**Aggregation Traits**: Composite traits like `HoprChainApi` automatically implement for types that implement all constituent traits with matching error types.

**Error Unification**: API traits use associated `type Error` to allow implementors to define their own error types while maintaining a consistent trait interface.

## Code Style

### Critical Rules

- Use `tracing::info!()` not `info!()` (explicit prefix required)
- Error handling: `thiserror` for error types
- Async locks: `parking_lot::Mutex` (sync), `tokio::sync::Mutex` (async) — NEVER `std::sync::Mutex`
- Naming: `snake_case` functions/vars, `CamelCase` types/traits
- Documentation: `///` for public APIs with examples
- Follow Clean Code principles
- Flag long functions, deep nesting, and magic numbers
- Use clear, descriptive names
- Document public interfaces with rationale and functionality descriptions
- Always break code up into modules and components so that it can be easily reused

For language-specific rules see [rust.md](rust.md).

## Project-Specific Conventions

### HOPR Protocol Types

- **Address**: On-chain Ethereum address (`hopr_types::primitive::prelude::Address`)
- **OffchainPublicKey / OffchainKeypair**: Ed25519 keys for packet routing
- **PeerId**: libp2p identity derived from OffchainPublicKey
- **ChannelEntry / ChannelId**: Payment channel state
- **Balance / Currency**: Token amounts with currency type safety

### Trait Design

- Use `async_trait` for dynamic dispatch compatibility
- Use `auto_impl` for automatic implementation delegation (`&`, `Box`, `Arc`)
- Associated error types must be `Error + Send + Sync + 'static`
- Prefer `BoxFuture` and `BoxStream` for trait return types that need type erasure

## Common Mistakes (AVOID)

1. `.unwrap()` in libraries → propagate errors with `?`
2. Missing tracing prefix → `tracing::info!()` not `info!()`
3. Compiler warnings → fix or `#[allow(reason)]` with justification
4. Hardcoding config values → use appropriate abstractions
